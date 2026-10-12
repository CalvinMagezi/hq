//! HQ as an OAuth-protected MCP resource for claude.ai custom connectors.
//! Plan and threat model: `docs/plans/claude-connector.md`.

mod authorize;
mod cimd;
mod consent;
pub(crate) mod guard;
mod token;

use crate::WsState;
use axum::Json;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::routing::{get, post};
use hq_core::config::ConnectorSettings;
use hq_db::mcp_connector::GrantScope;
use serde_json::{Value, json};
use std::sync::Arc;

/// Where Claude reads the protected resource metadata (RFC 9728).
pub(crate) const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";
/// The path-suffixed form, which clients try first for a resource at `/mcp`.
pub(crate) const PROTECTED_RESOURCE_MCP_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
/// The one scope advertised. Claude requests every advertised scope, so the grant's real
/// scope (full, read-only or tasks) is chosen on the consent page instead.
pub(crate) const SCOPE: &str = "hq";
/// Where Claude reads the authorization server metadata (RFC 8414).
pub(crate) const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";

/// Unix seconds, the clock every OAuth record is kept in.
pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Registers the connector's routes. Settings can always ask whether it is on; everything
/// else exists only while it is, so a disabled connector answers nothing.
pub(crate) fn routes(
    router: axum::Router<Arc<WsState>>,
    state: &WsState,
) -> axum::Router<Arc<WsState>> {
    let router = router.route("/api/oauth/connector", get(consent::status));
    if state.connector.is_none() {
        return router;
    }
    router
        .route(PROTECTED_RESOURCE_PATH, get(protected_resource))
        .route(PROTECTED_RESOURCE_MCP_PATH, get(protected_resource))
        .route(AUTHORIZATION_SERVER_PATH, get(authorization_server))
        .route("/oauth/authorize", get(authorize::authorize))
        .route("/oauth/token", post(token::token))
        .route("/oauth/revoke", post(token::revoke))
        .route("/api/oauth/connector/arm", post(consent::arm))
        .route("/api/oauth/connector/disarm", post(consent::disarm))
        .route("/api/oauth/connector/off", post(consent::off))
        .route("/api/oauth/grants/{id}/revoke", post(consent::revoke_grant))
        .route("/api/oauth/requests/{id}", get(consent::request))
        .route("/api/oauth/requests/{id}/approve", post(consent::approve))
        .route("/api/oauth/requests/{id}/deny", post(consent::deny))
}

impl WsState {
    /// The scope the consent page preselects.
    pub(crate) fn connector_default_scope(&self) -> GrantScope {
        self.connector
            .as_ref()
            .and_then(|c| GrantScope::parse(&c.default_scope))
            .unwrap_or(GrantScope::ReadOnly)
    }
}

async fn authorization_server(State(state): State<Arc<WsState>>) -> Json<Value> {
    let settings = state
        .connector
        .as_ref()
        .expect("route registered only with a connector");
    Json(server_metadata(settings))
}

/// No registration endpoint: clients identify themselves with a metadata document (CIMD).
fn server_metadata(s: &ConnectorSettings) -> Value {
    json!({
        "issuer": s.public,
        "authorization_endpoint": format!("{}/oauth/authorize", s.consent),
        "token_endpoint": format!("{}/oauth/token", s.public),
        "revocation_endpoint": format!("{}/oauth/revoke", s.public),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "client_id_metadata_document_supported": true,
        "scopes_supported": [SCOPE, "offline_access"],
    })
}

async fn protected_resource(State(state): State<Arc<WsState>>) -> Json<Value> {
    let origins = state
        .connector
        .as_ref()
        .expect("route registered only with a connector");
    Json(resource_metadata(origins))
}

fn resource_metadata(origins: &ConnectorSettings) -> Value {
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
    origins: Option<&ConnectorSettings>,
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
mod flow_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn origins() -> ConnectorSettings {
        ConnectorSettings {
            public: "https://hq.example.com".into(),
            consent: "https://hq.example.ts.net".into(),
            default_scope: "read_only".into(),
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
            default_scope: None,
        };
        // Pinned, not read from the machine: CI runs this under an ambient Lite profile.
        let mut state = WsState::new(vault.path().to_path_buf(), None)
            .with_profile(hq_core::config::Profile::Full);
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
