//! Settings, Devices: the signed-in devices (`GET /api/auth/devices`), signing one out
//! (`DELETE /api/auth/devices/{id}`) and signing out everywhere (`POST /api/auth/rotate`).
//! All three sit behind the web sign-in, and answer 404 when the server asks for none.

use super::{DEVICE_SIGN_IN_SOURCE, no_store};
use crate::WsState;
use crate::auth::WebCredential;
use crate::error::ApiError;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;

pub(crate) const DEVICES_PATH: &str = "/api/auth/devices";
pub(crate) const DEVICE_PATH: &str = "/api/auth/devices/{id}";
pub(crate) const ROTATE_PATH: &str = "/api/auth/rotate";
pub(crate) const CURRENT_DEVICE_PATH: &str = "/api/auth/devices/current";
pub(crate) const KEEP_PATH: &str = "/api/auth/devices/{id}/keep";

/// Devices whose new sign-in item still waits for the owner.
fn new_devices(db: &hq_db::Database) -> std::collections::HashSet<String> {
    let found = db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT dedup_key FROM value_items WHERE dedup_key LIKE ?1 AND source_task = ?2 \
             AND state IN ('pending', 'delivered', 'routed')",
        )?;
        let keys = stmt
            .query_map(
                [
                    format!("{}%", super::SIGN_IN_ITEM_PREFIX),
                    DEVICE_SIGN_IN_SOURCE.to_string(),
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(keys)
    });
    found
        .unwrap_or_default()
        .into_iter()
        .filter_map(|k| {
            k.strip_prefix(super::SIGN_IN_ITEM_PREFIX)
                .map(str::to_string)
        })
        .collect()
}
/// How the token that "Sign out everywhere" hands back shows in Settings, Devices.
pub(crate) const ROTATE_LABEL: &str = "Sign out everywhere";

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The 404 these routes answer on a server that asks for no sign-in.
fn sign_in_off(state: &WsState) -> Option<Response> {
    (!state.web_auth_enabled()).then(|| ApiError::not_found().into_response())
}

fn current_device(credential: Option<&Extension<WebCredential>>) -> Option<&str> {
    match credential.map(|Extension(c)| c) {
        Some(WebCredential::Device(id)) => Some(id),
        _ => None,
    }
}

/// `GET /api/auth/devices`: every signed-in device, which one is this one, and whether the
/// operator's configured token still works. Never a token or a hash.
pub(crate) async fn list_handler(
    State(state): State<Arc<WsState>>,
    credential: Option<Extension<WebCredential>>,
) -> Response {
    if let Some(refused) = sign_in_off(&state) {
        return refused;
    }
    let devices = match hq_db::web_auth::list(&state.db, now()) {
        Ok(devices) => devices,
        Err(e) => return ApiError::internal(e).into_response(),
    };
    let current = current_device(credential.as_ref());
    let fresh = new_devices(&state.db);
    let rows: Vec<_> = devices
        .into_iter()
        .map(|d| {
            let is_current = current == Some(d.id.as_str());
            json!({
                "id": d.id,
                "label": d.label,
                "created_at": d.created_at,
                "last_used_at": d.last_used_at,
                "expires_at": d.expires_at,
                "browser": d.browser,
                "system": d.system,
                "shared": d.shared,
                "keep": d.keep,
                "idle_expires_after": d.idle_expires_after,
                "new": fresh.contains(&d.id),
                "current": is_current,
            })
        })
        .collect();
    let operator_link_active = state
        .configured_web_token()
        .is_some_and(|t| matches!(hq_db::web_auth::is_revoked(&state.db, t), Ok(false)));
    no_store(
        StatusCode::OK,
        json!({
            "devices": rows,
            "operator_link_active": operator_link_active,
            "this_device_uses_operator_link": matches!(
                credential.as_ref().map(|Extension(c)| c),
                Some(WebCredential::Configured)
            ),
        }),
    )
}

/// `DELETE /api/auth/devices/{id}`: signs one device out, and closes open chat sockets so its
/// socket goes too; the others reconnect.
pub(crate) async fn delete_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(refused) = sign_in_off(&state) {
        return refused;
    }
    sign_out_device(&state, &id)
}

/// `DELETE /api/auth/devices/current`: Sign out this browser. Only a device token can; the
/// operator link signs out with Sign out everywhere.
pub(crate) async fn delete_current_handler(
    State(state): State<Arc<WsState>>,
    credential: Option<Extension<WebCredential>>,
) -> Response {
    if let Some(refused) = sign_in_off(&state) {
        return refused;
    }
    match current_device(credential.as_ref()) {
        Some(id) => sign_out_device(&state, id),
        None => ApiError::bad_request(
            "this browser uses the first sign-in link; sign out everywhere to turn it off",
        )
        .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct KeepBody {
    keep: bool,
}

/// `POST /api/auth/devices/{id}/keep`: Keep signed in, so 90 days without use never sign it out.
pub(crate) async fn keep_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<KeepBody>,
) -> Response {
    if let Some(refused) = sign_in_off(&state) {
        return refused;
    }
    match hq_db::web_auth::set_keep(&state.db, &id, body.keep, now()) {
        Ok(true) => no_store(StatusCode::OK, json!({ "id": id, "keep": body.keep })),
        Ok(false) => ApiError::not_found().into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

fn sign_out_device(state: &WsState, id: &str) -> Response {
    match hq_db::web_auth::delete(&state.db, id) {
        Ok(false) => ApiError::not_found().into_response(),
        Err(e) => ApiError::internal(e).into_response(),
        Ok(true) => {
            super::settle_sign_in_item(&state.db, id);
            state.end_web_sessions();
            if let Err(e) = hq_db::push_subscriptions::delete_for_device(&state.db, id) {
                tracing::warn!(device = %id, "web: a signed-out device keeps its push subscription: {e}");
            }
            tracing::info!(device = %id, "web: a device was signed out");
            StatusCode::NO_CONTENT.into_response()
        }
    }
}

/// What else a stolen sign-in could have set up: claude.ai connector grants (and an open
/// approval window), and Web Push subscriptions, dropped before the notice so no stranger's
/// browser is pushed it. Each is best effort: the sign-out itself has already committed.
fn end_other_access(state: &WsState, now: i64) {
    let revoked = state
        .db
        .with_conn(|c| hq_db::mcp_connector::revoke_all(c, "owner signed out everywhere", now));
    match revoked {
        Ok(n) => tracing::info!(
            grants = n,
            "web: sign-out everywhere revoked connector grants"
        ),
        Err(e) => {
            tracing::error!("web: sign-out everywhere could not revoke connector grants: {e}")
        }
    }
    if let Err(e) = hq_db::push_subscriptions::delete_all(&state.db) {
        tracing::error!("web: sign-out everywhere could not drop push subscriptions: {e}");
    }
}

/// Every device that signed in is gone, so no new sign-in waits for an answer.
fn settle_every_sign_in_item(db: &hq_db::Database) {
    for id in new_devices(db) {
        super::settle_sign_in_item(db, &id);
    }
}

/// `POST /api/auth/rotate`: signs out every device and the configured token in one
/// transaction, voids the pending sign-in code and open sockets, revokes connector grants and
/// push subscriptions, and returns the one new token this device keeps.
pub(crate) async fn rotate_handler(State(state): State<Arc<WsState>>) -> Response {
    if let Some(refused) = sign_in_off(&state) {
        return refused;
    }
    let _serialized = state.auth_lock.lock().await;
    state.device_codes.clear();
    let at = chrono::Utc::now();
    let minted = hq_db::web_auth::rotate(
        &state.db,
        state.configured_web_token(),
        ROTATE_LABEL,
        at.timestamp(),
    );
    let minted = match minted {
        Ok(minted) => minted,
        Err(e) => {
            tracing::error!("web: signing out everywhere failed, nothing changed: {e}");
            return ApiError::internal("could not sign out everywhere, nothing changed")
                .into_response();
        }
    };
    if state.configured_web_token().is_some() {
        state.mark_configured_signed_out();
    }
    state.device_codes.clear();
    state.end_web_sessions();
    end_other_access(&state, at.timestamp());
    settle_every_sign_in_item(&state.db);
    tracing::info!(device = %minted.id, "web: signed out everywhere");
    let paired = paired_machines(&state.db);
    state.notify_owner(
        DEVICE_SIGN_IN_SOURCE,
        "HQ signed out every device",
        &format!(
            "Every device and the original login link were signed out of HQ at {} UTC. Only \
             the device that asked stays signed in. If it was not you, send /login to your HQ \
             bot or ask whoever runs your server for a new link, then sign out everywhere again.\
             {}",
            at.format("%Y-%m-%d %H:%M"),
            untouched_note(&paired)
        ),
    );
    no_store(
        StatusCode::OK,
        json!({ "token": minted.token, "paired_machines_kept": paired }),
    )
}

/// The machines paired over the bridge, by name. Signing out everywhere does not touch them:
/// each keeps its pairing and the MCP key it got at setup until it is revoked on its own.
fn paired_machines(db: &hq_db::Database) -> Vec<String> {
    match hq_db::bridge::list(db, false) {
        Ok(peers) => peers.into_iter().map(|p| p.label).collect(),
        Err(e) => {
            tracing::warn!("web: could not list paired machines: {e:#}");
            Vec::new()
        }
    }
}

/// What signing out everywhere left as it was, said plainly in the owner's notice.
fn untouched_note(paired: &[String]) -> String {
    if paired.is_empty() {
        return String::new();
    }
    format!(
        " Paired machines were not signed out and keep their access to HQ, MCP key included: {}. \
         To cut one off, run `hq bridge peers` on this server, then `hq bridge revoke <id>`.",
        paired.join(", ")
    )
}
