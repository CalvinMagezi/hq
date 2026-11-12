//! Deleting a note from the apps, the Trash that lists what was deleted, and restoring from it.
//! A delete is a move into `_trash/` (`hq_vault::reorg`), the same one `vault_delete` makes.

use crate::WsState;
use crate::error::ApiError;
use crate::vault_api::{index_web_note, resolve_in_vault, split_frontmatter};
use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const NOTEBOOKS_PREFIX: &str = "Notebooks/";

#[derive(Deserialize)]
pub(crate) struct PathQuery {
    path: String,
}

fn vault_of(state: &WsState) -> Result<Arc<hq_vault::VaultClient>, ApiError> {
    hq_vault::VaultClient::new(state.vault_path.clone())
        .map(Arc::new)
        .map_err(ApiError::internal)
}

/// The note's path below `_trash/<day>/`, which is the path Lite's hidden folders are judged on.
fn path_below_trash_day(trash_rel: &str) -> &str {
    trash_rel.splitn(3, '/').nth(2).unwrap_or("")
}

/// Checks a trash path the way a note path is checked, and that the file is still there. Lite's
/// hidden folders are judged on the path below the day folder, because `_trash` itself counts as
/// hidden there; `hq_vault::reorg` refuses a file whose real location is outside the trash.
fn checked_trash_file(state: &WsState, rel: &str) -> Result<(), ApiError> {
    if !state.path_allowed(path_below_trash_day(rel)) {
        return Err(ApiError::not_found());
    }
    let Some(abs) = resolve_in_vault(&state.vault_path, rel).filter(|_| !rel.is_empty()) else {
        return Err(ApiError::bad_request("invalid path"));
    };
    if !abs.is_file() {
        return Err(ApiError::not_found());
    }
    Ok(())
}

pub(crate) async fn note_delete_handler(
    State(state): State<Arc<WsState>>,
    Query(query): Query<PathQuery>,
) -> Response {
    let rel = query.path.as_str();
    if !state.path_allowed(rel) {
        return ApiError::not_found().into_response();
    }
    let Some(abs) = resolve_in_vault(&state.vault_path, rel).filter(|_| !rel.is_empty()) else {
        return ApiError::bad_request("invalid path").into_response();
    };
    if !rel.starts_with(NOTEBOOKS_PREFIX) {
        return ApiError::bad_request("only notes in Notebooks can be deleted here")
            .into_response();
    }
    if !state.resolved_allowed(&abs) || !abs.is_file() {
        return ApiError::not_found().into_response();
    }
    let vault = match vault_of(&state) {
        Ok(vault) => vault,
        Err(e) => return e.into_response(),
    };
    let db = Some(Arc::new(state.db.clone()));
    match hq_tools::vault_reorg::trash_and_forget(vault, db, rel).await {
        Ok(trash_path) => Json(json!({"success": true, "trash_path": trash_path})).into_response(),
        Err(e) => ApiError::bad_request(e.to_string()).into_response(),
    }
}

pub(crate) async fn trash_list_handler(State(state): State<Arc<WsState>>) -> Response {
    let vault = match vault_of(&state) {
        Ok(vault) => vault,
        Err(e) => return e.into_response(),
    };
    let listed = match tokio::task::spawn_blocking(move || vault.list_trash()).await {
        Ok(Ok(listed)) => listed,
        Ok(Err(e)) => return ApiError::internal(e).into_response(),
        Err(e) => return ApiError::internal(e).into_response(),
    };
    let items: Vec<Value> = listed
        .into_iter()
        .filter(|n| state.path_allowed(&n.original_path))
        .map(|n| json!(n))
        .collect();
    Json(json!({
        "items": items,
        "retention_days": hq_vault::reorg::TRASH_RETENTION_DAYS,
    }))
    .into_response()
}

pub(crate) async fn trash_read_handler(
    State(state): State<Arc<WsState>>,
    Query(query): Query<PathQuery>,
) -> Response {
    if let Err(e) = checked_trash_file(&state, &query.path) {
        return e.into_response();
    }
    let vault = match vault_of(&state) {
        Ok(vault) => vault,
        Err(e) => return e.into_response(),
    };
    let rel = query.path;
    match tokio::task::spawn_blocking(move || vault.read_trashed(&rel)).await {
        Ok(Ok(raw)) => Json(json!({"content": split_frontmatter(&raw).1})).into_response(),
        Ok(Err(e)) => ApiError::bad_request(e.to_string()).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn trash_restore_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let rel = body
        .get("trash_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Err(e) = checked_trash_file(&state, &rel) {
        return e.into_response();
    }
    let vault = match vault_of(&state) {
        Ok(vault) => vault,
        Err(e) => return e.into_response(),
    };
    // Whether the note's folder is still there decides where it goes back to.
    let restored = match tokio::task::spawn_blocking(move || vault.restore_note(&rel)).await {
        Ok(Ok(restored)) => restored,
        Ok(Err(e)) => return ApiError::bad_request(e.to_string()).into_response(),
        Err(e) => return ApiError::internal(e).into_response(),
    };
    let hq_vault::reorg::RestoredNote {
        original_path,
        path: restored,
        restored_to,
    } = restored;
    let restored_to = restored_to.as_str();
    let abs = state.vault_path.join(&restored);
    if let Ok(raw) = std::fs::read_to_string(&abs) {
        index_web_note(&state, &abs, &raw);
    }
    // The note's old records come back with it, under its new name if it has one. Restoring is
    // not writing, so it stays credited to whoever wrote it and an agent's note does not become
    // the owner's.
    let path = restored.clone();
    if let Err(e) = state.db.with_conn(move |c| {
        if original_path == path {
            hq_db::provenance::clear_deleted(c, &path).map(|_| ())
        } else {
            hq_db::provenance::move_path(c, &original_path, &path).map(|_| ())
        }
    }) {
        tracing::warn!(error = %e, "note restore: could not revive its provenance");
    }
    Json(json!({"success": true, "path": restored, "restored_to": restored_to})).into_response()
}

#[cfg(test)]
mod tests {
    use crate::{WsState, create_router};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use std::sync::Arc;
    use tower::ServiceExt;

    const BODY_LIMIT: usize = 1 << 20;

    fn vault_with_note() -> (tempfile::TempDir, axum::Router) {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks/Inbox")).unwrap();
        std::fs::write(
            vault.path().join("Notebooks/Inbox/a.md"),
            "---\nx: 1\n---\nhello",
        )
        .unwrap();
        let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
        (vault, app)
    }

    async fn call(app: &axum::Router, method: &str, uri: &str, body: &str) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn delete_list_read_and_restore_round_trip() {
        let (vault, app) = vault_with_note();
        let (status, deleted) =
            call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        assert_eq!(status, StatusCode::OK);
        assert!(!vault.path().join("Notebooks/Inbox/a.md").exists());
        let trash_path = deleted["trash_path"].as_str().unwrap().to_string();

        let (_, listed) = call(&app, "GET", "/api/note/trash", "").await;
        assert_eq!(listed["items"][0]["original_path"], "Notebooks/Inbox/a.md");
        assert_eq!(listed["retention_days"], 30);

        let (status, read) = call(
            &app,
            "GET",
            &format!("/api/note/trash/read?path={trash_path}"),
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(read["content"].as_str().unwrap().trim(), "hello");

        let body = serde_json::json!({ "trash_path": trash_path }).to_string();
        let (status, restored) = call(&app, "POST", "/api/note/trash/restore", &body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(restored["path"], "Notebooks/Inbox/a.md");
        assert_eq!(restored["restored_to"], "original");
        assert!(vault.path().join("Notebooks/Inbox/a.md").exists());
    }

    #[tokio::test]
    async fn restore_says_folder_when_only_a_parent_survives() {
        let (vault, app) = vault_with_note();
        std::fs::create_dir_all(vault.path().join("Notebooks/Area/Deep")).unwrap();
        std::fs::write(vault.path().join("Notebooks/Area/Deep/b.md"), "x").unwrap();
        let (_, deleted) = call(
            &app,
            "DELETE",
            "/api/note?path=Notebooks/Area/Deep/b.md",
            "",
        )
        .await;
        let trash_path = deleted["trash_path"].as_str().unwrap().to_string();
        std::fs::remove_dir_all(vault.path().join("Notebooks/Area/Deep")).unwrap();
        let body = serde_json::json!({ "trash_path": trash_path }).to_string();
        let (_, restored) = call(&app, "POST", "/api/note/trash/restore", &body).await;
        assert_eq!(restored["restored_to"], "folder");
        assert_eq!(restored["path"], "Notebooks/Area/b.md");
    }

    #[test]
    fn the_trash_sentence_names_the_real_retention() {
        let sentence = hq_core::losing::consequence("vault_delete", &serde_json::json!({}), None);
        let days = hq_vault::reorg::TRASH_RETENTION_DAYS.to_string();
        assert!(sentence.unwrap().contains(&days));
    }

    #[tokio::test]
    async fn restore_says_vault_when_the_original_folder_is_gone() {
        let (vault, app) = vault_with_note();
        let (_, deleted) = call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        let trash_path = deleted["trash_path"].as_str().unwrap().to_string();
        std::fs::remove_dir_all(vault.path().join("Notebooks/Inbox")).unwrap();
        let body = serde_json::json!({ "trash_path": trash_path }).to_string();
        let (status, restored) = call(&app, "POST", "/api/note/trash/restore", &body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(restored["restored_to"], "vault");
        assert_eq!(restored["path"], "Notebooks/a.md");
        assert!(vault.path().join("Notebooks/a.md").exists());
        assert!(!vault.path().join("Notebooks/Inbox").exists());
    }

    #[tokio::test]
    async fn lite_can_read_and_restore_what_it_deleted() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks/Inbox")).unwrap();
        std::fs::write(vault.path().join("Notebooks/Inbox/a.md"), "hello").unwrap();
        let state = WsState::new(vault.path().to_path_buf(), None)
            .with_profile(hq_core::config::Profile::Lite);
        let app = create_router(Arc::new(state));
        let (_, deleted) = call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        let trash_path = deleted["trash_path"].as_str().unwrap().to_string();
        let uri = format!("/api/note/trash/read?path={trash_path}");
        assert_eq!(call(&app, "GET", &uri, "").await.0, StatusCode::OK);
        let body = serde_json::json!({ "trash_path": trash_path }).to_string();
        let (status, _) = call(&app, "POST", "/api/note/trash/restore", &body).await;
        assert_eq!(status, StatusCode::OK);
        assert!(vault.path().join("Notebooks/Inbox/a.md").exists());
    }

    #[tokio::test]
    async fn the_new_routes_refuse_paths_outside_their_place() {
        let (vault, app) = vault_with_note();
        std::fs::create_dir_all(vault.path().join("_system")).unwrap();
        std::fs::write(vault.path().join("_system/SOUL.md"), "x").unwrap();
        for path in ["../x.md", "/etc/hosts"] {
            let (status, _) = call(&app, "DELETE", &format!("/api/note?path={path}"), "").await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "delete {path}");
            let (status, _) = call(
                &app,
                "GET",
                &format!("/api/note/trash/read?path={path}"),
                "",
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "read {path}");
            let body = serde_json::json!({ "trash_path": path }).to_string();
            let (status, _) = call(&app, "POST", "/api/note/trash/restore", &body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "restore {path}");
        }
        let (status, _) = call(&app, "DELETE", "/api/note?path=_system/SOUL.md", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(vault.path().join("_system/SOUL.md").exists());
        let (status, _) = call(&app, "GET", "/api/note/trash/read?path=_system/SOUL.md", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let body = r#"{"trash_path":"Notebooks/Inbox/a.md"}"#;
        let (status, _) = call(&app, "POST", "/api/note/trash/restore", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/gone.md", "").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
