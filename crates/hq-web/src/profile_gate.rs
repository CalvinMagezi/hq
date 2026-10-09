//! Route gating for `profile: lite`.
//!
//! Lite serves the web app, tasks and the owner's notes, and nothing that starts a session,
//! runs a tool-using chat turn, changes settings or reaches another service. The gate is a
//! middleware on the whole router: a path the profile does not list answers the same JSON 404
//! as a path that does not exist, so a client learns nothing about what is switched off, and
//! enforcement does not depend on the web app hiding a button.

use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

use crate::WsState;

/// `/api` prefixes Lite serves. A prefix matches itself and anything below it, so
/// `/api/tasks` allows `/api/tasks/{id}/comments` but not `/api/tasksX`.
const LITE_API: &[&str] = &[
    "/api/tasks",
    "/api/spaces",
    "/api/folders",
    "/api/initiatives",
    "/api/note",
    "/api/search",
    "/api/tree",
    "/api/vault",
    "/api/vault-status",
    "/api/vault-signals",
    "/api/vault-asset",
    "/api/pinned",
    "/api/pin",
    "/api/notifications",
    // Opens the update socket the app listens on for live task changes.
    "/api/ws-ticket",
];

/// Whether `profile: lite` serves this request. Everything outside `/api` and `/hooks` is the
/// app's static files, `/health`, `/ws` (events; chat turns are refused in the handler) and
/// `/mcp` (own key, narrowed registry), which are served as under the full profile.
pub(crate) fn lite_allows(_method: &Method, path: &str) -> bool {
    if path.starts_with("/hooks") {
        return false;
    }
    if path != "/api" && !path.starts_with("/api/") {
        return true;
    }
    // Dot segments and doubled slashes are never part of a route HQ serves; a proxy that
    // normalizes them could turn a listed prefix into an unlisted route.
    if path.split('/').skip(1).any(|seg| seg == "." || seg == "..") || path.contains("//") {
        return false;
    }
    let listed = LITE_API
        .iter()
        .any(|p| path == *p || path.strip_prefix(p).is_some_and(|rest| rest.starts_with('/')));
    // Starting a conversation is chat, which Lite does not run.
    listed && !is_export(path)
}

/// Exports render documents with external tools; not part of Lite.
fn is_export(path: &str) -> bool {
    path == "/api/note/pdf" || path == "/api/note/export"
}

/// The middleware: a no-op under the full profile.
pub(crate) async fn lite_gate(
    State(state): State<Arc<WsState>>,
    request: Request,
    next: Next,
) -> Response {
    if state.profile().is_lite() && !lite_allows(request.method(), request.uri().path()) {
        return crate::error::ApiError::not_found().into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lite_serves_tasks_notes_search_and_the_app_shell() {
        for (method, path) in [
            (Method::GET, "/api/tasks"),
            (Method::POST, "/api/tasks"),
            (Method::PATCH, "/api/tasks/abc"),
            (Method::GET, "/api/tasks/abc/comments"),
            (Method::POST, "/api/tasks/abc/comments"),
            (Method::GET, "/api/spaces"),
            (Method::GET, "/api/note"),
            (Method::PUT, "/api/note"),
            (Method::POST, "/api/note/create"),
            (Method::GET, "/api/search"),
            (Method::GET, "/api/vault/folders"),
            (Method::GET, "/api/vault-status"),
            (Method::POST, "/api/ws-ticket"),
            (Method::GET, "/api/notifications"),
            (Method::POST, "/api/notifications/mark-all-read"),
            (Method::GET, "/health"),
            (Method::GET, "/ws"),
            (Method::POST, "/mcp"),
            (Method::GET, "/"),
            (Method::GET, "/tasks"),
            (Method::GET, "/assets/app.js"),
        ] {
            assert!(lite_allows(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn lite_refuses_sessions_chat_settings_and_everything_unlisted() {
        for (method, path) in [
            (Method::GET, "/api/harness-sessions"),
            (Method::POST, "/api/harness-sessions"),
            (Method::POST, "/api/harness-sessions/x/send"),
            (Method::GET, "/api/harness-sessions/x/screen/stream"),
            (Method::GET, "/api/workbench/hosts"),
            (Method::POST, "/api/workbench/hosts/h/dirs"),
            (Method::GET, "/api/setup/status"),
            (Method::POST, "/api/setup/provider"),
            (Method::POST, "/api/admin/broadcast"),
            (Method::GET, "/api/settings"),
            (Method::GET, "/api/threads"),
            (Method::POST, "/api/threads"),
            (Method::GET, "/api/threads/t/messages"),
            (Method::POST, "/api/chat/uploads"),
            (Method::GET, "/api/copilot-usage"),
            (Method::GET, "/api/openrouter-usage"),
            (Method::GET, "/api/budgets"),
            (Method::PUT, "/api/budgets"),
            (Method::GET, "/api/usage/providers"),
            (Method::GET, "/api/note/pdf"),
            (Method::POST, "/api/note/export"),
            (Method::GET, "/api/unknown"),
            (Method::POST, "/hooks/gmail"),
            (Method::GET, "/hooks/anything"),
            (Method::GET, "/api"),
        ] {
            assert!(!lite_allows(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn a_prefix_does_not_match_a_longer_name() {
        for path in ["/api/tasksX", "/api/notes", "/api/searching", "/api/pinned-x", "/api/vaults"] {
            assert!(!lite_allows(&Method::GET, path), "{path}");
        }
    }

    #[test]
    fn dot_segments_and_doubled_slashes_under_api_are_refused() {
        for path in [
            "/api/./harness-sessions",
            "/api/tasks/../harness-sessions",
            "/api/tasks/./x",
            "/api//harness-sessions",
            "/api/tasks//x",
            "/api/tasks/..",
            "/api/note/../../hooks/gmail",
        ] {
            assert!(!lite_allows(&Method::GET, path), "{path}");
        }
        // Legitimate ids that merely contain dots are fine.
        assert!(lite_allows(&Method::GET, "/api/tasks/PERS.1/comments"));
        assert!(lite_allows(&Method::GET, "/api/note"));
    }

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hq_core::config::Profile;
    use tower::ServiceExt;

    fn app(profile: Profile) -> (axum::Router, tempfile::TempDir) {
        let vault = tempfile::TempDir::new().unwrap();
        let state = WsState::new(vault.path().to_path_buf(), None).with_profile(profile);
        (crate::create_router(Arc::new(state)), vault)
    }

    async fn status(app: &axum::Router, method: &str, uri: &str) -> StatusCode {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:5678")
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap().status()
    }

    /// The same requests under both profiles: the full one answers (anything but 404), and
    /// Lite answers the unlisted ones exactly as it answers a path that does not exist.
    #[tokio::test]
    async fn lite_answers_unlisted_routes_like_unknown_ones_and_full_does_not() {
        let (lite, _v1) = app(Profile::Lite);
        let (full, _v2) = app(Profile::Full);
        let unknown = status(&lite, "GET", "/api/definitely-not-a-route").await;
        assert_eq!(unknown, StatusCode::NOT_FOUND);

        for (method, path) in [
            ("GET", "/api/harness-sessions"),
            ("GET", "/api/workbench/hosts"),
            ("GET", "/api/setup/status"),
            ("GET", "/api/settings"),
            ("GET", "/api/threads"),
            ("GET", "/api/budgets"),
            ("POST", "/api/admin/broadcast"),
            ("POST", "/hooks/gmail"),
        ] {
            assert_eq!(status(&lite, method, path).await, unknown, "lite {method} {path}");
            assert_ne!(
                status(&full, method, path).await,
                StatusCode::NOT_FOUND,
                "the full profile must serve {method} {path}, or this test proves nothing"
            );
        }
    }

    #[tokio::test]
    async fn lite_still_serves_tasks_search_health_and_the_task_events_socket_route() {
        let (lite, _v) = app(Profile::Lite);
        for (method, path) in [
            ("GET", "/api/spaces"),
            ("GET", "/api/tasks"),
            ("GET", "/api/search?q=x"),
            ("GET", "/api/vault-status"),
            ("GET", "/health"),
        ] {
            assert_ne!(status(&lite, method, path).await, StatusCode::NOT_FOUND, "{method} {path}");
        }
    }

    /// A proxy that normalizes dot segments must not turn a listed prefix into a session route.
    #[tokio::test]
    async fn lite_refuses_dot_segment_detours_to_sessions() {
        let (lite, _v) = app(Profile::Lite);
        let blocked = status(&lite, "GET", "/api/tasks/../harness-sessions").await;
        assert_eq!(blocked, StatusCode::NOT_FOUND);
    }
}
