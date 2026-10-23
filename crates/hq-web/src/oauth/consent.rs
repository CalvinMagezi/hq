//! `/api/oauth/*`: what the PWA's consent page and Settings section call. Every route sits
//! behind the web token and the origin guard, and changes need `X-HQ-Client`.

use crate::WsState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hq_db::mcp_connector::{self as store, GrantScope};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const DEFAULT_ARM_MINUTES: i64 = 10;
const MAX_ARM_MINUTES: i64 = 60;
const MAX_LABEL_CHARS: usize = 60;
/// Calls Settings lists, newest first.
const RECENT_CALLS: i64 = 20;
/// The cookie tying a pending request to the browser that started it.
const BINDER_COOKIE: &str = "hq_oauth_request";
/// Matches the request's lifetime in the store.
const BINDER_MAX_AGE_SECS: i64 = store::REQUEST_TTL_SECS;
const GONE: &str = "this request has expired, was already answered, or was started in another \
                    browser; press Connect in Claude again from this one";

/// `Set-Cookie` for a request's binder: unreadable by scripts, sent only to the request routes.
/// `Secure` is left off only for a plain-http loopback consent origin, where WebKit drops a
/// `Secure` cookie and the connection never leaves the device.
pub(crate) fn binder_cookie(binder: &str, secure: bool) -> String {
    let secure = if secure { " Secure;" } else { "" };
    format!(
        "{BINDER_COOKIE}={binder}; Path=/api/oauth/requests; HttpOnly;{secure} \
         SameSite=Strict; Max-Age={BINDER_MAX_AGE_SECS}"
    )
}

/// The binder this browser holds, if any.
fn binder(headers: &HeaderMap) -> String {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == BINDER_COOKIE)
        .map(|(_, value)| value.to_string())
        .unwrap_or_default()
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn db_error(e: anyhow::Error) -> Response {
    tracing::error!("oauth: consent API failed: {e}");
    refuse(StatusCode::INTERNAL_SERVER_ERROR, "database error")
}

fn connector_off() -> Response {
    refuse(StatusCode::NOT_FOUND, "the claude.ai connector is off")
}

/// The connector's state for Settings: origins, the arming window and live grants.
pub(crate) async fn status(State(state): State<Arc<WsState>>) -> Response {
    let Some(origins) = state.connector.as_ref() else {
        return Json(json!({ "enabled": false })).into_response();
    };
    let now = super::now();
    let read = state.db.with_conn(|c| {
        Ok((
            store::armed_until(c, now)?,
            store::list_grants(c, false, now)?,
            store::recent_calls(c, None, RECENT_CALLS)?,
        ))
    });
    match read {
        Ok((armed_until, grants, calls)) => Json(json!({
            "enabled": true,
            "url": origins.resource(),
            "consent_origin": origins.consent,
            "default_scope": state.connector_default_scope().as_str(),
            "armed_until": armed_until,
            "grants": grants,
            "recent_calls": calls,
        }))
        .into_response(),
        Err(e) => db_error(e),
    }
}

#[derive(Deserialize, Default)]
pub(crate) struct ArmBody {
    minutes: Option<i64>,
}

pub(crate) async fn arm(
    State(state): State<Arc<WsState>>,
    body: Option<Json<ArmBody>>,
) -> Response {
    if state.connector.is_none() {
        return connector_off();
    }
    let minutes = body
        .and_then(|Json(b)| b.minutes)
        .unwrap_or(DEFAULT_ARM_MINUTES)
        .clamp(1, MAX_ARM_MINUTES);
    let until = super::now() + minutes * 60;
    match state.db.with_conn(|c| store::arm(c, until)) {
        Ok(()) => Json(json!({ "armed_until": until })).into_response(),
        Err(e) => db_error(e),
    }
}

/// Closes the window and drops requests waiting in it.
pub(crate) async fn disarm(State(state): State<Arc<WsState>>) -> Response {
    match state.db.with_conn(store::close_window) {
        Ok(()) => Json(json!({ "armed_until": null })).into_response(),
        Err(e) => db_error(e),
    }
}

/// The kill switch: every grant revoked and the window closed.
pub(crate) async fn off(State(state): State<Arc<WsState>>) -> Response {
    let revoked = state
        .db
        .with_conn(|c| store::revoke_all(c, "owner turned the connector off", super::now()));
    match revoked {
        Ok(n) => Json(json!({ "revoked": n })).into_response(),
        Err(e) => db_error(e),
    }
}

pub(crate) async fn revoke_grant(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
) -> Response {
    let revoked = state
        .db
        .with_conn(|c| store::revoke_grant(c, &id, "revoked by the owner", super::now()));
    match revoked {
        Ok(true) => Json(json!({ "revoked": true })).into_response(),
        Ok(false) => refuse(StatusCode::NOT_FOUND, "no live grant with that id"),
        Err(e) => db_error(e),
    }
}

/// A pending request as the consent page shows it.
pub(crate) async fn request(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if state.connector.is_none() {
        return connector_off();
    }
    let now = super::now();
    let binder = binder(&headers);
    let found = state.db.with_conn(|c| {
        Ok((
            store::get_request(c, &id, &binder, now)?,
            store::armed_until(c, now)?,
        ))
    });
    match found {
        Ok((Some(req), armed_until)) => Json(json!({
            "client_name": req.client_name,
            "client_id": req.client_id,
            "redirect_host": host_of(&req.redirect_uri),
            "resource": req.resource,
            "created_at": req.created_at,
            "expires_at": req.expires_at,
            "armed_until": armed_until.filter(|a| *a == req.armed_until),
            "default_scope": state.connector_default_scope().as_str(),
        }))
        .into_response(),
        Ok((None, _)) => refuse(StatusCode::NOT_FOUND, GONE),
        Err(e) => db_error(e),
    }
}

/// The redirect's host, with its port when it has one, so a loopback callback shows which
/// local port will receive the code.
fn host_of(uri: &str) -> String {
    let Ok(url) = reqwest::Url::parse(uri) else {
        return String::new();
    };
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

#[derive(Deserialize)]
pub(crate) struct ApproveBody {
    scope: String,
    label: Option<String>,
}

/// The owner's approval. One approval per arming: the window closes once it is used.
pub(crate) async fn approve(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<ApproveBody>,
) -> Response {
    if state.connector.is_none() {
        return connector_off();
    }
    let Some(scope) = GrantScope::parse(&body.scope) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "scope must be full, read_only or tasks",
        );
    };
    let label = clean_label(body.label.as_deref());
    let now = super::now();
    let binder = binder(&headers);
    let decided = state
        .db
        .with_conn(|c| store::approve_request(c, &id, &binder, scope, &label, now));
    match decided {
        Ok(store::Approval::Issued { request, code }) => {
            let redirect = callback(&request, &[("code", &code)]);
            Json(json!({ "redirect": redirect })).into_response()
        }
        Ok(store::Approval::NotArmed) => refuse(
            StatusCode::CONFLICT,
            "HQ is no longer accepting connections; allow one again in Settings",
        ),
        Ok(store::Approval::Gone) => refuse(StatusCode::CONFLICT, GONE),
        Err(e) => db_error(e),
    }
}

pub(crate) async fn deny(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let binder = binder(&headers);
    let taken = state
        .db
        .with_conn(|c| store::deny_request(c, &id, &binder, super::now()));
    match taken {
        Ok(Some(req)) => {
            let redirect = callback(&req, &[("error", "access_denied")]);
            Json(json!({ "redirect": redirect })).into_response()
        }
        Ok(None) => refuse(StatusCode::NOT_FOUND, GONE),
        Err(e) => db_error(e),
    }
}

fn clean_label(label: Option<&str>) -> String {
    let label: String = label
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_LABEL_CHARS)
        .collect();
    let label = label.trim();
    if label.is_empty() {
        "Claude".to_string()
    } else {
        label.to_string()
    }
}

/// The client's redirect URI with `params` and the client's own `state`.
fn callback(req: &store::AuthRequest, params: &[(&str, &str)]) -> String {
    let Ok(mut url) = reqwest::Url::parse(&req.redirect_uri) else {
        return req.redirect_uri.clone();
    };
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
        if let Some(s) = &req.state {
            q.append_pair("state", s);
        }
    }
    url.into()
}
