//! `POST /oauth/token` and `POST /oauth/revoke`. Claude's servers call these with
//! form-encoded bodies (RFC 6749, RFC 7009); Claude is a public client, so PKCE and the
//! one-time refresh tokens are what bind the tokens to it. A Gemini client is confidential: it
//! authenticates with the secret the owner created, and keeps one refresh token.

use crate::WsState;
use axum::Form;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hq_db::mcp_connector::{self as store, Refresh, TokenPair};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

const MIN_VERIFIER_LEN: usize = 43;
const MAX_VERIFIER_LEN: usize = 128;

type Params = HashMap<String, String>;

pub(crate) async fn token(
    State(state): State<Arc<WsState>>,
    headers: HeaderMap,
    Form(form): Form<Params>,
) -> Response {
    let now = super::now();
    let form = match authenticated_form(&state, &headers, form) {
        Ok(f) => f,
        Err(Refusal::Request) => return oauth_error("invalid_request"),
        Err(Refusal::Client) => return invalid_client(),
    };
    match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => exchange_code(&state, &form, now),
        Some("refresh_token") => refresh(&state, &form, now),
        Some(_) => oauth_error("unsupported_grant_type"),
        None => oauth_error("invalid_request"),
    }
}

/// Why a token request was refused before its grant was looked at.
enum Refusal {
    Request,
    Client,
}

/// The form with a confidential client's id in it once its secret checks out. A form from a
/// client id HQ did not create (Claude's metadata URL) passes through untouched.
fn authenticated_form(
    state: &WsState,
    headers: &HeaderMap,
    mut form: Params,
) -> Result<Params, Refusal> {
    let basic = basic_credentials(headers);
    let form_secret = form.get("client_secret").cloned();
    if basic.is_some() && form_secret.is_some() {
        return Err(Refusal::Request);
    }
    let form_id = form.get("client_id").cloned();
    let (id, secret) = match basic {
        Some((id, secret)) => {
            if form_id.as_ref().is_some_and(|f| *f != id) {
                return Err(Refusal::Request);
            }
            (Some(id), Some(secret))
        }
        None => (form_id, form_secret),
    };
    let Some(id) = id.filter(|id| id.starts_with(store::CLIENT_PREFIX)) else {
        return Ok(form);
    };
    let gemini_on = state.connector.as_ref().is_some_and(|c| c.gemini);
    let secret = secret.unwrap_or_default();
    let valid = gemini_on
        && !secret.is_empty()
        && state
            .db
            .with_conn(|c| store::authenticate_client(c, &id, &secret))
            .unwrap_or(false);
    if !valid {
        tracing::warn!("oauth: a registered client failed to authenticate");
        return Err(Refusal::Client);
    }
    form.insert("client_id".into(), id);
    Ok(form)
}

/// `Authorization: Basic` credentials, form-decoded as RFC 6749 section 2.3.1 requires.
fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = value.strip_prefix("Basic ")?.trim();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let pair = String::from_utf8(decoded).ok()?;
    let (id, secret) = pair.split_once(':')?;
    // A form-encoded value never holds a raw `&` or `#`, so either one is a malformed header.
    let unescape = |s: &str| {
        if s.contains(['&', '#']) {
            return None;
        }
        let url = reqwest::Url::parse(&format!("http://form.invalid/?v={s}")).ok()?;
        url.query_pairs().next().map(|(_, v)| v.into_owned())
    };
    Some((unescape(id)?, unescape(secret)?))
}

fn invalid_client() -> Response {
    let mut res = no_store(
        StatusCode::UNAUTHORIZED,
        json!({ "error": "invalid_client" }),
    );
    res.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(r#"Basic realm="agent-hq""#),
    );
    res
}

/// The S256 challenge of a verifier, or `None` when the verifier breaks RFC 7636's rules.
pub(crate) fn s256(verifier: &str) -> Option<String> {
    let ok_len = (MIN_VERIFIER_LEN..=MAX_VERIFIER_LEN).contains(&verifier.len());
    let ok_chars = verifier
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'));
    if !(ok_len && ok_chars) {
        return None;
    }
    let digest = Sha256::digest(verifier.as_bytes());
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest))
}

fn exchange_code(state: &WsState, form: &Params, now: i64) -> Response {
    let get = |k: &str| form.get(k).map(String::as_str);
    let (Some(code), Some(client_id), Some(redirect_uri), Some(verifier)) = (
        get("code"),
        get("client_id"),
        get("redirect_uri"),
        get("code_verifier"),
    ) else {
        return oauth_error("invalid_request");
    };
    let Some(challenge) = s256(verifier) else {
        return oauth_error("invalid_grant");
    };
    let exchange = store::CodeExchange {
        code,
        client_id,
        redirect_uri,
        challenge: &challenge,
        resource: get("resource"),
    };
    match state
        .db
        .with_conn(|c| store::redeem_code(c, &exchange, now))
    {
        Ok(Some(pair)) => {
            tracing::info!(grant = %pair.grant_id, "oauth: a client connected");
            announce_connection(state, &pair.grant_id, now);
            tokens(&pair)
        }
        Ok(None) => oauth_error("invalid_grant"),
        Err(e) => server_error(e),
    }
}

fn refresh(state: &WsState, form: &Params, now: i64) -> Response {
    let get = |k: &str| form.get(k).map(String::as_str);
    let (Some(token), Some(client_id)) = (get("refresh_token"), get("client_id")) else {
        return oauth_error("invalid_request");
    };
    let outcome = state
        .db
        .with_conn(|c| store::refresh(c, token, client_id, get("resource"), now));
    match outcome {
        Ok(Refresh::Issued(pair)) => tokens(&pair),
        Ok(Refresh::Refused) => oauth_error("invalid_grant"),
        Ok(Refresh::Reused { grant_id }) => {
            tracing::warn!(grant = %grant_id, "oauth: a spent refresh token came back; grant revoked");
            state.notify_owner(
                super::guard::SOURCE,
                "A Claude connection was disconnected",
                &format!(
                    "Grant {grant_id} presented a refresh token that had already been used, which \
                     means a copy of it exists somewhere else. HQ revoked the grant. Reconnect \
                     from Claude if it was yours."
                ),
            );
            oauth_error("invalid_grant")
        }
        Err(e) => server_error(e),
    }
}

/// Tells the owner a new connection is live, with its name and what it may reach.
fn announce_connection(state: &WsState, grant_id: &str, now: i64) {
    let grant = state
        .db
        .with_conn(|c| store::list_grants(c, false, now))
        .ok()
        .and_then(|gs| gs.into_iter().find(|g| g.id == grant_id));
    let Some(g) = grant else {
        return;
    };
    let title = if g.profile == store::Profile::Gemini.as_str() {
        "Gemini is connected to HQ"
    } else {
        "Claude is connected to HQ"
    };
    state.notify_owner(
        super::guard::SOURCE,
        title,
        &format!(
            "\"{}\" ({}) can now reach HQ with {} access. Disconnect it any time in Settings or \
             with `hq connector revoke {}`.",
            g.label, g.client_name, g.grant_scope, g.id
        ),
    );
}

/// RFC 7009: the answer is the same whether or not the token was known.
pub(crate) async fn revoke(
    State(state): State<Arc<WsState>>,
    Form(form): Form<Params>,
) -> Response {
    let Some(token) = form.get("token") else {
        return oauth_error("invalid_request");
    };
    if let Err(e) = state
        .db
        .with_conn(|c| store::revoke_token(c, token, super::now()))
    {
        return server_error(e);
    }
    no_store(StatusCode::OK, json!({}))
}

fn tokens(pair: &TokenPair) -> Response {
    no_store(
        StatusCode::OK,
        json!({
            "access_token": pair.access_token,
            "token_type": "Bearer",
            "expires_in": pair.expires_in,
            "refresh_token": pair.refresh_token,
            "scope": pair.scope,
        }),
    )
}

fn oauth_error(code: &'static str) -> Response {
    no_store(StatusCode::BAD_REQUEST, json!({ "error": code }))
}

fn server_error(e: anyhow::Error) -> Response {
    tracing::error!("oauth: token endpoint failed: {e}");
    no_store(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": "server_error" }),
    )
}

fn no_store(status: StatusCode, body: serde_json::Value) -> Response {
    let mut res = (status, axum::Json(body)).into_response();
    let headers = res.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s256_matches_rfc_7636_and_refuses_bad_verifiers() {
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").as_deref(),
            Some("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        );
        assert!(s256("short").is_none());
        assert!(s256(&"a".repeat(129)).is_none());
        assert!(s256(&format!("{}!", "a".repeat(50))).is_none());
    }
}
