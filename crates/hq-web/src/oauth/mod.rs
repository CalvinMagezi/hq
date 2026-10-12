//! HQ as an OAuth-protected MCP resource for claude.ai custom connectors.
//! Plan and threat model: `docs/plans/claude-connector.md`.

use crate::WsState;
use axum::Json;
use axum::extract::State;
use axum::http::HeaderValue;
use hq_core::config::ConnectorOrigins;
use serde_json::{Value, json};
use std::sync::Arc;

/// Where Claude reads the protected resource metadata (RFC 9728).
pub(crate) const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";
/// The path-suffixed form, which clients try first for a resource at `/mcp`.
pub(crate) const PROTECTED_RESOURCE_MCP_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
/// The one scope advertised. Claude requests every advertised scope, so the grant's real
/// scope (full, read-only or tasks) is chosen on the consent page instead.
pub(crate) const SCOPE: &str = "hq";

/// Registers the connector's routes when it is on; otherwise they stay unknown paths.
pub(crate) fn routes(
    router: axum::Router<Arc<WsState>>,
    state: &WsState,
) -> axum::Router<Arc<WsState>> {
    if state.connector.is_none() {
        return router;
    }
    router
        .route(
            PROTECTED_RESOURCE_PATH,
            axum::routing::get(protected_resource),
        )
        .route(
            PROTECTED_RESOURCE_MCP_PATH,
            axum::routing::get(protected_resource),
        )
}

async fn protected_resource(State(state): State<Arc<WsState>>) -> Json<Value> {
    let origins = state
        .connector
        .as_ref()
        .expect("route registered only with a connector");
    Json(resource_metadata(origins))
}

fn resource_metadata(origins: &ConnectorOrigins) -> Value {
    json!({
        "resource": origins.resource(),
        "authorization_servers": [origins.public],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "Agent HQ",
    })
}

/// The `WWW-Authenticate` value for a refused `/mcp` call. With the connector on it points
/// Claude at the metadata; `invalid_token` tells a client its credential was presented and
/// refused (an expired access token), so it refreshes instead of starting over.
pub(crate) fn www_authenticate(
    origins: Option<&ConnectorOrigins>,
    secret_presented: bool,
) -> HeaderValue {
    let mut params = vec![r#"realm="agent-hq""#.to_string()];
    if secret_presented {
        params.push(r#"error="invalid_token""#.into());
    }
    if let Some(o) = origins {
        params.push(format!(
            r#"resource_metadata="{}{PROTECTED_RESOURCE_PATH}""#,
            o.public
        ));
    }
    HeaderValue::from_str(&format!("Bearer {}", params.join(", ")))
        .unwrap_or_else(|_| HeaderValue::from_static("Bearer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origins() -> ConnectorOrigins {
        ConnectorOrigins {
            public: "https://hq.example.com".into(),
            consent: "https://hq.example.ts.net".into(),
        }
    }

    #[test]
    fn metadata_names_the_exact_resource_and_this_server_as_issuer() {
        let m = resource_metadata(&origins());
        assert_eq!(m["resource"], "https://hq.example.com/mcp");
        assert_eq!(
            m["authorization_servers"],
            json!(["https://hq.example.com"])
        );
        assert_eq!(m["scopes_supported"], json!([SCOPE]));
    }

    async fn get(router: axum::Router, path: &str) -> axum::response::Response {
        use tower::ServiceExt;
        let req = axum::http::Request::get(path)
            .body(axum::body::Body::empty())
            .unwrap();
        router.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn metadata_is_served_only_with_the_connector_on() {
        let vault = tempfile::TempDir::new().unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        let off = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let router = routes(axum::Router::new(), &off).with_state(off);
        let res = get(router, PROTECTED_RESOURCE_PATH).await;
        assert_eq!(res.status(), axum::http::StatusCode::NOT_FOUND);

        state.connector = Some(origins());
        let on = Arc::new(state);
        for path in [PROTECTED_RESOURCE_PATH, PROTECTED_RESOURCE_MCP_PATH] {
            let router = routes(axum::Router::new(), &on).with_state(on.clone());
            let res = get(router, path).await;
            assert_eq!(res.status(), axum::http::StatusCode::OK, "{path}");
            let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["resource"], "https://hq.example.com/mcp");
        }
    }

    #[test]
    fn configuring_turns_the_connector_on_only_when_valid_and_never_under_lite() {
        let vault = tempfile::TempDir::new().unwrap();
        let cfg = hq_core::config::McpConnectorConfig {
            enabled: true,
            public_url: Some("https://hq.example.com".into()),
            authorize_url: None,
        };
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        assert!(
            state
                .configure_connector(&cfg)
                .unwrap_err()
                .contains("web_auth_token")
        );
        assert!(state.connector.is_none());
        state.web_auth_token = Some("owner-token".into());
        state.configure_connector(&cfg).unwrap();
        assert_eq!(
            state.connector.as_ref().unwrap().public,
            "https://hq.example.com"
        );

        let mut lite = WsState::new(vault.path().to_path_buf(), None)
            .with_profile(hq_core::config::Profile::Lite);
        lite.web_auth_token = Some("owner-token".into());
        assert!(lite.configure_connector(&cfg).unwrap_err().contains("lite"));
        assert!(lite.connector.is_none());
    }

    #[test]
    fn the_challenge_points_at_metadata_only_when_the_connector_is_on() {
        let on = www_authenticate(Some(&origins()), false);
        assert_eq!(
            on.to_str().unwrap(),
            r#"Bearer realm="agent-hq", resource_metadata="https://hq.example.com/.well-known/oauth-protected-resource""#
        );
        let off = www_authenticate(None, true);
        assert_eq!(
            off.to_str().unwrap(),
            r#"Bearer realm="agent-hq", error="invalid_token""#
        );
    }
}
