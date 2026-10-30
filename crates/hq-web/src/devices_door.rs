//! The paired-devices door (`bridge.web_listen`): a second loopback listener that the bridge's
//! `web` service points at, so the owner's paired devices reach the web app, `/api`, `/ws` and
//! `/mcp` by key over iroh. Everything arriving here looks like loopback, so it opens only when
//! the web token is on and the `/mcp` development switch is off. The connector's OAuth paths never
//! answer here, and `/mcp` takes a configured API key only: no connector grant and no session
//! token, so the two doors share no credential (`docs/security/PAIRING.md`, section 14).

use crate::WsState;
use crate::mcp_http::McpKeys;
use crate::tls::not_found;
use axum::{Router, body::Body, extract::Request, middleware::Next};
use std::{net::SocketAddr, sync::Arc};

/// Wraps `router` for the paired-devices door: the connector's OAuth and discovery paths are not
/// found, and `/mcp` needs one of `keys` before any handler runs.
pub(crate) fn guard(router: Router, keys: McpKeys) -> Router {
    router.layer(axum::middleware::from_fn(
        move |req: Request<Body>, next: Next| {
            let keys = keys.clone();
            async move {
                if !allowed(req.uri().path(), req.headers(), &keys) {
                    return not_found();
                }
                next.run(req).await
            }
        },
    ))
}

fn allowed(path: &str, headers: &axum::http::HeaderMap, keys: &McpKeys) -> bool {
    let under = |prefix: &str| path == prefix || path.starts_with(&format!("{prefix}/"));
    if under("/oauth") || under("/.well-known") {
        return false;
    }
    !under("/mcp") || (path == "/mcp" && keys.presented(headers))
}

/// Why the door stays shut, if it must.
fn refusal(state: &WsState, auth_on: bool, dev_switch: bool) -> Option<&'static str> {
    if state.profile().is_lite() {
        return Some("not available under profile: lite");
    }
    if !auth_on {
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
        let auth_on = state.web_auth_enabled_async().await;
        if let Some(why) = refusal(&state, auth_on, dev_switch) {
            return refuse(why);
        }
        serve(listen, guard(router, McpKeys::from_env())).await;
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

    async fn door(router: Router) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = guard(router, McpKeys::with_full(KEY));
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
    /// grant and session token reach `/mcp`.
    #[tokio::test]
    async fn serves_the_owners_api_and_keyed_mcp_and_nothing_of_the_connectors() {
        let w = world();
        let addr = door(w.router.clone()).await;
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
        for secret in [grant.access_token.as_str(), session.as_str(), WEB_TOKEN] {
            let bearer = format!("Bearer {secret}");
            assert_eq!(
                call(addr, "POST", "/mcp", &[("authorization", &bearer)]).await,
                404,
                "a credential that is not an API key reached /mcp"
            );
        }
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
        assert!(refusal(&w.state, false, false).is_some());
        assert!(refusal(&w.state, true, true).is_some());
        assert!(refusal(&w.state, true, false).is_none());
        let lite = crate::tls::tests::support::world_with(hq_core::config::Profile::Lite);
        assert!(refusal(&lite.state, true, false).is_some());
    }
}
