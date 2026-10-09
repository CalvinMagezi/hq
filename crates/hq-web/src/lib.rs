//! Embedded web UI server with API endpoints and WebSocket.

mod api;
pub use api::HEALTH_SERVICE;
mod chat_uploads;
mod error;
pub mod auth;
pub mod gmail_ingest;
mod mcp_http;
mod notifications_api;
mod notifications_watch;
mod origin;
mod security_headers;
mod session_driver;
mod sessions_api;
mod workbench_api;
mod copilot_usage_api;
mod openrouter_usage_api;
mod budgets_api;
mod usage_forecast_api;
mod usage_providers_api;
mod settings_api;
mod setup_api;
mod subagent_followup;
mod tasks_api;
mod tasks_watch;
mod threads_api;
mod vault_api;
mod ws;

use axum::{
    Router,
    routing::{get, post},
};
use hq_tools::registry::ToolRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

/// Install the rustls process-level CryptoProvider. Required because the
/// dependency tree enables BOTH `aws-lc-rs` and `ring`, so rustls 0.23 cannot
/// auto-select one and panics on first TLS use. Idempotent; call once at process
/// startup, before any TLS client is built.
pub fn install_default_crypto_provider() {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// Shared server state.
pub struct WsState {
    pub(crate) tx: broadcast::Sender<String>,
    pub(crate) vault_path: PathBuf,
    /// Opened once at server construction and reused wherever request handlers
    /// need DB access (e.g. MCP-over-HTTP gateway telemetry), instead of
    /// opening a fresh connection pool per request.
    pub(crate) db: hq_db::Database,
    pub(crate) static_dir: Option<PathBuf>,
    pub(crate) registry: Option<Arc<ToolRegistry>>,
    pub(crate) hq_config: Option<Arc<hq_core::config::HqConfig>>,
    /// Token required on /ws and /api (see `auth::web_auth_middleware`). Set
    /// by the caller from the same config its bind check used.
    pub web_auth_token: Option<String>,
    /// Browser origins allowed besides loopback and same-origin pages, from
    /// `web_allowed_origins` (see `origin::origin_guard`).
    pub(crate) allowed_origins: Vec<String>,
    /// Whether the server listens on loopback only, which is the one place the
    /// `/mcp` development switch may apply.
    pub web_bind_is_loopback: bool,
    /// Single-use tickets for opening `/ws` when a web token is set.
    pub(crate) ws_tickets: Arc<auth::WsTickets>,
    /// Abort handles for in-flight chat turns, keyed by thread id. A "stop"
    /// WS message aborts the turn's driver task, which drops the harness
    /// stream and finalizes the turn.
    pub(crate) active_chat_turns: ws::ChatTurnMap,
}

impl WsState {
    pub fn new(vault_path: PathBuf, static_dir: Option<PathBuf>) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let db = hq_db::Database::open(&vault_path.join("_data").join("vault.db"))
            .expect("failed to open vault database");
        let hq_config = hq_core::config::HqConfig::load().ok().map(Arc::new);
        let allowed_origins = hq_config
            .as_ref()
            .map(|c| c.web_allowed_origins.clone())
            .unwrap_or_default();
        let web_bind_is_loopback = hq_config
            .as_ref()
            .is_none_or(|c| auth::bind_is_loopback(&c.web_bind));
        Self {
            tx,
            vault_path,
            db,
            static_dir,
            registry: None,
            hq_config,
            web_auth_token: None,
            allowed_origins,
            web_bind_is_loopback,
            ws_tickets: Arc::new(auth::WsTickets::default()),
            active_chat_turns: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub fn with_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    pub(crate) fn broadcast(&self, msg: &str) {
        let _ = self.tx.send(msg.to_string());
    }

}

/// Makes `hq_ask` usable on this server: fails the asks a previous process left
/// pending, then installs the runner that starts replies in this server's chat.
/// Call once, after the state is shared.
pub fn install_ask_runner(state: &Arc<WsState>) {
    let failed = ws::reconcile_asks_after_restart(&state.db);
    if failed > 0 {
        tracing::info!(failed, "failed hq_ask questions whose reply died with the last process");
    }
    if !hq_tools::ask::install_runner(Arc::new(ws::WebAskRunner::new(state))) {
        tracing::warn!("an hq_ask runner was already installed; keeping it");
    }
}

/// Events buffered per client before a slow one starts missing them; every streamed token is one.
const BROADCAST_CAPACITY: usize = 4096;

/// Create the web router — vault-focused endpoints only.
pub fn create_router(state: Arc<WsState>) -> Router {
    let static_dir = state
        .static_dir
        .as_deref()
        .unwrap_or(std::path::Path::new("web/dist"))
        .to_path_buf();

    notifications_watch::spawn_notifications_watcher(state.clone());
    tasks_watch::spawn_tasks_watcher(state.clone());
    session_driver::spawn_session_driver(state.clone());
    subagent_followup::spawn_subagent_followup(state.clone());

    let router = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/health", get(api::health_handler))
        .route("/api/vault-status", get(api::vault_status_handler))
        .route("/api/ws-ticket", post(auth::ws_ticket_handler))
        .route(
            "/api/admin/broadcast",
            axum::routing::post(api::admin_broadcast_handler),
        )
        .route("/api/search", get(api::search_handler))
        .route(
            "/api/note",
            get(api::note_read_handler).put(vault_api::note_update_handler),
        )
        .route("/api/note/create", post(vault_api::note_create_handler))
        .route("/api/note/pdf", get(vault_api::note_pdf_handler))
        .route("/api/note/export", get(vault_api::note_export_handler))
        .route("/api/vault/folders", get(vault_api::folders_handler))
        .route("/api/vault-signals", get(vault_api::signals_handler))
        .route("/api/tree", get(api::tree_handler))
        .route("/api/settings", get(settings_api::settings_handler))
        .route("/api/setup/status", get(setup_api::status_handler))
        .route("/api/setup/test", post(setup_api::test_handler))
        .route("/api/setup/provider", post(setup_api::provider_handler))
        .route(
            "/api/copilot-usage",
            get(copilot_usage_api::copilot_usage_handler),
        )
        .route(
            "/api/openrouter-usage",
            get(openrouter_usage_api::openrouter_usage_handler),
        )
        .route(
            "/api/budgets",
            get(budgets_api::budgets_handler).put(budgets_api::put_budgets_handler),
        )
        .route(
            "/api/usage/forecast",
            get(usage_forecast_api::usage_forecast_handler),
        )
        .route(
            "/api/usage/providers",
            get(usage_providers_api::usage_providers_handler),
        )
        .route("/api/pinned", get(api::pinned_handler))
        .route("/api/pin", axum::routing::post(api::pin_toggle_handler))
        .route("/api/vault-asset", get(api::vault_asset_handler))
        .route("/api/chat/uploads", chat_uploads::route())
        .route(
            "/api/threads",
            get(threads_api::list_threads_handler).post(threads_api::create_thread_handler),
        )
        .route(
            "/api/threads/{thread_id}/messages",
            get(threads_api::get_thread_messages_handler),
        )
        .route(
            "/api/threads/{thread_id}/archive",
            axum::routing::post(threads_api::archive_thread_handler),
        )
        .route("/api/notifications", get(notifications_api::list_notifications_handler))
        .route(
            "/api/notifications/{id}/action",
            axum::routing::post(notifications_api::notification_action_handler),
        )
        .route(
            "/api/notifications/mark-all-read",
            axum::routing::post(notifications_api::mark_all_read_handler),
        )
        .route(
            "/api/spaces",
            get(tasks_api::list_spaces_handler).post(tasks_api::create_space_handler),
        )
        .route(
            "/api/folders",
            get(tasks_api::list_folders_handler).post(tasks_api::create_folder_handler),
        )
        .route(
            "/api/initiatives",
            get(tasks_api::list_initiatives_handler).post(tasks_api::create_initiative_handler),
        )
        .route(
            "/api/tasks",
            get(tasks_api::list_tasks_handler).post(tasks_api::create_task_handler),
        )
        .route(
            "/api/tasks/{id}",
            axum::routing::patch(tasks_api::update_task_handler)
                .delete(tasks_api::delete_task_handler),
        )
        .route(
            "/api/tasks/{id}/comments",
            get(tasks_api::list_comments_handler).post(tasks_api::add_comment_handler),
        )
        .route(
            "/api/tasks/{id}/events",
            get(tasks_api::list_task_events_handler),
        )
        .route(
            "/api/threads/{thread_id}/sessions",
            get(sessions_api::list_thread_sessions_handler),
        )
        .route("/api/harness-sessions", get(sessions_api::list_all_handler).post(workbench_api::spawn_handler))
        .route("/api/workbench/hosts", get(workbench_api::hosts_handler))
        .route(
            "/api/workbench/hosts/{host}/dirs",
            get(workbench_api::list_dirs_handler).post(workbench_api::make_dir_handler),
        )
        .route("/api/harness-sessions/{id}/stop", post(workbench_api::stop_handler))
        .route("/api/harness-sessions/{id}/resume", post(workbench_api::resume_handler))
        .route("/api/harness-sessions/{id}/rename", post(workbench_api::rename_handler))
        .route("/api/harness-sessions/{id}/archive", post(workbench_api::archive_handler))
        .route("/api/harness-sessions/{id}", get(sessions_api::get_session_handler))
        .route("/api/harness-sessions/{id}/screen", get(sessions_api::screen_handler))
        .route("/api/harness-sessions/{id}/screen/stream", get(sessions_api::screen_stream_handler))
        .route("/api/harness-sessions/{id}/send", post(sessions_api::send_handler))
        .route("/api/harness-sessions/{id}/adopt", post(sessions_api::adopt_handler))
        .route(
            "/api/harness-sessions/{id}/drive",
            post(sessions_api::set_drive_handler),
        )
        .route(
            "/api/harness-sessions/{id}/goal",
            post(sessions_api::set_goal_handler),
        )
        .route(
            "/api/harness-sessions/{id}/unwatch",
            post(sessions_api::unwatch_handler),
        )
        .route(
            "/hooks/gmail",
            axum::routing::post(gmail_ingest::gmail_hook_handler),
        )
        // Unknown server paths 404 as JSON instead of falling through to the PWA shell.
        .route("/api/{*rest}", axum::routing::any(not_found_json))
        .route("/ws/{*rest}", axum::routing::any(not_found_json))
        .route("/hooks/{*rest}", axum::routing::any(not_found_json))
        .route("/mcp/{*rest}", axum::routing::any(not_found_json))
        .route("/mcp", axum::routing::post(mcp_http::mcp_handler));

    router
        // Hashed build chunks: a missing one must 404, not come back as the HTML shell.
        .nest_service("/assets", ServeDir::new(static_dir.join("assets")))
        // The PWA: static files, and index.html for every client-side route.
        // Added before the layers so they wrap it too.
        .fallback_service(ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html"))))
        .layer(axum::middleware::from_fn(no_cache_shell))
        // Slow mobile links: gzip/brotli the PWA's JS and CSS and every JSON response.
        .layer(tower_http::compression::CompressionLayer::new())
        // Inside CORS, so preflight requests are answered before auth runs.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::web_auth_middleware,
        ))
        // Before auth, so a hostile page learns nothing from a 401.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            origin::origin_guard,
        ))
        .layer(cors_layer(state.allowed_origins.clone()))
        // Outermost, so CORS preflights and guard rejections carry the headers too.
        .layer(axum::middleware::from_fn_with_state(
            security_headers::content_security_policy(&static_dir),
            security_headers::set_security_headers,
        ))
        .with_state(state)
}

/// CORS grants read access only to origins the origin guard would let through.
fn cors_layer(allowed: Vec<String>) -> CorsLayer {
    let predicate = move |origin: &axum::http::HeaderValue, parts: &axum::http::request::Parts| {
        let host = parts
            .headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok());
        origin
            .to_str()
            .is_ok_and(|o| origin::origin_allowed(o, host, &allowed))
    };
    CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(predicate))
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PATCH,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::header::HeaderName::from_static("x-api-key"),
        ])
}

async fn not_found_json() -> axum::response::Response {
    axum::response::IntoResponse::into_response(error::ApiError::not_found())
}

/// The PWA shell and its service worker must revalidate on every load, or an
/// installed app keeps running an old build after a deploy.
async fn no_cache_shell(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let is_sw = req.uri().path() == "/sw.js";
    let mut res = next.run(req).await;
    let is_html = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if is_sw || is_html {
        res.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
    }
    res
}

#[cfg(test)]
mod web_auth_router_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn status(app: &Router, request: Request<Body>) -> StatusCode {
        app.clone().oneshot(request).await.unwrap().status()
    }

    /// The guard sits inside CORS and in front of every /api route, but never
    /// in front of /health, which the deploy rollback check polls.
    #[tokio::test]
    async fn token_guards_api_but_not_health_or_preflight() {
        let vault = tempfile::TempDir::new().unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        state.web_auth_token = Some("s3cret".to_string());
        let app = create_router(Arc::new(state));

        let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
        assert_eq!(status(&app, get("/api/spaces")).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(&app, get("/health")).await, StatusCode::OK);
        let authed = Request::get("/api/spaces")
            .header("authorization", "Bearer s3cret")
            .body(Body::empty())
            .unwrap();
        assert_ne!(status(&app, authed).await, StatusCode::UNAUTHORIZED);
        let preflight = |origin: &str| {
            Request::options("/api/spaces")
                .header("origin", origin)
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .unwrap()
        };
        let own = app.clone().oneshot(preflight("http://localhost:5173")).await.unwrap();
        assert_eq!(own.status(), StatusCode::OK);
        assert_eq!(own.headers()["access-control-allow-origin"], "http://localhost:5173");
        let hostile = app.clone().oneshot(preflight("https://evil.example")).await.unwrap();
        assert!(
            !hostile.headers().contains_key("access-control-allow-origin"),
            "a hostile origin must not be granted read access"
        );
    }

    fn local(method: &str, uri: &str) -> axum::http::request::Builder {
        Request::builder().method(method).uri(uri).header("host", "127.0.0.1:5678")
    }

    /// Unauthenticated loopback mode: a hostile page can neither change state
    /// nor open the chat socket, and a rebound DNS name is refused outright.
    #[tokio::test]
    async fn hostile_pages_cannot_use_an_unauthenticated_instance() {
        let vault = tempfile::TempDir::new().unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        state.allowed_origins = Vec::new();
        let app = create_router(Arc::new(state));

        let post = local("POST", "/api/note/create")
            .header("origin", "https://evil.example")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"title":"pwned","content":""}"#))
            .unwrap();
        assert_eq!(status(&app, post).await, StatusCode::FORBIDDEN);
        assert!(!vault.path().join("Notebooks/Inbox/pwned.md").exists());

        let socket = local("GET", "/ws").header("origin", "https://evil.example").body(Body::empty()).unwrap();
        assert_eq!(status(&app, socket).await, StatusCode::FORBIDDEN);

        let rebound = Request::get("/api/spaces")
            .header("host", "rebind.evil.example:5678")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(&app, rebound).await, StatusCode::FORBIDDEN);

        let own = local("GET", "/api/spaces").header("origin", "http://127.0.0.1:5678").body(Body::empty()).unwrap();
        assert_eq!(status(&app, own).await, StatusCode::OK);
    }

    /// A page on another loopback port passes the origin check, so session-changing
    /// requests also need a header it cannot add without a failed CORS preflight.
    #[tokio::test]
    async fn session_posts_need_the_client_header() {
        let vault = tempfile::TempDir::new().unwrap();
        let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
        let post = |path: &str, header: bool| {
            let mut b = local("POST", path)
                .header("origin", "http://localhost:9999")
                .header("content-type", "application/json");
            if header {
                b = b.header("x-hq-client", "web");
            }
            b.body(Body::from("{}")).unwrap()
        };
        for action in ["send", "adopt", "stop", "resume", "rename", "archive"] {
            let path = format!("/api/harness-sessions/hs-none/{action}");
            assert_eq!(
                status(&app, post(&path, false)).await,
                StatusCode::FORBIDDEN,
                "{action}"
            );
            assert_ne!(
                status(&app, post(&path, true)).await,
                StatusCode::FORBIDDEN,
                "{action}"
            );
        }
        for action in ["drive", "goal", "unwatch"] {
            let path = format!("/api/harness-sessions/hs-none/{action}");
            assert_ne!(
                status(&app, post(&path, false)).await,
                StatusCode::FORBIDDEN,
                "{action} stays open to cached clients"
            );
        }
        for path in ["/api/harness-sessions", "/api/workbench/hosts/native/dirs"] {
            assert_eq!(status(&app, post(path, false)).await, StatusCode::FORBIDDEN, "{path}");
            assert_ne!(status(&app, post(path, true)).await, StatusCode::FORBIDDEN, "{path}");
        }
        let preflight = Request::options("/api/harness-sessions/hs-none/send")
            .header("origin", "http://localhost:9999")
            .header("host", "127.0.0.1:5678")
            .header("access-control-request-method", "POST")
            .header("access-control-request-headers", "x-hq-client,content-type")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(preflight).await.unwrap();
        let allowed = res
            .headers()
            .get("access-control-allow-headers")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            !allowed.to_ascii_lowercase().contains("x-hq-client"),
            "CORS must not grant the header: {allowed}"
        );
    }

    /// URL tokens are refused; the socket opens with a one-time ticket that was
    /// minted over the Authorization header.
    #[tokio::test]
    async fn socket_uses_single_use_tickets_not_url_tokens() {
        let vault = tempfile::TempDir::new().unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        state.web_auth_token = Some("s3cret".to_string());
        let state = Arc::new(state);
        let app = create_router(state.clone());

        let url_token = local("GET", "/api/spaces?token=s3cret").body(Body::empty()).unwrap();
        assert_eq!(status(&app, url_token).await, StatusCode::UNAUTHORIZED);
        let unauthed_mint = local("POST", "/api/ws-ticket").body(Body::empty()).unwrap();
        assert_eq!(status(&app, unauthed_mint).await, StatusCode::UNAUTHORIZED);

        let mint = local("POST", "/api/ws-ticket")
            .header("authorization", "Bearer s3cret")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(mint).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
        let ticket = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["ticket"]
            .as_str()
            .unwrap()
            .to_string();

        let with_ticket = |t: &str| local("GET", &format!("/ws?ticket={t}")).body(Body::empty()).unwrap();
        // A plain GET is not an upgrade, so the handler rejects it, but it got past auth.
        assert_ne!(status(&app, with_ticket(&ticket)).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(&app, with_ticket(&ticket)).await, StatusCode::UNAUTHORIZED, "replayed");
        let ticket_elsewhere = local("GET", &format!("/api/spaces?ticket={}", state.ws_tickets.mint()))
            .body(Body::empty())
            .unwrap();
        assert_eq!(status(&app, ticket_elsewhere).await, StatusCode::UNAUTHORIZED);
    }

    /// Client routes get the PWA shell (never cached); unknown API paths 404 as JSON.
    #[tokio::test]
    async fn spa_fallback_serves_shell_but_not_for_api_paths() {
        let vault = tempfile::TempDir::new().unwrap();
        let web = tempfile::TempDir::new().unwrap();
        std::fs::write(web.path().join("index.html"), "<html>shell</html>").unwrap();
        std::fs::write(web.path().join("sw.js"), "//sw").unwrap();
        std::fs::create_dir_all(web.path().join("assets")).unwrap();
        std::fs::write(web.path().join("assets/app-1.js"), "//app ".repeat(200)).unwrap();
        let state = WsState::new(vault.path().to_path_buf(), Some(web.path().to_path_buf()));
        let app = create_router(Arc::new(state));
        let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();

        for uri in ["/", "/vault/Notebooks/a.md", "/tasks/123"] {
            let res = app.clone().oneshot(get(uri)).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            assert_eq!(res.headers()["cache-control"], "no-cache", "{uri}");
        }
        let gz = app
            .clone()
            .oneshot(Request::get("/assets/app-1.js").header("accept-encoding", "gzip").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(gz.headers()["content-encoding"], "gzip", "static assets are compressed");
        let sw = app.clone().oneshot(get("/sw.js")).await.unwrap();
        assert_eq!(sw.headers()["cache-control"], "no-cache");
        let asset = app.clone().oneshot(get("/assets/app-1.js")).await.unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
        let chunk = app.clone().oneshot(get("/assets/gone-123.js")).await.unwrap();
        assert_eq!(chunk.status(), StatusCode::NOT_FOUND);
        for uri in ["/api/nope", "/ws/nope", "/mcp/nope", "/hooks/nope"] {
            let res = app.clone().oneshot(get(uri)).await.unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
            assert!(res.headers()["content-type"].to_str().unwrap().starts_with("application/json"), "{uri}");
        }
    }

    /// Every vault endpoint that takes a client path refuses to leave the vault.
    #[tokio::test]
    async fn vault_endpoints_reject_traversal() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
        let json_req = |method: &str, uri: &str, body: &str| {
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
        for req in [
            get("/api/tree?recursive=true&path=../"),
            get("/api/note?path=../etc/passwd"),
            get("/api/note/pdf?path=../etc/passwd"),
            get("/api/note/export?path=../etc/passwd&format=html"),
            json_req("POST", "/api/note/create", r#"{"folder":"../x","title":"t","content":""}"#),
            json_req("PUT", "/api/note", r#"{"path":"../x.md","content":""}"#),
            json_req("PUT", "/api/note", r#"{"path":"/etc/hosts","content":""}"#),
        ] {
            let uri = req.uri().to_string();
            assert_eq!(status(&app, req).await, StatusCode::BAD_REQUEST, "{uri}");
        }
        let created = json_req("POST", "/api/note/create", r#"{"title":"Hello","content":"hi"}"#);
        assert_eq!(status(&app, created).await, StatusCode::OK);
        assert!(vault.path().join("Notebooks/Inbox/Hello.md").exists());
    }

    #[tokio::test]
    async fn note_pdf_endpoint_validates_its_input() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(vault.path().join("Notebooks/pic.png"), b"x").unwrap();
        let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
        let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
        assert_eq!(status(&app, get("/api/note/pdf")).await, StatusCode::BAD_REQUEST);
        assert_eq!(status(&app, get("/api/note/pdf?path=Missing")).await, StatusCode::NOT_FOUND);
        assert_eq!(
            status(&app, get("/api/note/pdf?path=Notebooks/pic.png")).await,
            StatusCode::BAD_REQUEST
        );
    }

    async fn export_vault() -> (tempfile::TempDir, Router) {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(
            vault.path().join("Notebooks/Plan.md"),
            "---\ntitle: Plan\n---\nHello\n\n## Sites\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        )
        .unwrap();
        std::fs::write(vault.path().join("Notebooks/Prose.md"), "Just words.\n").unwrap();
        std::fs::write(vault.path().join("Notebooks/pic.png"), b"x").unwrap();
        let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
        (vault, app)
    }

    #[tokio::test]
    async fn note_export_endpoint_validates_its_input() {
        let (_vault, app) = export_vault().await;
        let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
        assert_eq!(status(&app, get("/api/note/export")).await, StatusCode::BAD_REQUEST);
        assert_eq!(
            status(&app, get("/api/note/export?path=Missing&format=html")).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(&app, get("/api/note/export?path=Plan&format=docx-nope")).await,
            StatusCode::BAD_REQUEST,
            "an unknown format is the caller's mistake"
        );
        assert_eq!(
            status(&app, get("/api/note/export?path=Notebooks/pic.png&format=html")).await,
            StatusCode::BAD_REQUEST
        );
        // A note that has nothing of the requested kind is a 400, not a 500.
        for format in ["csv", "xlsx", "json", "code"] {
            let uri = format!("/api/note/export?path=Prose&format={format}");
            assert_eq!(status(&app, get(&uri)).await, StatusCode::BAD_REQUEST, "{uri}");
        }
    }

    #[tokio::test]
    async fn note_export_endpoint_serves_each_format_with_matching_headers() {
        let (_vault, app) = export_vault().await;
        for (format, mime, ext) in [
            ("html", "text/html", "html"),
            ("md", "text/markdown", "md"),
            ("csv", "text/csv", "csv"),
            ("json", "application/json", "json"),
            ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "xlsx"),
            ("ipynb", "application/x-ipynb+json", "ipynb"),
        ] {
            let uri = format!("/api/note/export?path=Plan&format={format}");
            let res = app
                .clone()
                .oneshot(Request::get(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            assert!(res.headers()["content-type"].to_str().unwrap().starts_with(mime), "{uri}");
            let disposition = res.headers()["content-disposition"].to_str().unwrap().to_owned();
            assert!(disposition.contains(&format!("Plan.{ext}")), "{uri}: {disposition}");
            // A router layer may rewrite no-store to no-cache; either keeps a
            // private note export out of shared caches.
            let cache = res.headers()["cache-control"].to_str().unwrap().to_owned();
            assert!(cache.contains("no-store") || cache.contains("no-cache"), "{uri}: {cache}");
            let body = axum::body::to_bytes(res.into_body(), 16 * 1024 * 1024).await.unwrap();
            assert!(!body.is_empty(), "{uri}");
        }
    }

    /// The built-in renderer needs no external tool, so this is a normal test.
    /// It steps aside when a developer has forced one of the older engines.
    #[tokio::test]
    async fn note_pdf_endpoint_serves_a_pdf() {
        if std::env::var("HQ_PDF_ENGINE").is_ok_and(|v| !v.trim().is_empty()) {
            return;
        }
        let (_vault, app) = export_vault().await;
        for uri in ["/api/note/pdf?path=Plan", "/api/note/export?path=Plan&format=pdf", "/api/note/export?path=Plan"] {
            let res = app
                .clone()
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            assert_eq!(res.headers()["content-type"], "application/pdf", "{uri}");
            assert!(res.headers()["content-disposition"].to_str().unwrap().contains("Plan.pdf"));
            let body = axum::body::to_bytes(res.into_body(), 16 * 1024 * 1024).await.unwrap();
            assert!(body.starts_with(b"%PDF"), "{uri}");
        }
    }
}

#[cfg(test)]
mod security_header_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const SHELL: &str = r#"<html><head><script>boot()</script></head><body><script type="module" src="/assets/m.js"></script></body></html>"#;

    fn app_with_vault(files: &[(&str, &str)]) -> (Router, tempfile::TempDir, tempfile::TempDir) {
        let vault = tempfile::TempDir::new().unwrap();
        for (name, body) in files {
            std::fs::write(vault.path().join(name), body).unwrap();
        }
        let web = tempfile::TempDir::new().unwrap();
        std::fs::write(web.path().join("index.html"), SHELL).unwrap();
        let state = WsState::new(vault.path().to_path_buf(), Some(web.path().to_path_buf()));
        (create_router(Arc::new(state)), vault, web)
    }

    async fn get(app: &Router, uri: &str) -> axum::response::Response {
        let req = Request::get(uri)
            .header("host", "127.0.0.1:5678")
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn every_response_carries_the_baseline_headers() {
        let (app, _v, _w) = app_with_vault(&[]);
        for uri in ["/", "/some/client/route", "/health", "/api/nope"] {
            let res = get(&app, uri).await;
            let h = res.headers();
            assert_eq!(h["x-content-type-options"], "nosniff", "{uri}");
            assert_eq!(h["referrer-policy"], "no-referrer", "{uri}");
            assert_eq!(h["x-frame-options"], "DENY", "{uri}");
            let csp = h["content-security-policy"].to_str().unwrap();
            assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'"), "{uri}: {csp}");
        }
    }

    #[tokio::test]
    async fn the_served_shell_is_runnable_under_its_own_policy() {
        let (app, _v, _w) = app_with_vault(&[]);
        let res = get(&app, "/").await;
        let csp = res.headers()["content-security-policy"].to_str().unwrap().to_string();
        let html = String::from_utf8(
            axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec(),
        )
        .unwrap();
        for hash in security_headers::inline_script_hashes(&html) {
            assert!(csp.contains(&format!("'{hash}'")), "inline script not allowed: {hash}");
        }
        let script_src = csp.split(';').find(|d| d.trim().starts_with("script-src")).unwrap();
        assert!(!script_src.contains("'unsafe-inline'"), "{script_src}");
    }

    /// Built-app tests need `bun run build` in apps/hq-web. CI sets
    /// `HQ_REQUIRE_BUILT_APP=1` so a missing build fails instead of skipping.
    fn built_app_missing(what: &str) {
        assert!(
            std::env::var_os("HQ_REQUIRE_BUILT_APP").is_none(),
            "HQ_REQUIRE_BUILT_APP is set but apps/hq-web is not built ({what})"
        );
        eprintln!("skipped: apps/hq-web is not built");
    }

    #[tokio::test]
    async fn the_socket_host_of_the_request_is_allowed_in_connect_src() {
        let (app, _v, _w) = app_with_vault(&[]);
        let req = Request::get("/").header("host", "hq.example.ts.net:8443").body(Body::empty()).unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let csp = res.headers()["content-security-policy"].to_str().unwrap().to_string();
        assert!(csp.contains("ws://hq.example.ts.net:8443 wss://hq.example.ts.net:8443"), "{csp}");
    }

    /// Reads the real build when it exists (`bun run build` in apps/hq-web).
    #[test]
    fn a_built_shell_only_loads_same_origin_assets_allowed_by_the_policy() {
        let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/hq-web/dist/client");
        let Ok(bytes) = std::fs::read(dist.join("index.html")) else {
            built_app_missing("index.html");
            return;
        };
        let html = String::from_utf8_lossy(&bytes).into_owned();
        let csp = security_headers::content_security_policy(&dist);
        let csp = csp.to_str().unwrap();
        let hashes = security_headers::inline_script_hashes(&html);
        let inline_count = html
            .split("<script")
            .skip(1)
            .filter(|t| !t.split('>').next().unwrap_or("").contains("src="))
            .count();
        assert!(inline_count >= 1 && hashes.len() == inline_count, "{inline_count} inline scripts, {} hashes", hashes.len());
        for hash in &hashes {
            assert!(csp.contains(&format!("'{hash}'")), "inline script missing from CSP");
        }
        // The TanStack stream barrier holds raw NUL bytes; the hash must be of the parser's text.
        if html.contains('\u{FFFD}') || bytes.contains(&0) {
            let raw_hash = |t: &str| {
                use base64::Engine;
                use sha2::Digest;
                format!("sha256-{}", base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(t.as_bytes())))
            };
            let raw_script = html.split("<script class=\"$tsr\" id=\"$tsr-stream-barrier\">").nth(1).and_then(|r| r.split("</script>").next());
            if let Some(raw) = raw_script.filter(|r| r.contains('\0')) {
                assert!(!csp.contains(&raw_hash(raw)), "the raw-byte hash must not be what is served");
            }
        }
        for attr in ["src=\"", "href=\""] {
            for part in html.split(attr).skip(1) {
                let url = part.split('"').next().unwrap();
                assert!(url.starts_with('/') && !url.starts_with("//"), "cross-origin or inline load: {url}");
            }
        }
        assert!(!html.contains("onclick="), "inline event handlers are blocked by the policy");
    }

    /// `nosniff` makes a browser refuse a worker script served with a non-JS type.
    #[tokio::test]
    async fn the_built_pdf_worker_is_served_as_javascript() {
        let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/hq-web/dist/client");
        let Some(worker) = std::fs::read_dir(dist.join("assets"))
            .ok()
            .and_then(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.starts_with("pdf.worker")))
        else {
            built_app_missing("pdf.worker asset");
            return;
        };
        let vault = tempfile::TempDir::new().unwrap();
        let state = WsState::new(vault.path().to_path_buf(), Some(dist));
        let app = create_router(Arc::new(state));
        let res = get(&app, &format!("/assets/{worker}")).await;
        assert_eq!(res.status(), StatusCode::OK);
        let ctype = res.headers()["content-type"].to_str().unwrap();
        assert!(ctype.contains("javascript"), "{ctype}");
        assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    }

    #[tokio::test]
    async fn active_vault_files_download_in_a_sandbox() {
        let (app, _v, _w) = app_with_vault(&[
            ("pic.svg", "<svg onload=alert(1)/>"),
            ("page.html", "<script>alert(1)</script>"),
            ("data.xml", "<a/>"),
            ("UPPER.SVG", "<svg/>"),
            ("photo.png", "x"),
            ("note.txt", "hi"),
        ]);
        for name in ["pic.svg", "page.html", "data.xml", "UPPER.SVG"] {
            let res = get(&app, &format!("/api/vault-asset?path={name}")).await;
            assert_eq!(res.status(), StatusCode::OK, "{name}");
            assert_eq!(res.headers()["content-disposition"], "attachment", "{name}");
            assert_eq!(res.headers()["content-security-policy"], "sandbox", "{name}");
            assert_eq!(res.headers()["x-content-type-options"], "nosniff", "{name}");
        }
        for name in ["photo.png", "note.txt"] {
            let res = get(&app, &format!("/api/vault-asset?path={name}")).await;
            assert!(!res.headers().contains_key("content-disposition"), "{name}");
            assert_ne!(res.headers()["content-security-policy"], "sandbox", "{name}");
        }
    }
}
