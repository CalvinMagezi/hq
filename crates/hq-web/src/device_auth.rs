//! Signing in another device without pasting the web token. A signed-in device asks for a
//! one-time code (`POST /api/auth/device-code`), and the new device trades it for a device token
//! of its own (`POST /api/auth/exchange`, the one `/api` route the web token does not guard).
//! `devices` lists, signs out and rotates those tokens.

use crate::WsState;
use crate::error::ApiError;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hq_core::pairing::constant_time_eq;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

pub(crate) const DEVICE_CODE_PATH: &str = "/api/auth/device-code";
pub(crate) const EXCHANGE_PATH: &str = "/api/auth/exchange";
/// Inbox source of the "a device signed in" notice; Web Push always sends it.
pub(crate) const DEVICE_SIGN_IN_SOURCE: &str = "device-sign-in";
/// How a device token minted by the exchange shows in Settings, Devices.
pub(crate) const CODE_SIGN_IN_LABEL: &str = "Signed in with a code";

const CODE_TTL_SECS: i64 = 10 * 60;
const FAILURE_WINDOW_SECS: i64 = 10 * 60;
const MAX_FAILURES_PER_CLIENT: usize = 5;
/// Past this many wrong codes from anyone, the pending code is burned and every exchange waits.
const MAX_FAILURES_GLOBAL: usize = 20;
/// Bounds the per-client map, since forwarding headers are client supplied and can rotate.
const MAX_TRACKED_CLIENTS: usize = 1024;
const CODE_GROUP_LEN: usize = 5;
const UNKNOWN_CLIENT: &str = "unknown";

struct Pending {
    /// Random per code, so equal codes never share a stored hash.
    salt: String,
    hash: [u8; 32],
    expires_at: i64,
}

#[derive(Default)]
struct Ledger {
    pending: Option<Pending>,
    failures: Vec<i64>,
    failures_by_client: HashMap<String, Vec<i64>>,
}

impl Ledger {
    fn prune(&mut self, now: i64) {
        let since = now - FAILURE_WINDOW_SECS;
        self.failures.retain(|t| *t > since);
        self.failures_by_client.retain(|_, times| {
            times.retain(|t| *t > since);
            !times.is_empty()
        });
    }

    fn limited(&self, client: &str) -> bool {
        self.failures.len() >= MAX_FAILURES_GLOBAL
            || self
                .failures_by_client
                .get(client)
                .is_some_and(|t| t.len() >= MAX_FAILURES_PER_CLIENT)
    }

    fn record_failure(&mut self, client: &str, now: i64) {
        self.failures.push(now);
        if self.failures.len() >= MAX_FAILURES_GLOBAL {
            self.pending = None;
        }
        let tracked = self.failures_by_client.contains_key(client);
        if tracked || self.failures_by_client.len() < MAX_TRACKED_CLIENTS {
            self.failures_by_client
                .entry(client.to_string())
                .or_default()
                .push(now);
        }
    }
}

/// What redeeming a code came to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Redeemed {
    Accepted,
    Refused,
    Limited,
}

/// The one outstanding sign-in code, kept in memory: a restart voids it, and the owner asks
/// for another.
#[derive(Default)]
pub(crate) struct DeviceCodes {
    ledger: Mutex<Ledger>,
}

impl DeviceCodes {
    fn ledger(&self) -> MutexGuard<'_, Ledger> {
        self.ledger.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A fresh code, formatted `XXXXX-XXXXX`, replacing any earlier one; and when it expires.
    pub(crate) fn issue(&self, now: i64) -> (String, i64) {
        let code = hq_core::pairing::random_code();
        let salt = uuid::Uuid::new_v4().simple().to_string();
        let expires_at = now + CODE_TTL_SECS;
        self.ledger().pending = Some(Pending {
            hash: hash_code(&salt, &code),
            salt,
            expires_at,
        });
        (format_code(&code), expires_at)
    }

    /// Voids the pending code, for a sign-out everywhere.
    pub(crate) fn clear(&self) {
        self.ledger().pending = None;
    }

    /// Checks `presented` against the pending code. A limited client is refused before any
    /// comparison, and a match spends the code.
    pub(crate) fn redeem(&self, presented: &str, client: &str, now: i64) -> Redeemed {
        let mut ledger = self.ledger();
        ledger.prune(now);
        if ledger.limited(client) {
            return Redeemed::Limited;
        }
        let matched = ledger.pending.as_ref().is_some_and(|p| {
            p.expires_at > now && constant_time_eq(&hash_code(&p.salt, presented), &p.hash)
        });
        if !matched {
            ledger.record_failure(client, now);
            return Redeemed::Refused;
        }
        ledger.pending = None;
        Redeemed::Accepted
    }
}

/// Uppercase letters and digits only, so `abcde fghij` and `ABCDE-FGHIJ` are the same code.
fn normalize(code: &str) -> String {
    code.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn hash_code(salt: &str, code: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(normalize(code).as_bytes());
    hasher.finalize().into()
}

fn format_code(code: &str) -> String {
    let (a, b) = code.split_at(CODE_GROUP_LEN.min(code.len()));
    format!("{a}-{b}")
}

/// Who is guessing, for the per-client limit. Behind Tailscale Serve or Caddy every peer is
/// loopback, so this is the proxy's forwarding header; a direct caller can forge it, which is
/// why the global cap is the real bound.
fn client_key(headers: &HeaderMap) -> String {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    header("x-forwarded-for")
        .or_else(|| header("x-real-ip"))
        .unwrap_or(UNKNOWN_CLIENT)
        .to_string()
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub(crate) fn no_store(status: StatusCode, body: serde_json::Value) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// `POST /api/auth/device-code`: behind the web token, so only a signed-in device gets one.
pub(crate) async fn device_code_handler(State(state): State<Arc<WsState>>) -> Response {
    if !state.web_auth_enabled() {
        return ApiError::not_found().into_response();
    }
    let (code, expires_at) = state.device_codes.issue(now());
    no_store(
        StatusCode::OK,
        json!({ "code": code, "expires_at": expires_at, "expires_in": CODE_TTL_SECS }),
    )
}

#[derive(Deserialize)]
pub(crate) struct ExchangeBody {
    #[serde(default)]
    code: String,
}

/// `POST /api/auth/exchange`: trades a pending code for a new device token. Unauthenticated, so
/// a wrong, spent or expired code all get the same answer.
pub(crate) async fn exchange_handler(
    State(state): State<Arc<WsState>>,
    headers: HeaderMap,
    Json(body): Json<ExchangeBody>,
) -> Response {
    if !state.web_auth_enabled() {
        return ApiError::not_found().into_response();
    }
    let at = chrono::Utc::now();
    match state
        .device_codes
        .redeem(&body.code, &client_key(&headers), at.timestamp())
    {
        Redeemed::Limited => no_store(
            StatusCode::TOO_MANY_REQUESTS,
            json!({ "error": "too many attempts, wait a few minutes" }),
        ),
        Redeemed::Refused => no_store(
            StatusCode::UNAUTHORIZED,
            json!({ "error": "that code is not valid" }),
        ),
        Redeemed::Accepted => sign_in_with_code(&state, at),
    }
}

fn sign_in_with_code(state: &WsState, at: chrono::DateTime<chrono::Utc>) -> Response {
    let minted = match hq_db::web_auth::mint(&state.db, CODE_SIGN_IN_LABEL, at.timestamp(), None) {
        Ok(minted) => minted,
        Err(e) => {
            tracing::error!(
                "web: a one-time code was right but no device token could be saved: {e}"
            );
            return ApiError::internal("could not sign this device in, get a new code")
                .into_response();
        }
    };
    tracing::info!(device = %minted.id, "web: a device signed in with a one-time code");
    state.notify_owner(
        DEVICE_SIGN_IN_SOURCE,
        "A new device signed in to HQ",
        &format!(
            "A device signed in with a one-time code at {} UTC. If it was not you, open \
             Settings, Devices in HQ and sign it out.",
            at.format("%Y-%m-%d %H:%M")
        ),
    );
    no_store(StatusCode::OK, json!({ "token": minted.token }))
}

pub(crate) mod devices;

#[cfg(test)]
mod tests;
