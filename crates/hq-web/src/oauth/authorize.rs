//! `GET /oauth/authorize`: the owner's browser arrives here from Claude. HQ checks the request,
//! stores it, and sends the browser to the PWA consent page. It never issues a code itself.

use super::cimd;
use crate::WsState;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use hq_db::mcp_connector as store;
use std::collections::HashMap;
use std::sync::Arc;

/// The hosted Claude apps' callback, and the one Anthropic has announced it will move to.
const CLAUDE_CALLBACKS: &[&str] = &[
    "https://claude.ai/api/mcp/auth_callback",
    "https://claude.com/api/mcp/auth_callback",
];
/// PKCE verifiers are 43 to 128 characters, so an S256 challenge is exactly this long.
const S256_CHALLENGE_LEN: usize = 43;
const MAX_STATE_LEN: usize = 1024;
const MAX_SCOPE_LEN: usize = 256;

/// Whether HQ will send a code to `uri`: Claude's hosted callback, or a loopback `/callback`
/// on any port for Claude Code (RFC 8252).
pub(crate) fn redirect_allowed(uri: &str) -> bool {
    if CLAUDE_CALLBACKS.contains(&uri) {
        return true;
    }
    let Ok(url) = reqwest::Url::parse(uri) else {
        return false;
    };
    url.scheme() == "http"
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))
        && url.path() == "/callback"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

fn base64url(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The request's fields once checked against everything but the client's own document.
struct Checked<'a> {
    client_id: &'a str,
    redirect_uri: &'a str,
    challenge: &'a str,
    state: Option<&'a str>,
    scope: &'a str,
}

fn check<'a>(q: &'a HashMap<String, String>, resource: &str) -> Result<Checked<'a>, &'static str> {
    let get = |k: &str| q.get(k).map(String::as_str);
    if get("response_type") != Some("code") {
        return Err("only the authorization code flow is supported");
    }
    let client_id = get("client_id").ok_or("the request names no client")?;
    let redirect_uri = get("redirect_uri").ok_or("the request has no redirect URI")?;
    if !redirect_allowed(redirect_uri) {
        return Err("the redirect URI is not Claude's");
    }
    if get("code_challenge_method") != Some("S256") {
        return Err("PKCE with S256 is required");
    }
    let challenge = get("code_challenge").unwrap_or_default();
    if challenge.len() != S256_CHALLENGE_LEN || !base64url(challenge) {
        return Err("the PKCE challenge is malformed");
    }
    if get("resource").is_some_and(|r| r != resource) {
        return Err("the request is for a different server");
    }
    let state = get("state");
    let scope = get("scope").unwrap_or(super::SCOPE);
    if state.is_some_and(|s| s.len() > MAX_STATE_LEN) || scope.len() > MAX_SCOPE_LEN {
        return Err("the request is too large");
    }
    Ok(Checked {
        client_id,
        redirect_uri,
        challenge,
        state,
        scope,
    })
}

pub(crate) async fn authorize(
    State(state): State<Arc<WsState>>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(origins) = state.connector.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let now = super::now();
    let armed = state
        .db
        .with_conn(|c| store::armed_until(c, now))
        .ok()
        .flatten();
    if armed.is_none() {
        return page(
            StatusCode::FORBIDDEN,
            "HQ is not accepting new connections right now",
            "In HQ, open Settings, then Claude connector, and choose Allow a new connection. \
             Then press Connect in Claude again within ten minutes.",
        );
    }
    let resource = origins.resource();
    let checked = match check(&q, &resource) {
        Ok(c) => c,
        Err(why) => {
            return page(
                StatusCode::BAD_REQUEST,
                "This connection request is not valid",
                why,
            );
        }
    };
    let meta = match cimd::client_metadata(checked.client_id).await {
        Ok(m) => m,
        Err(why) => return page(StatusCode::BAD_REQUEST, "This client cannot connect", why),
    };
    if !meta.redirect_uris.iter().any(|u| u == checked.redirect_uri) {
        return page(
            StatusCode::BAD_REQUEST,
            "This connection request is not valid",
            "the client does not list this redirect URI",
        );
    }
    let new = store::NewRequest {
        client_id: checked.client_id,
        client_name: &meta.client_name,
        redirect_uri: checked.redirect_uri,
        state: checked.state,
        code_challenge: checked.challenge,
        resource: &resource,
        scope: checked.scope,
    };
    match state.db.with_conn(|c| store::create_request(c, &new, now)) {
        Ok(id) => (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/connect?request={id}"))],
        )
            .into_response(),
        Err(e) => {
            tracing::error!("oauth: storing an authorization request failed: {e}");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "try again",
            )
        }
    }
}

/// A plain page for the owner. Every string is a constant from this module, so nothing a
/// request carries is ever written into the HTML.
fn page(status: StatusCode, title: &'static str, detail: &'static str) -> Response {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
         content=\"width=device-width,initial-scale=1\"><title>Agent HQ</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem\">\
         <h1 style=\"font-size:1.25rem\">{title}</h1><p>{detail}</p></body></html>"
    );
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

#[cfg(test)]
mod tests;
