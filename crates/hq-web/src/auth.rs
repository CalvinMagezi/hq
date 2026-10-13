//! Caller identity resolution for `/mcp`: `AGENTHQ_API_KEY` (full access),
//! `AGENTHQ_SPARK_API_KEY` (restricted to `hq_mcp::gateway::SPARK_READONLY_ALLOWLIST`
//! by the caller), `AGENTHQ_TASKS_API_KEY` (`TASKS_ALLOWLIST`: task reads and writes
//! and nothing that runs code) and `AGENTHQ_HANDOFF_API_KEY` (`HANDOFF_ALLOWLIST`: task
//! writes and session spawn/send on top of the reads). With none set, `/mcp` refuses every request unless the
//! loopback-only development switch `HQ_MCP_DEV_NO_AUTH=1` is on.
//!
//! The web token for `/ws` and `/api` travels only in the Authorization header.
//! Browsers cannot set headers on a WebSocket, so the chat socket takes a
//! single-use ticket from `POST /api/ws-ticket` instead of the token itself.

use axum::http::HeaderMap;
use axum::response::IntoResponse;
use hq_core::middleware::resolve_api_key_candidate;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Env switch that leaves `/mcp` open when no key is configured. Honoured only
/// on a loopback bind and for requests no proxy forwarded, so turning it on in
/// a deployment behind Caddy or Tailscale has no effect.
pub(crate) const MCP_DEV_NO_AUTH_ENV: &str = "HQ_MCP_DEV_NO_AUTH";

/// Headers a reverse proxy adds. Their presence means the caller is not local.
const FORWARDING_HEADERS: [&str; 5] = [
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
    "tailscale-user-login",
];

/// How long a socket ticket stays redeemable. Long enough for the browser to
/// open the socket right after asking, short enough that a leaked URL is dead.
pub(crate) const WS_TICKET_TTL: Duration = Duration::from_secs(30);

/// Outstanding tickets are bounded so an authenticated client looping on the
/// mint endpoint cannot grow memory without limit.
const MAX_OUTSTANDING_TICKETS: usize = 256;

/// Which caller identity a request resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiIdentity {
    /// The original key. Unrestricted.
    Full,
    /// The Spark-scoped key. Callers must apply their own tool allowlist.
    Spark,
    /// The tasks-scoped key: the task tools and nothing else (no vault, no session
    /// tools, no code execution). Callers must apply `TASKS_ALLOWLIST`.
    Tasks,
    /// The handoff-scoped key: Spark's reads plus task writes and session
    /// spawn/send. Callers must apply `HANDOFF_ALLOWLIST`.
    Handoff,
}

/// Resolve the caller's identity against explicit key values rather than
/// reading `std::env::var` directly, so this stays a pure function callers
/// can unit test without mutating process-global state. `dev_open` comes from
/// [`mcp_dev_open`] and only matters when no key is configured. When one value
/// is configured for several scopes the narrowest wins (Spark, then Tasks, then
/// Handoff, then Full), so a duplicated key never widens access.
pub(crate) fn resolve_identity(
    headers: &HeaderMap,
    full_key: Option<&str>,
    spark_key: Option<&str>,
    handoff_key: Option<&str>,
    tasks_key: Option<&str>,
    dev_open: bool,
) -> Option<ApiIdentity> {
    if full_key.is_none() && spark_key.is_none() && handoff_key.is_none() && tasks_key.is_none() {
        return dev_open.then_some(ApiIdentity::Full);
    }

    let x_api_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let provided = resolve_api_key_candidate(x_api_key, bearer);

    if provided.is_empty() {
        return None;
    }
    let matches = |key: Option<&str>| key.is_some_and(|k| tokens_match(provided, k));
    if matches(spark_key) {
        return Some(ApiIdentity::Spark);
    }
    if matches(tasks_key) {
        return Some(ApiIdentity::Tasks);
    }
    if matches(handoff_key) {
        return Some(ApiIdentity::Handoff);
    }
    matches(full_key).then_some(ApiIdentity::Full)
}

/// The secret a request presents, from `x-api-key` or a Bearer header.
pub(crate) fn presented_secret(headers: &HeaderMap) -> &str {
    let x_api_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    resolve_api_key_candidate(x_api_key, bearer)
}

/// Whether `path` needs the web token: the chat sockets and the REST API.
/// `/health` (deploy checks), `/mcp` (own key) and static files stay open, and so does
/// the device-code exchange, which a device without the token must reach.
pub(crate) fn web_path_is_guarded(path: &str) -> bool {
    let api = path == "/ws" || path.starts_with("/ws/") || path.starts_with("/api/");
    api && path != crate::device_auth::EXCHANGE_PATH
}

/// Compare without an early exit, so response timing doesn't leak the token.
pub(crate) fn tokens_match(provided: &str, expected: &str) -> bool {
    let (a, b) = (provided.as_bytes(), expected.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether the development switch may open `/mcp` for this request.
pub(crate) fn mcp_dev_open(headers: &HeaderMap, switch_on: bool, bind_is_loopback: bool) -> bool {
    switch_on && bind_is_loopback && !FORWARDING_HEADERS.iter().any(|h| headers.contains_key(*h))
}

/// Whether a request carries the web token as a Bearer header. Tokens in the
/// URL are refused: they end up in history, proxy logs and referrers.
pub(crate) fn web_request_authorized(headers: &HeaderMap, expected: &str) -> bool {
    bearer(headers).is_some_and(|t| tokens_match(t, expected))
}

/// Single-use, short-lived tickets that stand in for the web token on the
/// chat socket, the one request a browser cannot put a header on.
#[derive(Default)]
pub(crate) struct WsTickets {
    issued: Mutex<HashMap<String, Instant>>,
}

impl WsTickets {
    pub(crate) fn mint(&self) -> String {
        let ticket = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut issued = self.issued.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        issued.retain(|_, expires| *expires > now);
        if issued.len() >= MAX_OUTSTANDING_TICKETS {
            let oldest = issued
                .iter()
                .min_by_key(|(_, e)| **e)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                issued.remove(&oldest);
            }
        }
        issued.insert(ticket.clone(), now + WS_TICKET_TTL);
        ticket
    }

    /// Voids every outstanding ticket.
    pub(crate) fn clear(&self) {
        self.issued
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Redeem a ticket. It is removed either way, so it can never be replayed.
    pub(crate) fn redeem(&self, ticket: &str) -> bool {
        let mut issued = self.issued.lock().unwrap_or_else(|e| e.into_inner());
        issued
            .remove(ticket)
            .is_some_and(|expires| expires > Instant::now())
    }
}

fn query_param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(k, _)| *k == name)
            .map(|(_, v)| v)
    })
}

pub fn bind_is_loopback(bind: &str) -> bool {
    bind == "localhost"
        || bind
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Refuse to serve the web API off-host without a token: every `/api` route
/// and the chat socket would otherwise be open to the network.
pub fn check_web_bind(bind: &str, token: Option<&str>) -> Result<(), String> {
    if bind_is_loopback(bind) || token.is_some_and(|t| !t.trim().is_empty()) {
        return Ok(());
    }
    Err(format!(
        "web_bind is {bind}, which exposes /ws and /api to the network without auth. \
         Set web_auth_token (or HQ_WEB_AUTH_TOKEN), or bind to 127.0.0.1 and put a \
         reverse proxy or Tailscale in front."
    ))
}

/// Which credential a request signed in with, kept in its extensions for handlers that care.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WebCredential {
    /// The configured web token (`web_auth_token`, `HQ_WEB_AUTH_TOKEN`).
    Configured,
    /// A device token, by its row id in `web_device_tokens`.
    Device(String),
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

/// Whether the configured token was signed out, cached on `WsState` so the configured-token
/// path needs no database read while the answer is fresh.
pub(crate) const CONFIGURED_UNKNOWN: u8 = 0;
const CONFIGURED_LIVE: u8 = 1;
const CONFIGURED_SIGNED_OUT: u8 = 2;
/// Another server on the same database (an `hq web` beside `hq start all`) may sign it out, so a
/// "live" answer is read again after this long.
const CONFIGURED_RECHECK_SECS: i64 = 60;

impl crate::WsState {
    /// The configured web token, when it is set and not blank. It may have been signed out.
    pub(crate) fn configured_web_token(&self) -> Option<&str> {
        self.web_auth_token
            .as_deref()
            .filter(|t| !t.trim().is_empty())
    }

    /// Whether `/ws` and `/api` ask for a sign-in: a configured token exists, signed out or
    /// not, or a device token does. A database error counts as yes, so it never opens the API.
    pub(crate) fn web_auth_enabled(&self) -> bool {
        hq_db::web_auth::sign_in_enabled(&self.db, self.configured_web_token()).unwrap_or_else(
            |e| {
                tracing::warn!(
                    "web auth: could not read device tokens, treating sign-in as on: {e}"
                );
                true
            },
        )
    }

    /// The credential `secret` is, if any: the configured token unless the owner signed it
    /// out, or a device token. A database error refuses.
    pub(crate) fn web_credential(&self, secret: &str) -> Option<WebCredential> {
        if secret.is_empty() {
            return None;
        }
        if let Some(configured) = self.configured_web_token()
            && tokens_match(secret, configured)
        {
            return self.configured_token_live(configured);
        }
        let now = chrono::Utc::now().timestamp();
        match hq_db::web_auth::verify(&self.db, secret, now) {
            Ok(device) => device.map(WebCredential::Device),
            Err(e) => {
                tracing::warn!("web auth: could not check a device token: {e}");
                None
            }
        }
    }

    /// [`Self::web_auth_enabled`] off the async worker threads. A configured token answers
    /// without touching the database.
    pub(crate) async fn web_auth_enabled_async(self: &std::sync::Arc<Self>) -> bool {
        if self.configured_web_token().is_some() {
            return true;
        }
        let state = self.clone();
        tokio::task::spawn_blocking(move || state.web_auth_enabled())
            .await
            .unwrap_or(true)
    }

    /// The configured token, when `secret` is it and it was seen live within
    /// [`CONFIGURED_RECHECK_SECS`]: no database read.
    fn configured_credential_cached(&self, secret: &str) -> Option<WebCredential> {
        let configured = self.configured_web_token()?;
        (self.configured_live_and_fresh() && tokens_match(secret, configured))
            .then_some(WebCredential::Configured)
    }

    fn configured_live_and_fresh(&self) -> bool {
        use std::sync::atomic::Ordering;
        let checked = self.configured_checked_at.load(Ordering::Acquire);
        self.configured_state.load(Ordering::Acquire) == CONFIGURED_LIVE
            && chrono::Utc::now().timestamp() - checked < CONFIGURED_RECHECK_SECS
    }

    /// Reads whether the configured token was signed out into the cache. Called when the server
    /// starts; a database error leaves it unknown, to be read again on the next request.
    pub(crate) fn load_configured_state(&self) {
        if let Some(configured) = self.configured_web_token() {
            let _ = self.configured_token_live(configured);
        }
    }

    /// Records that the configured token is signed out. It stays so for this process: the
    /// revocation is by hash, and only a new value (which needs a restart) undoes it.
    pub(crate) fn mark_configured_signed_out(&self) {
        self.configured_state
            .store(CONFIGURED_SIGNED_OUT, std::sync::atomic::Ordering::Release);
    }

    /// Live or signed out, from the cache when fresh. A database error keeps an earlier "live"
    /// (so an instance that never rotates is not locked out by a busy database) and refuses
    /// when nothing is known yet.
    fn configured_token_live(&self, configured: &str) -> Option<WebCredential> {
        use std::sync::atomic::Ordering;
        if self.configured_state.load(Ordering::Acquire) == CONFIGURED_SIGNED_OUT {
            return None;
        }
        if self.configured_live_and_fresh() {
            return Some(WebCredential::Configured);
        }
        match hq_db::web_auth::is_revoked(&self.db, configured) {
            Ok(false) => {
                self.configured_checked_at
                    .store(chrono::Utc::now().timestamp(), Ordering::Release);
                let _ = self.configured_state.compare_exchange(
                    CONFIGURED_UNKNOWN,
                    CONFIGURED_LIVE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                self.configured_token_known_live()
            }
            Ok(true) => {
                self.mark_configured_signed_out();
                None
            }
            Err(e) => {
                tracing::warn!(
                    "web auth: could not check whether the web token was signed out: {e}"
                );
                self.configured_token_known_live()
            }
        }
    }

    fn configured_token_known_live(&self) -> Option<WebCredential> {
        (self
            .configured_state
            .load(std::sync::atomic::Ordering::Acquire)
            == CONFIGURED_LIVE)
            .then_some(WebCredential::Configured)
    }

    /// After a sign-out: closes every open chat socket and voids outstanding socket tickets.
    /// Devices still signed in reconnect with their own token.
    pub(crate) fn end_web_sessions(&self) {
        self.ws_tickets.clear();
        self.auth_epoch
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
    }
}

/// Axum middleware enforcing the web sign-in on guarded paths. A no-op when no web token is
/// configured and no device token exists (loopback-only deployments).
pub(crate) async fn web_auth_middleware(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::WsState>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !web_path_is_guarded(request.uri().path()) {
        return next.run(request).await;
    }
    let presented = bearer(request.headers()).map(str::to_string);
    let cached = presented
        .as_deref()
        .and_then(|t| state.configured_credential_cached(t));
    let (enabled, credential) = match cached {
        Some(credential) => (true, Some(credential)),
        None => {
            let checking = state.clone();
            tokio::task::spawn_blocking(move || {
                let credential = presented.and_then(|t| checking.web_credential(&t));
                (checking.web_auth_enabled(), credential)
            })
            .await
            .unwrap_or((true, None))
        }
    };
    if !enabled {
        return next.run(request).await;
    }
    if let Some(credential) = credential {
        request.extensions_mut().insert(credential);
        return next.run(request).await;
    }
    let uri = request.uri();
    let ticket_ok = uri.path() == "/ws"
        && query_param(uri.query(), "ticket").is_some_and(|t| state.ws_tickets.redeem(t));
    if !ticket_ok {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

/// `POST /api/ws-ticket`: a ticket for opening `/ws`. Sits behind the web auth
/// middleware, so only a caller holding the token gets one.
pub(crate) async fn ws_ticket_handler(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::WsState>>,
) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "ticket": state.ws_tickets.mint(),
        "expires_in": WS_TICKET_TTL.as_secs(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(header_name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header_name.parse::<axum::http::HeaderName>().unwrap(),
            value.parse().unwrap(),
        );
        h
    }

    #[test]
    fn guards_socket_and_api_but_not_health_mcp_or_oauth_redirect() {
        for p in [
            "/ws",
            "/ws/other",
            "/api/tasks",
            "/api/notifications/x/action",
        ] {
            assert!(web_path_is_guarded(p), "{p}");
        }
        for p in [
            "/health",
            "/mcp",
            "/mcp/sse",
            "/",
            "/assets/app.js",
            "/wsx",
            "/api/auth/exchange",
        ] {
            assert!(!web_path_is_guarded(p), "{p}");
        }
        for p in [
            "/api/auth/device-code",
            "/api/auth/exchange/x",
            "/api/auth/exchangeX",
            "/api/auth/exchange/",
        ] {
            assert!(
                web_path_is_guarded(p),
                "only the exact exchange path is open: {p}"
            );
        }
    }

    #[test]
    fn web_token_accepted_only_from_bearer_header() {
        let bearer = headers_with("authorization", "Bearer s3cret");
        assert!(web_request_authorized(&bearer, "s3cret"));
        assert!(!web_request_authorized(&HeaderMap::new(), "s3cret"));
        let wrong = headers_with("authorization", "Bearer nope");
        assert!(!web_request_authorized(&wrong, "s3cret"));
        let short = headers_with("authorization", "Bearer s3cre");
        assert!(!web_request_authorized(&short, "s3cret"));
    }

    #[test]
    fn ws_tickets_are_single_use() {
        let tickets = WsTickets::default();
        let ticket = tickets.mint();
        assert!(!tickets.redeem("made-up"));
        assert!(tickets.redeem(&ticket));
        assert!(!tickets.redeem(&ticket), "a ticket must not be replayable");
    }

    #[test]
    fn ws_tickets_expire() {
        let tickets = WsTickets::default();
        let ticket = tickets.mint();
        tickets
            .issued
            .lock()
            .unwrap()
            .insert(ticket.clone(), Instant::now() - Duration::from_secs(1));
        assert!(!tickets.redeem(&ticket));
    }

    #[test]
    fn outstanding_tickets_are_bounded() {
        let tickets = WsTickets::default();
        for _ in 0..MAX_OUTSTANDING_TICKETS + 10 {
            tickets.mint();
        }
        assert!(tickets.issued.lock().unwrap().len() <= MAX_OUTSTANDING_TICKETS);
    }

    #[test]
    fn query_param_matches_whole_names() {
        assert_eq!(query_param(Some("a=1&ticket=abc"), "ticket"), Some("abc"));
        assert_eq!(query_param(Some("xticket=abc"), "ticket"), None);
        assert_eq!(query_param(None, "ticket"), None);
    }

    #[test]
    fn non_loopback_bind_needs_a_token() {
        assert!(check_web_bind("127.0.0.1", None).is_ok());
        assert!(check_web_bind("::1", None).is_ok());
        assert!(check_web_bind("localhost", None).is_ok());
        assert!(check_web_bind("0.0.0.0", None).is_err());
        assert!(check_web_bind("0.0.0.0", Some("  ")).is_err());
        assert!(check_web_bind("0.0.0.0", Some("s3cret")).is_ok());
    }

    #[test]
    fn closed_when_no_keys_configured() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_identity(&headers, None, None, None, None, false),
            None
        );
        let any_key = headers_with("x-api-key", "anything");
        assert_eq!(
            resolve_identity(&any_key, None, None, None, None, false),
            None
        );
    }

    #[test]
    fn dev_switch_opens_only_without_keys() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_identity(&headers, None, None, None, None, true),
            Some(ApiIdentity::Full)
        );
        assert_eq!(
            resolve_identity(&headers, Some("full-secret"), None, None, None, true),
            None
        );
    }

    #[test]
    fn dev_switch_needs_loopback_bind_and_an_unproxied_request() {
        let local = HeaderMap::new();
        assert!(mcp_dev_open(&local, true, true));
        assert!(!mcp_dev_open(&local, false, true));
        assert!(!mcp_dev_open(&local, true, false));
        for h in FORWARDING_HEADERS {
            let proxied = headers_with(h, "203.0.113.9");
            assert!(!mcp_dev_open(&proxied, true, true), "{h}");
        }
    }

    #[test]
    fn matches_full_key_via_x_api_key() {
        let headers = headers_with("x-api-key", "full-secret");
        assert_eq!(
            resolve_identity(
                &headers,
                Some("full-secret"),
                Some("spark-secret"),
                None,
                None,
                false
            ),
            Some(ApiIdentity::Full)
        );
    }

    #[test]
    fn matches_spark_key_via_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer spark-secret".parse().unwrap(),
        );
        assert_eq!(
            resolve_identity(
                &headers,
                Some("full-secret"),
                Some("spark-secret"),
                None,
                None,
                false
            ),
            Some(ApiIdentity::Spark)
        );
    }

    #[test]
    fn rejects_unknown_key() {
        let headers = headers_with("x-api-key", "not-a-real-key");
        assert_eq!(
            resolve_identity(
                &headers,
                Some("full-secret"),
                Some("spark-secret"),
                None,
                None,
                false
            ),
            None
        );
    }

    #[test]
    fn rejects_missing_key_when_keys_configured() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_identity(
                &headers,
                Some("full-secret"),
                Some("spark-secret"),
                None,
                None,
                false
            ),
            None
        );
    }

    #[test]
    fn matches_handoff_key_and_never_full() {
        let headers = headers_with("x-api-key", "handoff-secret");
        assert_eq!(
            resolve_identity(
                &headers,
                Some("full-secret"),
                Some("spark-secret"),
                Some("handoff-secret"),
                None,
                false
            ),
            Some(ApiIdentity::Handoff)
        );
        let full = headers_with("x-api-key", "full-secret");
        assert_eq!(
            resolve_identity(
                &full,
                Some("full-secret"),
                Some("spark-secret"),
                Some("handoff-secret"),
                None,
                false
            ),
            Some(ApiIdentity::Full),
            "the full key is unchanged"
        );
    }

    #[test]
    fn a_handoff_key_alone_opens_only_the_handoff_scope() {
        let headers = headers_with("x-api-key", "handoff-secret");
        assert_eq!(
            resolve_identity(&headers, None, None, Some("handoff-secret"), None, false),
            Some(ApiIdentity::Handoff)
        );
        let wrong = headers_with("x-api-key", "nope");
        assert_eq!(
            resolve_identity(&wrong, None, None, Some("handoff-secret"), None, false),
            None
        );
        assert_eq!(
            resolve_identity(
                &HeaderMap::new(),
                None,
                None,
                Some("handoff-secret"),
                None,
                false
            ),
            None
        );
        assert_eq!(
            resolve_identity(&headers, None, None, Some("other"), None, true),
            None,
            "a configured key closes the dev switch"
        );
    }

    #[test]
    fn a_key_reused_across_scopes_gets_the_narrowest() {
        let headers = headers_with("x-api-key", "same");
        assert_eq!(
            resolve_identity(&headers, Some("same"), None, Some("same"), None, false),
            Some(ApiIdentity::Handoff)
        );
        assert_eq!(
            resolve_identity(
                &headers,
                Some("same"),
                Some("same"),
                Some("same"),
                None,
                false
            ),
            Some(ApiIdentity::Spark)
        );
        assert_eq!(
            resolve_identity(&headers, Some("same"), None, None, None, false),
            Some(ApiIdentity::Full)
        );
    }

    #[test]
    fn spark_key_alone_resolves_spark() {
        let headers = headers_with("x-api-key", "spark-secret");
        assert_eq!(
            resolve_identity(&headers, None, Some("spark-secret"), None, None, false),
            Some(ApiIdentity::Spark)
        );
    }
    #[test]
    fn tasks_key_resolves_tasks_and_never_wider() {
        let headers = headers_with("x-api-key", "tasks-secret");
        assert_eq!(
            resolve_identity(
                &headers,
                Some("full"),
                Some("spark"),
                Some("handoff"),
                Some("tasks-secret"),
                false
            ),
            Some(ApiIdentity::Tasks)
        );
        assert_eq!(
            resolve_identity(&headers, None, None, None, Some("tasks-secret"), false),
            Some(ApiIdentity::Tasks),
            "a tasks key alone opens the tasks scope"
        );
        let wrong = headers_with("x-api-key", "nope");
        assert_eq!(
            resolve_identity(&wrong, None, None, None, Some("tasks-secret"), false),
            None
        );
        assert_eq!(
            resolve_identity(&headers, None, None, None, Some("other"), true),
            None,
            "a configured tasks key closes the dev switch"
        );
    }

    #[test]
    fn a_key_shared_with_tasks_gets_the_narrower_scope() {
        let headers = headers_with("x-api-key", "same");
        assert_eq!(
            resolve_identity(&headers, Some("same"), None, None, Some("same"), false),
            Some(ApiIdentity::Tasks)
        );
        assert_eq!(
            resolve_identity(&headers, None, None, Some("same"), Some("same"), false),
            Some(ApiIdentity::Tasks),
            "tasks is narrower than handoff, which can run code"
        );
        assert_eq!(
            resolve_identity(&headers, None, Some("same"), None, Some("same"), false),
            Some(ApiIdentity::Spark),
            "spark stays the narrowest"
        );
    }
}
