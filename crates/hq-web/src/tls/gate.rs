//! What a request must be before the router sees it, on the connector's own TLS door.
//!
//! The door faces the internet through a forwarder that reads the TLS server name and nothing
//! else, so the checks Caddy or a tunnel used to do in front of HQ happen here: only the
//! connector's paths, only its own name in `Host`, no credential that belongs to another door,
//! and a mark that the request did not come from this machine. A request that fails a check is a
//! plain 404, whichever check it was, and no handler, middleware or auth has run.

use axum::Router;
use axum::body::Body;
use axum::http::header::{AUTHORIZATION, HOST};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// The paths Claude's servers call, matched exactly: `deploy/Caddyfile.connector`'s list. The
/// consent side (`/oauth/authorize`, `/connect`, `/api/oauth/requests/*`) is not here and is
/// never served on this door: it belongs to the owner's browser on a private origin, and the
/// handler that starts it trusts `Host`, which a public name must not be asked to protect.
/// Whether a public name serves consent at all is decided with the connector-on-the-bridge work.
pub const CONNECTOR_PATHS: [&str; 6] = [
    "/mcp",
    "/oauth/token",
    "/oauth/revoke",
    "/.well-known/oauth-protected-resource",
    "/.well-known/oauth-protected-resource/mcp",
    "/.well-known/oauth-authorization-server",
];

/// Whether `path` is one of the connector's, byte for byte. No decoding and no normalising:
/// the router matches the same string, so what this lets through is what the router sees.
pub fn is_connector_path(path: &str) -> bool {
    CONNECTOR_PATHS.contains(&path)
}

/// Whether `path` is anywhere the connector lives, in any spelling a router might send to it:
/// the question for a listener that must never serve the connector (the own-devices door).
pub fn is_connector_territory(path: &str) -> bool {
    let first = |prefix: &str| path == prefix || path.starts_with(&format!("{prefix}/"));
    first("/mcp") || first("/oauth") || first("/.well-known")
}

/// Wraps `router` so it answers 404 for anything in the connector's territory before any of its
/// handlers or auth run. For the listener of the own-devices door: the two doors share no path.
pub fn refuse_connector_paths(router: Router) -> Router {
    router.layer(axum::middleware::from_fn(
        |req: Request<Body>, next: Next| async move {
            if is_connector_territory(req.uri().path()) {
                return not_found();
            }
            next.run(req).await
        },
    ))
}

/// The one answer to anything refused.
pub(crate) fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

/// Headers a proxy in front of HQ would add, and those HQ reads as "this caller is local" or as
/// an identity handed over by a tunnel. A client of this door chooses what it sends, so none of
/// them is believed.
pub(crate) const STRIPPED_HEADERS: [&str; 10] = [
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-real-ip",
    "tailscale-user-login",
    "tailscale-user-name",
    "tailscale-user-profile-pic",
    "tailscale-headers-info",
    "x-hq-session-id",
];
/// `mcp_http` opens `/mcp` for the development switch only when none of the forwarding headers
/// is present. This door adds one to every request, so the switch never opens the internet.
const FORWARDED_VALUE: &str = "by=hq-connector-tls;proto=https";
const API_KEY_HEADER: &str = "x-api-key";
const BEARER: &str = "Bearer ";
/// Access tokens start with this (`hq_db::mcp_connector::ACCESS_PREFIX`).
const ACCESS_PREFIX: &str = hq_db::mcp_connector::ACCESS_PREFIX;

/// Whether an access token is one a connector grant holds right now.
pub(crate) type GrantCheck = Arc<dyn Fn(&str) -> bool + Send + Sync>;

pub(crate) struct Gate {
    authorities: Vec<String>,
    canonical_host: HeaderValue,
    live_grant: GrantCheck,
    /// Stands in for a credential that is not a connector token. It looks like one, so the
    /// handler answers `invalid_token` and the client refreshes or starts over as usual, but it
    /// matches nothing, not even a key the owner chose with the same prefix.
    refused_credential: HeaderValue,
}

impl Gate {
    /// `authority` is the public origin's host, with `:port` when it names one.
    pub fn new(authority: &str, live_grant: GrantCheck) -> Self {
        let authority = authority.to_ascii_lowercase();
        let mut authorities = vec![authority.clone()];
        if !authority.contains(':') {
            authorities.push(format!("{authority}:443"));
        }
        let canonical_host =
            HeaderValue::from_str(&authority).expect("an origin's authority is a header value");
        let sentinel = format!(
            "{BEARER}{ACCESS_PREFIX}refused-{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        Self {
            authorities,
            canonical_host,
            live_grant,
            refused_credential: HeaderValue::from_str(&sentinel).expect("ascii"),
        }
    }

    /// Whether `parts` may go on, rewriting them for the router if so. Cheap and free of IO: it
    /// runs before a request is allowed to cost anything. The credential check that follows
    /// (`scrub_credentials`) reads the database and so runs after the request holds a slot.
    pub fn admit(&self, parts: &mut Parts) -> bool {
        if !is_connector_path(parts.uri.path()) || !self.host_is_ours(parts) {
            return false;
        }
        parts.headers.insert(HOST, self.canonical_host.clone());
        for name in STRIPPED_HEADERS {
            parts.headers.remove(HeaderName::from_static(name));
        }
        parts.headers.insert(
            HeaderName::from_static("forwarded"),
            HeaderValue::from_static(FORWARDED_VALUE),
        );
        true
    }

    /// `Host` and, for HTTP/2 and absolute-form requests, the URI's authority must each be this
    /// door's name, and a request with two `Host` headers is not one a router should guess at.
    /// Without this a client could send the private consent origin's name through the forwarder
    /// and reach the handler that answers only there.
    fn host_is_ours(&self, parts: &Parts) -> bool {
        let mut hosts = parts.headers.get_all(HOST).iter();
        let header = hosts.next();
        if hosts.next().is_some() {
            return false;
        }
        let from_header = match header {
            Some(v) => match v.to_str() {
                Ok(s) => Some(s.trim().to_ascii_lowercase()),
                Err(_) => return false,
            },
            None => None,
        };
        let from_uri = parts
            .uri
            .authority()
            .map(|a| a.as_str().to_ascii_lowercase());
        let ours = |s: &String| self.authorities.contains(s);
        match (&from_header, &from_uri) {
            (None, None) => false,
            (Some(h), None) => ours(h),
            (None, Some(a)) => ours(a),
            (Some(h), Some(a)) => ours(h) && ours(a),
        }
    }

    /// Only a live connector grant's access token goes through. Anything else a client
    /// presents, the owner's web token, a device token, an API key or an agent's session token,
    /// belongs to another door and is replaced by a credential that matches nothing.
    pub(crate) fn scrub_credentials(&self, parts: &mut Parts) {
        let api_key = parts.headers.remove(API_KEY_HEADER).is_some();
        let values: Vec<HeaderValue> = parts
            .headers
            .get_all(AUTHORIZATION)
            .iter()
            .cloned()
            .collect();
        // Whatever arrived, at most one value leaves, so nothing downstream can read another.
        parts.headers.remove(AUTHORIZATION);
        let foreign = api_key || values.len() > 1;
        let token = values.first().and_then(|v| {
            v.to_str()
                .ok()
                .and_then(|v| v.strip_prefix(BEARER))
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(|t| (v.clone(), t.to_string()))
        });
        match token {
            Some((value, t)) if !foreign && (self.live_grant)(&t) => {
                parts.headers.insert(AUTHORIZATION, value);
            }
            Some(_) => {
                parts
                    .headers
                    .insert(AUTHORIZATION, self.refused_credential.clone());
            }
            None if foreign => {
                parts
                    .headers
                    .insert(AUTHORIZATION, self.refused_credential.clone());
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRANT_TOKEN: &str = "hqo_live-grant-token";

    fn gate() -> Gate {
        Gate::new("hq.example.com", Arc::new(|t: &str| t == GRANT_TOKEN))
    }

    fn parts(uri: &str, headers: &[(&str, &str)]) -> Parts {
        let mut builder = Request::builder().method("POST").uri(uri);
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        builder.body(()).unwrap().into_parts().0
    }

    /// The request as the router would get it: admitted, then its credentials scrubbed.
    fn admitted(gate: &Gate, uri: &str, headers: &[(&str, &str)]) -> Option<Parts> {
        let mut p = parts(uri, headers);
        if !gate.admit(&mut p) {
            return None;
        }
        gate.scrub_credentials(&mut p);
        Some(p)
    }

    fn header<'a>(p: &'a Parts, name: &str) -> Option<&'a str> {
        p.headers.get(name).map(|v| v.to_str().unwrap())
    }

    #[test]
    fn only_the_connectors_own_paths_pass_and_only_spelled_exactly() {
        let g = gate();
        for path in CONNECTOR_PATHS {
            assert!(
                admitted(&g, path, &[("host", "hq.example.com")]).is_some(),
                "{path}"
            );
        }
        for path in [
            "/",
            "/api/tree",
            "/ws",
            "/health",
            "/connect",
            "/api/oauth/requests/abc",
            "/mcp/",
            "//mcp",
            "/%6Dcp",
            "/MCP",
            "/mcp/extra",
            "/oauth/",
            "/oauth/token/",
            "/.well-known/",
            "/.well-known/oauth-protected-resource/",
            "/.well-known/../api/tree",
            "/.well-known/oauth-protected-resource/../../api/tree",
            "/oauth/authorize",
            "/oauth/authorize?response_type=code",
            "/oauth/authorize/../../api/tree",
            "/assets/index.js",
        ] {
            assert!(
                admitted(&g, path, &[("host", "hq.example.com")]).is_none(),
                "{path}"
            );
        }
    }

    #[test]
    fn a_query_does_not_change_which_path_it_is() {
        let g = gate();
        assert!(admitted(&g, "/mcp?x=/api/tree", &[("host", "hq.example.com")]).is_some());
        assert!(admitted(&g, "/api/tree?next=/mcp", &[("host", "hq.example.com")]).is_none());
    }

    #[test]
    fn the_host_must_be_this_doors_name() {
        let g = gate();
        for ok in ["hq.example.com", "HQ.Example.COM", "hq.example.com:443"] {
            assert!(admitted(&g, "/mcp", &[("host", ok)]).is_some(), "{ok}");
        }
        for bad in [
            "hq.example.ts.net",
            "evil.example.com",
            "hq.example.com.evil.net",
            "hq.example.com:8443",
            "hq.example.com ",
            "user@hq.example.com",
            "",
        ] {
            // A padded value is trimmed by the parser, so only the truly different ones refuse.
            let allowed = bad.trim() == "hq.example.com";
            assert_eq!(
                admitted(&g, "/mcp", &[("host", bad)]).is_some(),
                allowed,
                "{bad:?}"
            );
        }
        assert!(admitted(&g, "/mcp", &[]).is_none(), "no Host at all");
    }

    #[test]
    fn two_host_headers_or_a_disagreeing_authority_are_refused() {
        let g = gate();
        assert!(
            admitted(
                &g,
                "/mcp",
                &[("host", "hq.example.com"), ("host", "hq.example.ts.net")]
            )
            .is_none()
        );
        assert!(
            admitted(
                &g,
                "https://hq.example.ts.net/mcp",
                &[("host", "hq.example.com")]
            )
            .is_none()
        );
        assert!(
            admitted(
                &g,
                "https://hq.example.com/mcp",
                &[("host", "hq.example.ts.net")]
            )
            .is_none()
        );
    }

    #[test]
    fn an_http2_request_with_only_an_authority_gets_the_host_the_handlers_read() {
        let g = gate();
        let p = admitted(&g, "https://hq.example.com/oauth/token", &[]).unwrap();
        assert_eq!(header(&p, "host"), Some("hq.example.com"));
        assert!(admitted(&g, "https://hq.example.ts.net/oauth/token", &[]).is_none());
    }

    #[test]
    fn a_door_with_a_port_wants_that_port() {
        let g = Gate::new("hq.example.com:8443", Arc::new(|_| false));
        assert!(admitted(&g, "/mcp", &[("host", "hq.example.com:8443")]).is_some());
        assert!(admitted(&g, "/mcp", &[("host", "hq.example.com")]).is_none());
        assert!(admitted(&g, "/mcp", &[("host", "hq.example.com:443")]).is_none());
    }

    #[test]
    fn every_request_is_marked_as_coming_through_a_proxy_and_a_forged_mark_is_replaced() {
        let g = gate();
        let p = admitted(
            &g,
            "/mcp",
            &[
                ("host", "hq.example.com"),
                ("forwarded", "for=127.0.0.1"),
                ("x-forwarded-for", "127.0.0.1"),
                ("x-real-ip", "127.0.0.1"),
                ("tailscale-user-login", "owner@example.com"),
                ("x-hq-session-id", "sess"),
            ],
        )
        .unwrap();
        assert_eq!(header(&p, "forwarded"), Some(FORWARDED_VALUE));
        for gone in [
            "x-forwarded-for",
            "x-real-ip",
            "tailscale-user-login",
            "x-hq-session-id",
        ] {
            assert!(p.headers.get(gone).is_none(), "{gone}");
        }
        // The property mcp_http relies on, asked of the real functions: with the development
        // switch on and no key configured, this request still resolves to nobody.
        let dev_open = crate::auth::mcp_dev_open(&p.headers, true, true);
        assert!(!dev_open);
        assert_eq!(
            crate::auth::resolve_identity(&p.headers, None, None, None, None, dev_open),
            None
        );
    }

    #[test]
    fn no_configured_api_key_gets_a_caller_an_identity_through_the_gate() {
        let g = gate();
        let host = ("host", "hq.example.com");
        let grant = format!("Bearer {GRANT_TOKEN}");
        let resolve = |p: &Parts| {
            crate::auth::resolve_identity(
                &p.headers,
                Some("full-key"),
                Some("spark-key"),
                Some("handoff-key"),
                Some("tasks-key"),
                false,
            )
        };
        for key in ["full-key", "spark-key", "handoff-key", "tasks-key"] {
            let bearer = format!("Bearer {key}");
            // A key in `x-api-key`, in `Authorization`, in both, and behind a good grant token.
            for headers in [
                vec![host, ("x-api-key", key)],
                vec![host, ("authorization", bearer.as_str())],
                vec![host, ("x-api-key", key), ("authorization", bearer.as_str())],
                vec![
                    host,
                    ("authorization", grant.as_str()),
                    ("authorization", bearer.as_str()),
                ],
                vec![
                    host,
                    ("authorization", bearer.as_str()),
                    ("authorization", grant.as_str()),
                ],
            ] {
                let p = admitted(&g, "/mcp", &headers).unwrap();
                assert_eq!(resolve(&p), None, "{key} via {headers:?}");
                assert!(p.headers.get_all("authorization").iter().count() <= 1);
            }
        }
        // Control: the same resolver does accept those keys when they arrive untouched.
        let raw = parts("/mcp", &[host, ("x-api-key", "full-key")]);
        assert_eq!(
            crate::auth::resolve_identity(&raw.headers, Some("full-key"), None, None, None, false),
            Some(crate::auth::ApiIdentity::Full)
        );
    }

    #[test]
    fn two_authorization_headers_leave_as_one_that_is_not_the_good_one() {
        let g = gate();
        let good = format!("Bearer {GRANT_TOKEN}");
        let p = admitted(
            &g,
            "/mcp",
            &[
                ("host", "hq.example.com"),
                ("authorization", good.as_str()),
                ("authorization", "Bearer something-else"),
            ],
        )
        .unwrap();
        let all: Vec<_> = p.headers.get_all("authorization").iter().collect();
        assert_eq!(all.len(), 1);
        assert_ne!(all[0].to_str().unwrap(), good);
    }

    #[test]
    fn a_live_grant_token_goes_through_and_nothing_else_does() {
        let g = gate();
        let bearer = |t: &str| format!("Bearer {t}");
        let host = ("host", "hq.example.com");

        let ok = bearer(GRANT_TOKEN);
        let p = admitted(&g, "/mcp", &[host, ("authorization", &ok)]).unwrap();
        assert_eq!(header(&p, "authorization"), Some(ok.as_str()));

        // A key, a web token, a device token: none of them is a live grant.
        for foreign in [
            "full-key",
            "web-token",
            "hqo_expired-or-unknown",
            "hqr_refresh",
        ] {
            let v = bearer(foreign);
            let p = admitted(&g, "/mcp", &[host, ("authorization", &v)]).unwrap();
            let got = header(&p, "authorization").unwrap();
            assert_ne!(got, v, "{foreign}");
            assert!(
                got.starts_with("Bearer hqo_"),
                "looks like a token so the client refreshes"
            );
            assert!(!(g.live_grant)(got.trim_start_matches("Bearer ")));
        }
    }

    #[test]
    fn an_api_key_header_never_arrives_even_beside_a_good_token() {
        let g = gate();
        let ok = format!("Bearer {GRANT_TOKEN}");
        let host = ("host", "hq.example.com");

        let alone = admitted(&g, "/mcp", &[host, ("x-api-key", "full-key")]).unwrap();
        assert!(alone.headers.get("x-api-key").is_none());
        assert!(
            header(&alone, "authorization")
                .unwrap()
                .starts_with("Bearer hqo_refused-")
        );

        let both = admitted(
            &g,
            "/mcp",
            &[host, ("x-api-key", "full-key"), ("authorization", &ok)],
        )
        .unwrap();
        assert!(both.headers.get("x-api-key").is_none());
        assert_ne!(header(&both, "authorization"), Some(ok.as_str()));
    }

    #[test]
    fn a_non_bearer_authorization_is_dropped_and_no_credential_stays_none() {
        let g = gate();
        let host = ("host", "hq.example.com");
        let basic = admitted(&g, "/mcp", &[host, ("authorization", "Basic abc")]).unwrap();
        assert!(basic.headers.get("authorization").is_none());
        let empty = admitted(&g, "/mcp", &[host, ("authorization", "Bearer ")]).unwrap();
        assert!(empty.headers.get("authorization").is_none());
        let none = admitted(&g, "/mcp", &[host]).unwrap();
        assert!(none.headers.get("authorization").is_none());
    }

    #[test]
    fn the_refusing_credential_is_random_per_gate() {
        let a = gate();
        let b = gate();
        assert_ne!(a.refused_credential, b.refused_credential);
    }

    #[test]
    fn connector_territory_covers_every_spelling_the_own_devices_door_must_refuse() {
        for path in [
            "/mcp",
            "/mcp/",
            "/mcp/x",
            "/oauth",
            "/oauth/token",
            "/oauth/authorize",
            "/.well-known",
            "/.well-known/oauth-protected-resource",
            "/.well-known/anything",
        ] {
            assert!(is_connector_territory(path), "{path}");
        }
        for path in [
            "/",
            "/api/tree",
            "/ws",
            "/health",
            "/mcpx",
            "/oauthx",
            "/connect",
            "/api/oauth/connector",
        ] {
            assert!(!is_connector_territory(path), "{path}");
        }
    }
}
