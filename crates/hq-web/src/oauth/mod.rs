//! HQ as an OAuth-protected MCP resource for claude.ai custom connectors.
//! Plan and threat model: `docs/plans/claude-connector.md`.

mod authorize;
mod cimd;
mod consent;
pub(crate) mod gemini;
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
/// The Gemini issuer's metadata: RFC 8414's path form for an issuer at `/gemini`.
pub(crate) const GEMINI_AUTHORIZATION_SERVER_PATH: &str =
    "/.well-known/oauth-authorization-server/gemini";
/// The Gemini resource's metadata (RFC 9728, path-suffixed for `/mcp/gemini/v1`).
pub(crate) const GEMINI_PROTECTED_RESOURCE_PATH: &str =
    "/.well-known/oauth-protected-resource/mcp/gemini/v1";
/// Advertised because Google's discovery requires a registration endpoint; it registers nothing.
pub(crate) const REGISTRATION_PATH: &str = "/oauth/register";

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
    let router = if state.connector.as_ref().is_some_and(|c| c.gemini) {
        router
            .route(
                GEMINI_AUTHORIZATION_SERVER_PATH,
                get(gemini_authorization_server),
            )
            .route(
                GEMINI_PROTECTED_RESOURCE_PATH,
                get(gemini_protected_resource),
            )
            .route(REGISTRATION_PATH, post(refuse_registration))
            .route(hq_core::config::GEMINI_MCP_PATH, post(gemini::mcp_handler))
    } else {
        router
    };
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

async fn gemini_authorization_server(State(state): State<Arc<WsState>>) -> Json<Value> {
    let settings = state
        .connector
        .as_ref()
        .expect("route registered only with a connector");
    Json(gemini_server_metadata(settings))
}

/// Gemini's own issuer, so nothing in Claude's document changes. Same endpoints: they tell the
/// profiles apart by client id, and a Gemini client must authenticate with its secret.
fn gemini_server_metadata(s: &ConnectorSettings) -> Value {
    json!({
        "issuer": s.gemini_issuer(),
        "authorization_endpoint": format!("{}/oauth/authorize", s.consent),
        "token_endpoint": format!("{}/oauth/token", s.public),
        "revocation_endpoint": format!("{}/oauth/revoke", s.public),
        "registration_endpoint": format!("{}{REGISTRATION_PATH}", s.public),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "revocation_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "scopes_supported": [SCOPE],
    })
}

async fn gemini_protected_resource(State(state): State<Arc<WsState>>) -> Json<Value> {
    let s = state
        .connector
        .as_ref()
        .expect("route registered only with a connector");
    Json(json!({
        "resource": s.gemini_resource(),
        "authorization_servers": [s.gemini_issuer()],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "Agent HQ (read-only)",
    }))
}

/// RFC 7591's refusal: clients are created by the owner with `hq connector client add`.
async fn refuse_registration() -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(json!({
            "error": "invalid_client_metadata",
            "error_description": "this server does not register clients; enter the client id \
                                  and secret the owner created",
        })),
    )
        .into_response()
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
    let metadata = origins.map(|o| format!("{}{PROTECTED_RESOURCE_PATH}", o.public));
    challenge(metadata.as_deref(), secret_presented)
}

/// The challenge for a refused call to the Gemini resource, pointing at its own metadata.
pub(crate) fn gemini_www_authenticate(
    origins: &ConnectorSettings,
    secret_presented: bool,
) -> HeaderValue {
    let metadata = format!("{}{GEMINI_PROTECTED_RESOURCE_PATH}", origins.public);
    challenge(Some(&metadata), secret_presented)
}

fn challenge(resource_metadata: Option<&str>, secret_presented: bool) -> HeaderValue {
    let mut params = vec![r#"realm="agent-hq""#.to_string()];
    if secret_presented {
        params.push(r#"error="invalid_token""#.into());
    }
    if let Some(url) = resource_metadata {
        params.push(format!(r#"resource_metadata="{url}""#));
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
            gemini: false,
        }
    }

    /// Claude's documents, byte for byte: the Gemini profile must never change them.
    #[test]
    fn claude_metadata_documents_are_pinned() {
        let mut gemini_on = origins();
        gemini_on.gemini = true;
        for s in [origins(), gemini_on] {
            assert_eq!(
                server_metadata(&s),
                json!({
                    "issuer": "https://hq.example.com",
                    "authorization_endpoint": "https://hq.example.ts.net/oauth/authorize",
                    "token_endpoint": "https://hq.example.com/oauth/token",
                    "revocation_endpoint": "https://hq.example.com/oauth/revoke",
                    "response_types_supported": ["code"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                    "code_challenge_methods_supported": ["S256"],
                    "token_endpoint_auth_methods_supported": ["none"],
                    "revocation_endpoint_auth_methods_supported": ["none"],
                    "client_id_metadata_document_supported": true,
                    "scopes_supported": ["hq", "offline_access"],
                })
            );
            assert_eq!(
                resource_metadata(&s),
                json!({
                    "resource": "https://hq.example.com/mcp",
                    "authorization_servers": ["https://hq.example.com"],
                    "scopes_supported": ["hq"],
                    "bearer_methods_supported": ["header"],
                    "resource_name": "Agent HQ",
                })
            );
        }
    }

    #[test]
    fn gemini_metadata_has_its_own_issuer_secret_auth_and_a_registration_endpoint() {
        let m = gemini_server_metadata(&origins());
        assert_eq!(m["issuer"], "https://hq.example.com/gemini");
        assert_eq!(m["token_endpoint"], "https://hq.example.com/oauth/token");
        assert_eq!(
            m["registration_endpoint"],
            "https://hq.example.com/oauth/register"
        );
        assert_eq!(
            m["token_endpoint_auth_methods_supported"],
            json!(["client_secret_basic", "client_secret_post"])
        );
        assert_eq!(m["response_types_supported"], json!(["code"]));
        assert_eq!(m["code_challenge_methods_supported"], json!(["S256"]));
        assert!(m.get("client_id_metadata_document_supported").is_none());
        assert_eq!(
            gemini_www_authenticate(&origins(), false).to_str().unwrap(),
            r#"Bearer realm="agent-hq", resource_metadata="https://hq.example.com/.well-known/oauth-protected-resource/mcp/gemini/v1""#
        );
    }

    #[tokio::test]
    async fn gemini_routes_exist_only_with_the_profile_on() {
        let vault = tempfile::TempDir::new().unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        state.connector = Some(origins());
        let off = Arc::new(state);
        let paths = [
            GEMINI_AUTHORIZATION_SERVER_PATH,
            GEMINI_PROTECTED_RESOURCE_PATH,
        ];
        for path in paths {
            let router = routes(axum::Router::new(), &off).with_state(off.clone());
            assert_eq!(
                get(router, path).await.status(),
                axum::http::StatusCode::NOT_FOUND,
                "{path}"
            );
        }
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        let mut s = origins();
        s.gemini = true;
        state.connector = Some(s);
        let on = Arc::new(state);
        for path in paths {
            let router = routes(axum::Router::new(), &on).with_state(on.clone());
            let res = get(router, path).await;
            assert_eq!(res.status(), axum::http::StatusCode::OK, "{path}");
        }
        let router = routes(axum::Router::new(), &on).with_state(on.clone());
        let res = get(router, hq_core::config::GEMINI_MCP_PATH).await;
        assert_eq!(
            res.status(),
            axum::http::StatusCode::METHOD_NOT_ALLOWED,
            "GET has no stream"
        );
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
            ..Default::default()
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
