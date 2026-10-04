//! Caller identity resolution for `/mcp`: `AGENTHQ_API_KEY` (full access),
//! `AGENTHQ_SPARK_API_KEY` (restricted to `hq_mcp::gateway::SPARK_READONLY_ALLOWLIST`
//! by the caller) and `AGENTHQ_HANDOFF_API_KEY` (`HANDOFF_ALLOWLIST`: task writes
//! and session spawn/send on top of the reads). With neither set, `/mcp` refuses every request unless the
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
    /// The handoff-scoped key: Spark's reads plus task writes and session
    /// spawn/send. Callers must apply `HANDOFF_ALLOWLIST`.
    Handoff,
}

/// Resolve the caller's identity against explicit key values rather than
/// reading `std::env::var` directly, so this stays a pure function callers
/// can unit test without mutating process-global state. `dev_open` comes from
/// [`mcp_dev_open`] and only matters when no key is configured. When one value
/// is configured for several scopes the narrowest wins, so a duplicated key
/// never widens access.
pub(crate) fn resolve_identity(
    headers: &HeaderMap,
    full_key: Option<&str>,
    spark_key: Option<&str>,
    handoff_key: Option<&str>,
    dev_open: bool,
) -> Option<ApiIdentity> {
    if full_key.is_none() && spark_key.is_none() && handoff_key.is_none() {
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
    if matches(handoff_key) {
        return Some(ApiIdentity::Handoff);
    }
    matches(full_key).then_some(ApiIdentity::Full)
}

/// Whether `path` needs the web token: the chat sockets and the REST API.
/// `/health` (deploy checks), `/mcp` (own key) and static files stay open.
pub(crate) fn web_path_is_guarded(path: &str) -> bool {
    path == "/ws" || path.starts_with("/ws/") || path.starts_with("/api/")
}

/// Compare without an early exit, so response timing doesn't leak the token.
pub(crate) fn tokens_match(provided: &str, expected: &str) -> bool {
    let (a, b) = (provided.as_bytes(), expected.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether the development switch may open `/mcp` for this request.
pub(crate) fn mcp_dev_open(headers: &HeaderMap, switch_on: bool, bind_is_loopback: bool) -> bool {
    switch_on
        && bind_is_loopback
        && !FORWARDING_HEADERS.iter().any(|h| headers.contains_key(*h))
}

/// Whether a request carries the web token as a Bearer header. Tokens in the
/// URL are refused: they end up in history, proxy logs and referrers.
pub(crate) fn web_request_authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| tokens_match(t, expected))
}

/// Single-use, short-lived tickets that stand in for the web token on the
/// chat socket, the one request a browser cannot put a header on.
#[derive(Default)]
pub(crate) struct WsTickets {
    issued: Mutex<HashMap<String, Instant>>,
}

impl WsTickets {
    pub(crate) fn mint(&self) -> String {
        let ticket = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let mut issued = self.issued.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        issued.retain(|_, expires| *expires > now);
        if issued.len() >= MAX_OUTSTANDING_TICKETS {
            let oldest = issued.iter().min_by_key(|(_, e)| **e).map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                issued.remove(&oldest);
            }
        }
        issued.insert(ticket.clone(), now + WS_TICKET_TTL);
        ticket
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

pub(crate) fn bind_is_loopback(bind: &str) -> bool {
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

/// Axum middleware enforcing `web_auth_token` on guarded paths. A no-op when
/// no token is configured (loopback-only deployments).
pub(crate) async fn web_auth_middleware(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::WsState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let expected = state
        .web_auth_token
        .as_deref()
        .filter(|t| !t.trim().is_empty());
    let Some(expected) = expected else {
        return next.run(request).await;
    };
    let uri = request.uri();
    if !web_path_is_guarded(uri.path()) || web_request_authorized(request.headers(), expected) {
        return next.run(request).await;
    }
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
        for p in ["/ws", "/ws/other", "/api/tasks", "/api/notifications/x/action"] {
            assert!(web_path_is_guarded(p), "{p}");
        }
        for p in ["/health", "/mcp", "/mcp/sse", "/", "/assets/app.js", "/wsx"] {
            assert!(!web_path_is_guarded(p), "{p}");
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
        assert_eq!(resolve_identity(&headers, None, None, None, false), None);
        let any_key = headers_with("x-api-key", "anything");
        assert_eq!(resolve_identity(&any_key, None, None, None, false), None);
    }

    #[test]
    fn dev_switch_opens_only_without_keys() {
        let headers = HeaderMap::new();
        assert_eq!(resolve_identity(&headers, None, None, None, true), Some(ApiIdentity::Full));
        assert_eq!(resolve_identity(&headers, Some("full-secret"), None, None, true), None);
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
            resolve_identity(&headers, Some("full-secret"), Some("spark-secret"), None, false),
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
            resolve_identity(&headers, Some("full-secret"), Some("spark-secret"), None, false),
            Some(ApiIdentity::Spark)
        );
    }

    #[test]
    fn rejects_unknown_key() {
        let headers = headers_with("x-api-key", "not-a-real-key");
        assert_eq!(
            resolve_identity(&headers, Some("full-secret"), Some("spark-secret"), None, false),
            None
        );
    }

    #[test]
    fn rejects_missing_key_when_keys_configured() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_identity(&headers, Some("full-secret"), Some("spark-secret"), None, false),
            None
        );
    }

    #[test]
    fn matches_handoff_key_and_never_full() {
        let headers = headers_with("x-api-key", "handoff-secret");
        assert_eq!(
            resolve_identity(&headers, Some("full-secret"), Some("spark-secret"), Some("handoff-secret"), false),
            Some(ApiIdentity::Handoff)
        );
        let full = headers_with("x-api-key", "full-secret");
        assert_eq!(
            resolve_identity(&full, Some("full-secret"), Some("spark-secret"), Some("handoff-secret"), false),
            Some(ApiIdentity::Full),
            "the full key is unchanged"
        );
    }

    #[test]
    fn a_handoff_key_alone_opens_only_the_handoff_scope() {
        let headers = headers_with("x-api-key", "handoff-secret");
        assert_eq!(
            resolve_identity(&headers, None, None, Some("handoff-secret"), false),
            Some(ApiIdentity::Handoff)
        );
        let wrong = headers_with("x-api-key", "nope");
        assert_eq!(resolve_identity(&wrong, None, None, Some("handoff-secret"), false), None);
        assert_eq!(resolve_identity(&HeaderMap::new(), None, None, Some("handoff-secret"), false), None);
        assert_eq!(
            resolve_identity(&headers, None, None, Some("other"), true),
            None,
            "a configured key closes the dev switch"
        );
    }

    #[test]
    fn a_key_reused_across_scopes_gets_the_narrowest() {
        let headers = headers_with("x-api-key", "same");
        assert_eq!(
            resolve_identity(&headers, Some("same"), None, Some("same"), false),
            Some(ApiIdentity::Handoff)
        );
        assert_eq!(
            resolve_identity(&headers, Some("same"), Some("same"), Some("same"), false),
            Some(ApiIdentity::Spark)
        );
        assert_eq!(
            resolve_identity(&headers, Some("same"), None, None, false),
            Some(ApiIdentity::Full)
        );
    }

    #[test]
    fn spark_key_alone_resolves_spark() {
        let headers = headers_with("x-api-key", "spark-secret");
        assert_eq!(
            resolve_identity(&headers, None, Some("spark-secret"), None, false),
            Some(ApiIdentity::Spark)
        );
    }
}
