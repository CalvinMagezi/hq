//! `GET /oauth/authorize`: the owner's browser arrives here from Claude. HQ checks the request,
//! stores it, and sends the browser to the PWA consent page. It never issues a code itself.

use super::cimd;
use crate::WsState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
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
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(settings) = state.connector.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // With a private consent origin, the public name never answers here: no arming oracle and
    // no way to fill the waiting list from the internet, whatever the proxy in front forwards.
    if !on_origin(&headers, &settings.consent) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let now = super::now();
    let armed = state.db.with_conn(|c| store::armed_until(c, now));
    if !matches!(armed, Ok(Some(_))) {
        return not_armed();
    }
    let resource = settings.resource();
    let checked = match check(&q, &resource) {
        Ok(c) => c,
        Err(why) => return page(StatusCode::BAD_REQUEST, INVALID, why),
    };
    let meta = match cimd::client_metadata(checked.client_id).await {
        Ok(m) => m,
        Err(why) => return page(StatusCode::BAD_REQUEST, "This client cannot connect", why),
    };
    if !meta
        .redirect_uris
        .iter()
        .any(|listed| redirect_matches(listed, checked.redirect_uri))
    {
        return page(
            StatusCode::BAD_REQUEST,
            INVALID,
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
    store_request(&state, &new, now, settings.consent.starts_with("https://"))
}

const INVALID: &str = "This connection request is not valid";
const HTTPS_DEFAULT_PORT: &str = ":443";
const HTTP_DEFAULT_PORT: &str = ":80";

/// Whether the request's `Host` is the given origin's authority (https, or http on loopback).
pub(crate) fn on_origin(headers: &HeaderMap, origin: &str) -> bool {
    let (authority, default_port) = if let Some(a) = origin.strip_prefix("https://") {
        (a, HTTPS_DEFAULT_PORT)
    } else if let Some(a) = origin.strip_prefix("http://") {
        (a, HTTP_DEFAULT_PORT)
    } else {
        return false;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h.trim().to_ascii_lowercase())
        .unwrap_or_default();
    host.strip_suffix(default_port).unwrap_or(&host) == authority
}

fn not_armed() -> Response {
    page(
        StatusCode::FORBIDDEN,
        "HQ is not accepting new connections right now",
        "In HQ, open Settings, then Claude connector, and choose Allow a new connection. \
         Then press Connect in Claude again within ten minutes.",
    )
}

/// Stores the request bound to a fresh cookie in this browser and sends it to the consent page.
fn store_request(
    state: &WsState,
    new: &store::NewRequest,
    now: i64,
    secure_cookie: bool,
) -> Response {
    let binder = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    match state
        .db
        .with_conn(|c| store::create_request(c, new, &binder, now))
    {
        Ok(store::Pending::Created(id)) => (
            StatusCode::FOUND,
            [
                (header::LOCATION, format!("/connect?request={id}")),
                (
                    header::SET_COOKIE,
                    super::consent::binder_cookie(&binder, secure_cookie),
                ),
            ],
        )
            .into_response(),
        Ok(store::Pending::NotArmed) => not_armed(),
        Ok(store::Pending::Full) => page(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many connection requests are waiting",
            "Close the window in HQ's Settings, allow a new connection again, then press Connect in Claude.",
        ),
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

/// Whether a redirect URI matches one the client's document lists. Loopback URIs match on
/// scheme, host and path with any port (RFC 8252 section 7.3): Claude Code lists them with no
/// port and picks one per session.
fn redirect_matches(listed: &str, requested: &str) -> bool {
    if listed == requested {
        return true;
    }
    let (Ok(a), Ok(b)) = (reqwest::Url::parse(listed), reqwest::Url::parse(requested)) else {
        return false;
    };
    let loopback = matches!(a.host_str(), Some("localhost" | "127.0.0.1"));
    loopback
        && a.scheme() == "http"
        && a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.path() == b.path()
        && a.query().is_none()
        && b.query().is_none()
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
