//! Cross-origin protection for the web API, the chat socket and `/mcp`.
//!
//! A page on another site can make a victim's browser send requests to a local
//! instance. CORS alone does not stop that: it only hides responses, never
//! blocks a state-changing request or a WebSocket. So a browser request that
//! carries a foreign `Origin` is refused before it reaches a handler, and an
//! instance without a web token also checks `Host` against the names it is
//! reachable under, because a rebound DNS name makes the attacker same-origin.

use axum::http::{Method, StatusCode, header};
use axum::response::IntoResponse;
use std::net::IpAddr;

const LOCALHOST: &str = "localhost";

/// The host part of `host[:port]`, lowercased, with IPv6 brackets removed.
fn hostname(authority: &str) -> String {
    let authority = authority.trim().to_ascii_lowercase();
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    authority.split(':').next().unwrap_or("").to_string()
}

fn is_loopback_name(host: &str) -> bool {
    host == LOCALHOST
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// `scheme://authority` of an origin, lowercased and without a trailing slash.
fn normalize_origin(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn origin_authority(origin: &str) -> Option<&str> {
    origin.split_once("://").map(|(_, rest)| rest)
}

/// Whether a browser `Origin` may call this server: a loopback page, one of the
/// configured origins, or the page this server itself served (same origin).
pub(crate) fn origin_allowed(origin: &str, host_header: Option<&str>, allowed: &[String]) -> bool {
    let origin = normalize_origin(origin);
    let Some(authority) = origin_authority(&origin) else {
        return false;
    };
    if is_loopback_name(&hostname(authority)) {
        return true;
    }
    if allowed.iter().any(|a| normalize_origin(a) == origin) {
        return true;
    }
    host_header.is_some_and(|h| h.trim().eq_ignore_ascii_case(authority))
}

/// Whether `Host` names this server: loopback, or the host of a configured
/// origin. Only enforced when no web token is set; with a token, a rebound
/// name still cannot authenticate.
pub(crate) fn host_allowed(host_header: &str, allowed: &[String]) -> bool {
    let host = hostname(host_header);
    is_loopback_name(&host)
        || allowed.iter().any(|a| {
            origin_authority(&normalize_origin(a)).is_some_and(|auth| hostname(auth) == host)
        })
}

/// Paths a hostile page must not reach: everything the web token guards, plus
/// `/mcp`, whose key a browser would never hold but whose dev mode is open.
fn is_protected_path(path: &str) -> bool {
    crate::auth::web_path_is_guarded(path) || path == "/mcp" || path.starts_with("/mcp/")
}

/// Why a request is refused, if it is. `host` is the `Host` header, or the URI
/// authority on HTTP/2. A request with neither did not come from a browser.
pub(crate) fn rejection(
    method: &Method,
    path: &str,
    host: Option<&str>,
    origin: Option<&str>,
    allowed: &[String],
    has_web_token: bool,
) -> Option<&'static str> {
    if !is_protected_path(path) || method == Method::OPTIONS {
        return None;
    }
    if !has_web_token && host.is_some_and(|h| !host_allowed(h, allowed)) {
        return Some("unrecognised Host header");
    }
    match origin {
        Some(origin) if !origin_allowed(origin, host, allowed) => Some("cross-origin request refused"),
        _ => None,
    }
}

/// Header every browser request that changes a harness session must carry. A
/// hostile page on another origin cannot set it without a CORS preflight, and
/// the CORS layer does not list it, so the preflight fails even for a page on
/// another loopback port (which `origin_allowed` otherwise trusts).
pub(crate) const CLIENT_HEADER: &str = "x-hq-client";

const SESSION_API_PREFIX: &str = "/api/harness-sessions/";
/// First-run setup writes a credential, so its POSTs need the header too.
const SETUP_API_PREFIX: &str = "/api/setup/";

/// Endpoints that type into or take over a session. `drive`, `goal` and `unwatch` predate the
/// header and must keep working for a cached PWA that does not send it yet.
const HEADER_GUARDED_ACTIONS: [&str; 2] = ["send", "adopt"];

/// Whether the request types into or adopts a harness session without the client header.
pub(crate) fn missing_client_header(method: &Method, path: &str, has_header: bool) -> bool {
    let guarded = path.strip_prefix(SESSION_API_PREFIX).is_some_and(|rest| {
        rest.rsplit('/')
            .next()
            .is_some_and(|action| HEADER_GUARDED_ACTIONS.contains(&action))
    });
    let guarded = guarded || path.starts_with(SETUP_API_PREFIX);
    guarded && !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) && !has_header
}

fn request_host(request: &axum::extract::Request) -> Option<String> {
    request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| request.uri().authority().map(|a| a.to_string()))
}

/// Axum middleware applying [`rejection`] with the server's configured origins.
pub(crate) async fn origin_guard(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<crate::WsState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let has_token = state
        .web_auth_token
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty());
    let host = request_host(&request);
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    let refused = rejection(
        request.method(),
        request.uri().path(),
        host.as_deref(),
        origin,
        &state.allowed_origins,
        has_token,
    );
    let has_client_header = request.headers().contains_key(CLIENT_HEADER);
    let refused = refused.or_else(|| {
        missing_client_header(request.method(), request.uri().path(), has_client_header)
            .then_some("missing X-HQ-Client header")
    });
    if let Some(reason) = refused {
        tracing::warn!(path = %request.uri().path(), reason, "web: request refused by origin guard");
        return (StatusCode::FORBIDDEN, reason).into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: Option<&str> = Some("127.0.0.1:5678");
    const EVIL: Option<&str> = Some("https://evil.example");

    fn tailnet() -> Vec<String> {
        vec!["https://hq.example.ts.net:8443/".to_string()]
    }

    #[test]
    fn loopback_configured_and_same_origin_are_allowed() {
        let allowed = tailnet();
        assert!(origin_allowed("http://localhost:5173", None, &[]));
        assert!(origin_allowed("http://127.0.0.1:5678", None, &[]));
        assert!(origin_allowed("http://[::1]:5678", None, &[]));
        assert!(origin_allowed("https://HQ.example.ts.net:8443", None, &allowed));
        assert!(origin_allowed("https://box.lan:5678", Some("box.lan:5678"), &[]));
    }

    #[test]
    fn hostile_and_opaque_origins_are_refused() {
        let allowed = tailnet();
        assert!(!origin_allowed("https://evil.example", Some("127.0.0.1:5678"), &allowed));
        assert!(!origin_allowed("null", Some("127.0.0.1:5678"), &allowed));
        assert!(!origin_allowed("https://hq.example.ts.net:9999", None, &allowed));
        assert!(!origin_allowed("https://localhost.evil.example", None, &allowed));
    }

    #[test]
    fn host_check_stops_dns_rebinding() {
        let allowed = tailnet();
        assert!(host_allowed("127.0.0.1:5678", &allowed));
        assert!(host_allowed("localhost", &allowed));
        assert!(host_allowed("[::1]:5678", &allowed));
        assert!(host_allowed("hq.example.ts.net:8443", &allowed));
        assert!(host_allowed("hq.example.ts.net", &allowed));
        assert!(!host_allowed("rebind.evil.example:5678", &allowed));
    }

    #[test]
    fn state_changing_request_from_hostile_page_is_refused() {
        for path in ["/api/tasks", "/api/note/create", "/ws", "/mcp"] {
            assert!(rejection(&Method::POST, path, LOCAL, EVIL, &[], false).is_some(), "{path}");
            assert!(rejection(&Method::POST, path, LOCAL, EVIL, &[], true).is_some(), "{path}");
        }
        assert!(rejection(&Method::GET, "/ws", LOCAL, EVIL, &[], false).is_some());
        assert!(rejection(&Method::GET, "/api/tasks", LOCAL, EVIL, &[], false).is_some());
    }

    #[test]
    fn non_browser_clients_and_own_pages_pass() {
        assert_eq!(rejection(&Method::POST, "/mcp", LOCAL, None, &[], false), None);
        assert_eq!(rejection(&Method::POST, "/mcp", None, None, &[], false), None);
        let (host, origin) = (Some("hq.example.ts.net:8443"), Some("https://hq.example.ts.net:8443"));
        assert_eq!(rejection(&Method::POST, "/api/tasks", host, origin, &tailnet(), false), None);
        assert_eq!(rejection(&Method::GET, "/ws", host, origin, &tailnet(), false), None);
    }

    #[test]
    fn rebound_host_is_refused_only_without_a_token() {
        let rebound = Some("rebind.evil.example:5678");
        assert!(rejection(&Method::GET, "/api/tasks", rebound, None, &[], false).is_some());
        assert_eq!(rejection(&Method::GET, "/api/tasks", rebound, None, &[], true), None);
        assert_eq!(rejection(&Method::GET, "/health", rebound, None, &[], false), None);
        assert_eq!(rejection(&Method::GET, "/", rebound, None, &[], false), None);
    }

    #[test]
    fn session_posts_need_the_client_header() {
        let post = |path: &str, header: bool| missing_client_header(&Method::POST, path, header);
        for path in ["send", "adopt"] {
            let full = format!("/api/harness-sessions/hs-1/{path}");
            assert!(post(&full, false), "{path} without the header");
            assert!(!post(&full, true), "{path} with the header");
        }
        for path in ["drive", "goal", "unwatch"] {
            let full = format!("/api/harness-sessions/hs-1/{path}");
            assert!(!post(&full, false), "{path} stays open to cached clients");
        }
        assert!(!missing_client_header(
            &Method::GET,
            "/api/harness-sessions/hs-1",
            false
        ));
        assert!(post("/api/setup/provider", false), "setup writes a credential");
        assert!(!post("/api/setup/provider", true));
        assert!(!missing_client_header(&Method::GET, "/api/setup/status", false));
        assert!(!post("/api/tasks", false), "other routes are unchanged");
    }

    #[test]
    fn preflight_is_left_to_the_cors_layer() {
        assert_eq!(rejection(&Method::OPTIONS, "/api/tasks", LOCAL, EVIL, &[], false), None);
    }
}
