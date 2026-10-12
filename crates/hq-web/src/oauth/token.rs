//! `POST /oauth/token` and `POST /oauth/revoke`. Claude's servers call these with
//! form-encoded bodies (RFC 6749, RFC 7009); Claude is a public client, so PKCE and the
//! one-time refresh tokens are what bind the tokens to it.

use crate::WsState;
use axum::Form;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
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

pub(crate) async fn token(State(state): State<Arc<WsState>>, Form(form): Form<Params>) -> Response {
    let now = super::now();
    match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => exchange_code(&state, &form, now),
        Some("refresh_token") => refresh(&state, &form, now),
        Some(_) => oauth_error("unsupported_grant_type"),
        None => oauth_error("invalid_request"),
    }
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
            oauth_error("invalid_grant")
        }
        Err(e) => server_error(e),
    }
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
