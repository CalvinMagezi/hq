//! `/api/oauth/*`: what the PWA's consent page and Settings section call. Every route sits
//! behind the web token and the origin guard, and changes need `X-HQ-Client`.

use crate::WsState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use hq_db::mcp_connector::{self as store, GrantScope};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const DEFAULT_ARM_MINUTES: i64 = 10;
const MAX_ARM_MINUTES: i64 = 60;
const MAX_LABEL_CHARS: usize = 60;

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
    let read = state
        .db
        .with_conn(|c| Ok((store::armed_until(c, now)?, store::list_grants(c, false)?)));
    match read {
        Ok((armed_until, grants)) => Json(json!({
            "enabled": true,
            "url": origins.resource(),
            "consent_origin": origins.consent,
            "default_scope": state.connector_default_scope().as_str(),
            "armed_until": armed_until,
            "grants": grants,
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

pub(crate) async fn disarm(State(state): State<Arc<WsState>>) -> Response {
    match state.db.with_conn(store::disarm) {
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
pub(crate) async fn request(State(state): State<Arc<WsState>>, Path(id): Path<String>) -> Response {
    if state.connector.is_none() {
        return connector_off();
    }
    let now = super::now();
    let found = state.db.with_conn(|c| {
        Ok((
            store::get_request(c, &id, now)?,
            store::armed_until(c, now)?,
        ))
    });
    match found {
        Ok((Some(req), armed_until)) => Json(json!({
            "client_name": req.client_name,
            "client_id": req.client_id,
            "redirect_host": host_of(&req.redirect_uri),
            "resource": req.resource,
            "expires_at": req.expires_at,
            "armed_until": armed_until,
            "default_scope": state.connector_default_scope().as_str(),
        }))
        .into_response(),
        Ok((None, _)) => refuse(
            StatusCode::NOT_FOUND,
            "this request has expired or was already answered",
        ),
        Err(e) => db_error(e),
    }
}

fn host_of(uri: &str) -> String {
    reqwest::Url::parse(uri)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
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
    let decided = state.db.with_conn(|c| {
        if store::armed_until(c, now)?.is_none() {
            return Ok(Err(
                "HQ is no longer accepting connections; allow one again in Settings",
            ));
        }
        let Some(req) = store::take_request(c, &id, now)? else {
            return Ok(Err("this request has expired or was already answered"));
        };
        let code = store::issue_code(c, &req, scope, &label, now)?;
        store::disarm(c)?;
        Ok(Ok(callback(&req, &[("code", &code)])))
    });
    match decided {
        Ok(Ok(redirect)) => Json(json!({ "redirect": redirect })).into_response(),
        Ok(Err(why)) => refuse(StatusCode::CONFLICT, why),
        Err(e) => db_error(e),
    }
}

pub(crate) async fn deny(State(state): State<Arc<WsState>>, Path(id): Path<String>) -> Response {
    let taken = state
        .db
        .with_conn(|c| store::take_request(c, &id, super::now()));
    match taken {
        Ok(Some(req)) => {
            let redirect = callback(&req, &[("error", "access_denied")]);
            Json(json!({ "redirect": redirect })).into_response()
        }
        Ok(None) => refuse(
            StatusCode::NOT_FOUND,
            "this request has expired or was already answered",
        ),
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
