//! The paired-devices door (`bridge.web_listen`): a second loopback listener that the bridge's
//! `web` service points at, so the owner's paired devices reach the web app, `/api`, `/ws` and
//! `/mcp` by key over iroh. Everything arriving here looks like loopback, so it opens only when
//! the web token is on and the `/mcp` development switch is off. The connector's OAuth paths never
//! answer here, and `/mcp` takes a configured API key, a paired device's own key or a launched
//! agent's session token, never a connector grant, so the two doors share no credential (`docs/security/PAIRING.md`, section 14).

use crate::WsState;
use crate::mcp_http::McpKeys;
use crate::tls::{STRIPPED_HEADERS, not_found};
use axum::{Router, body::Body, extract::Request, middleware::Next};
use std::{net::SocketAddr, sync::Arc};

/// Marks every request as forwarded, so the `/mcp` development switch can never apply here even
/// if it is turned on after the door opened.
const FORWARDED_VALUE: &str = "by=hq-paired-devices";

/// Wraps `router` for the paired-devices door: the connector's OAuth and discovery paths are not
/// found, and `/mcp` needs one of `keys` or a session token before any handler runs.
pub(crate) fn guard(router: Router, keys: McpKeys, state: Arc<WsState>) -> Router {
    router.layer(axum::middleware::from_fn(
        move |mut req: Request<Body>, next: Next| {
            let keys = keys.clone();
            let state = state.clone();
            async move {
                if !allowed(req.uri().path(), req.headers(), &keys, &state) {
                    return not_found();
                }
                // A paired device chooses its own headers: none that claim a proxy's or a
                // tunnel's word is believed.
                let headers = req.headers_mut();
                for name in STRIPPED_HEADERS {
                    headers.remove(name);
                }
                headers.insert(
                    axum::http::header::FORWARDED,
                    axum::http::HeaderValue::from_static(FORWARDED_VALUE),
                );
                next.run(req).await
            }
        },
    ))
}

fn allowed(path: &str, headers: &axum::http::HeaderMap, keys: &McpKeys, state: &WsState) -> bool {
    let under = |prefix: &str| path == prefix || path.starts_with(&format!("{prefix}/"));
    if under("/oauth") || under("/.well-known") {
        return false;
    }
    if !under("/mcp") {
        return true;
    }
    // Agents launched on a paired machine call back with their own session token
    // (`agent_host.agent_mcp_url`), and a paired machine with the key it got at setup; a
    // connector grant is never either.
    let secret = crate::auth::presented_secret(headers);
    path == "/mcp"
        && (keys.presented(headers)
            || crate::mcp_http::device_key_for_secret(state, secret)
            || crate::mcp_http::session_for_secret(state, secret).is_some())
}

/// Why the door stays shut, if it must.
/// The configured web token is required, not only a device token: revoking the last device token
/// turns sign-in off, which would open `/api` and `/ws` to every paired device.
fn refusal(state: &WsState, dev_switch: bool) -> Option<&'static str> {
    if state.profile().is_lite() {
        return Some("not available under profile: lite");
    }
    if state.configured_web_token().is_none() {
        return Some("set web_auth_token first: paired devices arrive as loopback");
    }
    dev_switch.then_some("HQ_MCP_DEV_NO_AUTH is on, which would open /mcp to every paired device")
}

/// Opens the door when `bridge.web_listen` is set. Called once from `create_router` with the
/// router the main listener serves. A refusal is logged and the rest of the server is unchanged.
pub(crate) fn spawn(state: &Arc<WsState>, router: Router) {
    let listen = match state.hq_config.as_ref().map(|c| c.bridge.web_listen()) {
        Some(Ok(Some(addr))) => addr,
        Some(Err(e)) => return refuse(&e),
        _ => return,
    };
    let state = state.clone();
    tokio::spawn(async move {
        let dev_switch = std::env::var(crate::auth::MCP_DEV_NO_AUTH_ENV).is_ok_and(|v| v == "1");
        if let Some(why) = refusal(&state, dev_switch) {
            return refuse(why);
        }
        let app = guard(router, McpKeys::from_env(), state.clone());
        serve(listen, app).await;
    });
}

async fn serve(listen: SocketAddr, app: Router) {
    match tokio::net::TcpListener::bind(listen).await {
        Ok(listener) => {
            tracing::info!(%listen, "paired-devices door: listening");
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!("paired-devices door stopped: {e}");
            }
        }
        Err(e) => refuse(&format!("could not listen on {listen}: {e}")),
    }
}

fn refuse(why: &str) {
    tracing::error!("paired-devices door stays shut: {why}");
    eprintln!("paired-devices door not opened: {why}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::tests::support::{WEB_TOKEN, mint_grant, world};

    const KEY: &str = "owner-mcp-key";
    const TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

    async fn door(router: Router, state: Arc<WsState>) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = guard(router, McpKeys::with_full(KEY), state);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        addr
    }

    async fn call(addr: SocketAddr, method: &str, path: &str, auth: &[(&str, &str)]) -> u16 {
        let mut req = reqwest::Client::new()
            .request(method.parse().unwrap(), format!("http://{addr}{path}"))
            .header("content-type", "application/json")
            .body(if method == "POST" { TOOLS_LIST } else { "" });
        for (k, v) in auth {
            req = req.header(*k, *v);
        }
        req.send().await.unwrap().status().as_u16()
    }

    /// Mutation: `allowed` returns true for everything. The OAuth loop then sees 200s and the
    /// connector's grant reaches `/mcp`.
    #[tokio::test]
    async fn serves_the_owners_api_and_keyed_mcp_and_nothing_of_the_connectors() {
        let w = world();
        let addr = door(w.router.clone(), w.state.clone()).await;
        let bearer = format!("Bearer {WEB_TOKEN}");
        assert_eq!(
            call(addr, "GET", "/api/tree", &[("authorization", &bearer)]).await,
            200
        );
        assert_eq!(call(addr, "GET", "/api/tree", &[]).await, 401);
        // Past the door; the handler then checks the key against its own environment (unset in
        // tests, hence 401 rather than 404).
        assert_eq!(call(addr, "POST", "/mcp", &[("x-api-key", KEY)]).await, 401);

        let grant = mint_grant(&w.state);
        let session = w
            .state
            .db
            .with_conn(|c| hq_db::session_tokens::mint(c, "sess-1"))
            .unwrap();
        // A launched agent's session token reaches the handler, which serves it.
        let agent = format!("Bearer {session}");
        assert_eq!(
            call(addr, "POST", "/mcp", &[("authorization", &agent)]).await,
            200
        );
        for secret in [grant.access_token.as_str(), WEB_TOKEN, "not-a-token"] {
            let bearer = format!("Bearer {secret}");
            assert_eq!(
                call(addr, "POST", "/mcp", &[("authorization", &bearer)]).await,
                404,
                "a credential that is neither an API key nor a session token reached /mcp"
            );
        }
        w.state
            .db
            .with_conn(|c| hq_db::session_tokens::revoke(c, "sess-1"))
            .unwrap();
        assert_eq!(
            call(addr, "POST", "/mcp", &[("authorization", &agent)]).await,
            404,
            "a revoked session token reached /mcp"
        );
        for path in [
            "/oauth/authorize",
            "/oauth/token",
            "/oauth/revoke",
            "/.well-known/oauth-authorization-server",
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-protected-resource/mcp",
            "/mcp/extra",
        ] {
            for method in ["GET", "POST"] {
                assert_eq!(
                    call(addr, method, path, &[("x-api-key", KEY)]).await,
                    404,
                    "{method} {path}"
                );
            }
        }
    }

    #[tokio::test]
    async fn stays_shut_without_sign_in_or_with_the_dev_switch() {
        let w = world();
        assert!(refusal(&w.state, true).is_some());
        assert!(refusal(&w.state, false).is_none());
        let lite = crate::tls::tests::support::world_with(hq_core::config::Profile::Lite);
        assert!(refusal(&lite.state, false).is_some());
        // A device token alone does not count: the configured token must be set.
        let vault = tempfile::TempDir::new().unwrap();
        let no_token = WsState::new(vault.path().to_path_buf(), None);
        hq_db::web_auth::mint(&no_token.db, "phone", 0, None).unwrap();
        assert!(refusal(&no_token, false).is_some());
    }

    /// Mutation: `allowed` drops the device key check. The live key then gets 404 here.
    #[tokio::test]
    async fn a_paired_devices_key_opens_mcp_until_the_device_is_revoked() {
        let w = world();
        let addr = door(w.router.clone(), w.state.clone()).await;
        let key = crate::mcp_http::paired_device_key(&w.state.db, "aa11");
        let bearer = format!("Bearer {key}");
        assert_eq!(
            call(addr, "POST", "/mcp", &[("x-api-key", &key)]).await,
            200
        );
        assert_eq!(
            call(addr, "POST", "/mcp", &[("authorization", &bearer)]).await,
            200
        );
        assert_eq!(
            call(addr, "GET", "/mcp/extra", &[("x-api-key", &key)]).await,
            404
        );
        hq_db::bridge::revoke(&w.state.db, "aa11").unwrap();
        assert_eq!(
            call(addr, "POST", "/mcp", &[("x-api-key", &key)]).await,
            404
        );
    }
}
