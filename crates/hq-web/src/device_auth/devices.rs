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
    match hq_db::web_auth::delete(&state.db, &id) {
        Ok(false) => ApiError::not_found().into_response(),
        Err(e) => ApiError::internal(e).into_response(),
        Ok(true) => {
            state.end_web_sessions();
            tracing::info!(device = %id, "web: a device was signed out");
            StatusCode::NO_CONTENT.into_response()
        }
    }
}

/// `POST /api/auth/rotate`: signs out every device and the configured token in one
/// transaction, voids the pending sign-in code and open sockets, and returns the one new token
/// this device keeps.
pub(crate) async fn rotate_handler(State(state): State<Arc<WsState>>) -> Response {
    if let Some(refused) = sign_in_off(&state) {
        return refused;
    }
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
    state.device_codes.clear();
    state.end_web_sessions();
    tracing::info!(device = %minted.id, "web: signed out everywhere");
    state.notify_owner(
        DEVICE_SIGN_IN_SOURCE,
        "HQ signed out every device",
        &format!(
            "Every device and the original login link were signed out of HQ at {} UTC. Only \
             the device that asked stays signed in. If it was not you, send /login to your HQ \
             bot or ask whoever runs your server for a new link, then sign out everywhere again.",
            at.format("%Y-%m-%d %H:%M")
        ),
    );
    no_store(StatusCode::OK, json!({ "token": minted.token }))
}
