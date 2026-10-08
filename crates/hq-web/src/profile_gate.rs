//! Route gating for `profile: lite`.
//!
//! Lite serves the web app, tasks and the owner's notes, and nothing that starts a session,
//! runs a tool-using chat turn, changes settings or reaches another service. The gate is a
//! middleware on the whole router: a path the profile does not list answers the same JSON 404
//! as a path that does not exist, so a client learns nothing about what is switched off, and
//! enforcement does not depend on the web app hiding a button.

use axum::extract::{Request, State};
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
    // Work-lease time for the task board's timeline and Working now panel.
    "/api/work-sessions",
    "/api/task-time-report",
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
pub(crate) fn lite_allows(path: &str) -> bool {
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
    if state.profile().is_lite() && !lite_allows(request.uri().path()) {
        return crate::error::ApiError::not_found().into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;

    #[test]
    fn lite_serves_tasks_notes_search_and_the_app_shell() {
        for (method, path) in [
            (Method::GET, "/api/tasks"),
            (Method::POST, "/api/tasks"),
            (Method::PATCH, "/api/tasks/abc"),
            (Method::GET, "/api/tasks/abc/comments"),
            (Method::POST, "/api/tasks/abc/comments"),
            (Method::GET, "/api/spaces"),
            (Method::GET, "/api/work-sessions"),
            (Method::GET, "/api/task-time-report"),
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
            assert!(lite_allows(path), "{method} {path}");
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
            assert!(!lite_allows(path), "{method} {path}");
        }
    }

    #[test]
    fn a_prefix_does_not_match_a_longer_name() {
        for path in ["/api/tasksX", "/api/notes", "/api/searching", "/api/pinned-x", "/api/vaults"] {
            assert!(!lite_allows(path), "{path}");
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
            assert!(!lite_allows(path), "{path}");
        }
        // Legitimate ids that merely contain dots are fine.
        assert!(lite_allows("/api/tasks/PERS.1/comments"));
        assert!(lite_allows("/api/note"));
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
    async fn lite_still_serves_tasks_search_and_health() {
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

    async fn body_status(app: &axum::Router, method: &str, uri: &str, body: &str) -> StatusCode {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", "127.0.0.1:5678")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap().status()
    }

    /// Lite's notes surface is the owner's notes, not HQ's own folders: the SQLite database,
    /// identity, threads and mailboxes sit in the vault, and the full profile serves them.
    #[tokio::test]
    async fn lite_keeps_hq_folders_out_of_the_notes_surface() {
        let (lite, lv) = app(Profile::Lite);
        let (full, fv) = app(Profile::Full);
        for v in [&lv, &fv] {
            std::fs::create_dir_all(v.path().join("_system")).unwrap();
            std::fs::write(v.path().join("_system/IDENTITY.md"), "who i am").unwrap();
            std::fs::create_dir_all(v.path().join("Notebooks")).unwrap();
            std::fs::write(v.path().join("Notebooks/plan.md"), "a plan").unwrap();
        }
        for uri in [
            "/api/vault-asset?path=_system/IDENTITY.md",
            "/api/vault-asset?path=_data/vault.db",
            "/api/note?path=_system/IDENTITY.md",
            "/api/tree?path=_system",
        ] {
            assert_eq!(status(&lite, "GET", uri).await, StatusCode::NOT_FOUND, "lite {uri}");
        }
        // Controls: the full profile serves the same requests, and Lite serves a real note.
        assert_eq!(status(&full, "GET", "/api/vault-asset?path=_system/IDENTITY.md").await, StatusCode::OK);
        assert_eq!(status(&full, "GET", "/api/tree?path=_system").await, StatusCode::OK);
        assert_eq!(status(&lite, "GET", "/api/note?path=Notebooks/plan.md").await, StatusCode::OK);
        assert_eq!(status(&lite, "GET", "/api/vault-asset?path=Notebooks/plan.md").await, StatusCode::OK);

        // Writes into those folders are refused too, and nothing is created.
        let put = r#"{"path":"_system/IDENTITY.md","content":"overwritten"}"#;
        assert_eq!(body_status(&lite, "PUT", "/api/note", put).await, StatusCode::NOT_FOUND);
        assert_eq!(std::fs::read_to_string(lv.path().join("_system/IDENTITY.md")).unwrap(), "who i am");
        let create = r#"{"folder":"_system","title":"planted","content":"x"}"#;
        assert_eq!(body_status(&lite, "POST", "/api/note/create", create).await, StatusCode::NOT_FOUND);
        assert!(!lv.path().join("_system/planted.md").exists());
        let hidden = r#"{"folder":".git","title":"planted","content":"x"}"#;
        assert_eq!(body_status(&lite, "POST", "/api/note/create", hidden).await, StatusCode::NOT_FOUND);
        // Control: a note in a normal folder is created.
        let ok = r#"{"folder":"Notebooks/Inbox","title":"fresh","content":"x"}"#;
        assert_eq!(body_status(&lite, "POST", "/api/note/create", ok).await, StatusCode::OK);
        assert!(lv.path().join("Notebooks/Inbox/fresh.md").exists());
    }

    #[test]
    fn hidden_and_underscore_folders_are_the_ones_lite_hides() {
        for rel in ["_system/x.md", "_data/vault.db", "_threads/t.md", "_mailboxes/relay", "./_system/x", ".git/config", "_Odd/x"] {
            assert!(crate::vault_api::lite_hides(rel), "{rel}");
        }
        for rel in ["Notebooks/_draft.md", "Notebooks/Inbox/x.md", "plan.md", "Projects/a_b/c.md", ""] {
            assert!(!crate::vault_api::lite_hides(rel), "{rel:?}");
        }
    }

    /// The review that found these: a padded path is trimmed by the note resolver after the
    /// check, `path=.` lists the vault root, the home page's activity bucket previews `_system`,
    /// and a symlink in a normal folder can point into a hidden one.
    #[tokio::test]
    async fn lite_hides_padded_root_signal_and_symlink_routes_into_hq_folders() {
        let (lite, lv) = app(Profile::Lite);
        let (full, fv) = app(Profile::Full);
        for v in [&lv, &fv] {
            std::fs::create_dir_all(v.path().join("_system")).unwrap();
            std::fs::write(v.path().join("_system/SOUL.md"), "---\ntitle: Soul\n---\nprivate words").unwrap();
            std::fs::create_dir_all(v.path().join("Notebooks")).unwrap();
            std::fs::write(v.path().join("Notebooks/plan.md"), "a plan").unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(v.path().join("_system"), v.path().join("Notebooks/linked")).unwrap();
        }

        // A padded path is the same path once the resolver trims it.
        for uri in ["/api/note?path=%20_system/SOUL.md", "/api/note?path=%09_system/SOUL.md", "/api/note?path=_system/SOUL.md%20"] {
            assert_eq!(status(&lite, "GET", uri).await, StatusCode::NOT_FOUND, "lite {uri}");
        }
        assert_eq!(status(&full, "GET", "/api/note?path=%20_system/SOUL.md").await, StatusCode::OK, "control");

        // The vault root and its dot spellings are not a notes folder.
        for uri in ["/api/tree?path=.", "/api/tree?path=./", "/api/tree?path=Notebooks/..%2F"] {
            let got = status(&lite, "GET", uri).await;
            assert_ne!(got, StatusCode::OK, "lite {uri}");
        }
        assert_eq!(status(&full, "GET", "/api/tree?path=.").await, StatusCode::OK, "control");
        assert_eq!(status(&lite, "GET", "/api/tree").await, StatusCode::OK, "the default folder still lists");

        // The activity bucket would preview `_system` notes.
        let signals = |app: axum::Router| async move {
            let req = Request::builder()
                .uri("/api/vault-signals")
                .header("host", "127.0.0.1:5678")
                .body(Body::empty())
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let lite_signals = signals(lite.clone()).await;
        assert!(!lite_signals.contains("private words") && !lite_signals.contains("Soul"), "{lite_signals}");
        let full_signals = signals(full.clone()).await;
        assert!(full_signals.contains("Soul"), "the full profile shows activity, so this proves something: {full_signals}");

        // A symlink inside the notes that leads into a hidden folder.
        #[cfg(unix)]
        {
            for uri in [
                "/api/note?path=Notebooks/linked/SOUL.md",
                "/api/vault-asset?path=Notebooks/linked/SOUL.md",
            ] {
                assert_eq!(status(&lite, "GET", uri).await, StatusCode::NOT_FOUND, "lite {uri}");
                assert_eq!(status(&full, "GET", uri).await, StatusCode::OK, "full {uri}");
            }
            let put = r#"{"path":"Notebooks/linked/SOUL.md","content":"overwritten"}"#;
            assert_eq!(body_status(&lite, "PUT", "/api/note", put).await, StatusCode::NOT_FOUND);
            assert!(std::fs::read_to_string(lv.path().join("_system/SOUL.md")).unwrap().contains("private words"));
            let create = r#"{"folder":"Notebooks/linked","title":"planted","content":"x"}"#;
            assert_eq!(body_status(&lite, "POST", "/api/note/create", create).await, StatusCode::NOT_FOUND);
            assert!(!lv.path().join("_system/planted.md").exists());
        }
    }

    #[test]
    fn padded_and_resolved_paths_are_judged_like_the_resolver_reads_them() {
        for rel in [" _system/x", "\t_system/x", " ./_system/x", "  .git/config "] {
            assert!(crate::vault_api::lite_hides(rel), "{rel:?}");
        }
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(vault.path().join("_system")).unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        let hides = |p: &str| crate::vault_api::lite_hides_resolved(vault.path(), &vault.path().join(p));
        assert!(hides("_system/new-note.md"), "a note that does not exist yet");
        assert!(hides(""), "the vault root");
        assert!(!hides("Notebooks/new/deeper/x.md"));
        assert!(hides("../elsewhere"), "outside the vault is hidden too");
    }
}
