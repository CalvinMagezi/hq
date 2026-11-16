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
    // The owner is deleting here; the runtime says so, no argument does.
    match hq_tools::vault_reorg::trash_and_forget(vault, db, rel, true).await {
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
        .map(|n| {
            let by = state
                .db
                .with_conn(|c| hq_db::note_keep::deleted_by(c, &n.trash_path))
                .unwrap_or(hq_db::note_keep::DELETED_BY_YOU);
            let mut item = json!(n);
            item["deleted_by"] = json!(by);
            item
        })
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
    let trash_path = rel.clone();
    let restored = match tokio::task::spawn_blocking(move || vault.restore_note(&rel)).await {
        Ok(Ok(restored)) => restored,
        Ok(Err(e)) => return ApiError::bad_request(e.to_string()).into_response(),
        Err(e) => return ApiError::internal(e).into_response(),
    };
    if let Err(e) = state
        .db
        .with_conn(|c| hq_db::note_keep::forget_trash_item(c, &trash_path))
    {
        tracing::warn!(error = %e, "note restore: could not drop the who-deleted row");
    }
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

/// Removes one Trash item and clears the records of the note it held. `others_remain` says
/// whether another Trash copy of the same note stays; its records stay too. A new note under the
/// same name is found on disk, not in the records, and keeps its own.
fn purge_one(
    state: &WsState,
    vault: &hq_vault::VaultClient,
    trash_path: &str,
    others_remain: impl FnOnce(&str) -> bool,
) -> Result<(), ApiError> {
    let original = vault.purge_trashed(trash_path).map_err(|e| {
        let gone = e
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound);
        if gone {
            ApiError::not_found()
        } else {
            ApiError::bad_request(e.to_string())
        }
    })?;
    let another_copy = others_remain(&original);
    let live = std::fs::symlink_metadata(state.vault_path.join(&original)).is_ok();
    let trash_path = trash_path.to_string();
    let done = state.db.with_conn(move |c| {
        hq_db::note_keep::forget_trash_item(c, &trash_path)?;
        if another_copy {
            return Ok(());
        }
        hq_db::note_keep::purge_records(c, &original, live)
    });
    if let Err(e) = done {
        tracing::warn!(error = %e, "trash purge: could not clear the note's records");
    }
    Ok(())
}

/// `POST /api/note/trash/purge {path}`: delete one Trash item for good. Owner-only: no tool
/// reaches it.
pub(crate) async fn trash_purge_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let rel = body
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if rel.contains('\\') {
        return ApiError::bad_request("invalid path").into_response();
    }
    if let Err(e) = checked_trash_file(&state, &rel) {
        return e.into_response();
    }
    let vault = match vault_of(&state) {
        Ok(vault) => vault,
        Err(e) => return e.into_response(),
    };
    let worker = state.clone();
    let done = tokio::task::spawn_blocking(move || {
        let listed = vault.list_trash().unwrap_or_default();
        let this = rel.clone();
        let others = move |original: &str| {
            listed
                .iter()
                .any(|n| n.original_path == original && n.trash_path != this)
        };
        purge_one(&worker, &vault, &rel, others)
    })
    .await;
    match done {
        Ok(Ok(())) => Json(json!({"success": true, "purged": 1})).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// `POST /api/note/trash/empty`: delete everything the Trash lists, for good. Answers how many
/// went (`purged`) and how many could not (`failed`).
pub(crate) async fn trash_empty_handler(State(state): State<Arc<WsState>>) -> Response {
    let vault = match vault_of(&state) {
        Ok(vault) => vault,
        Err(e) => return e.into_response(),
    };
    let worker = state.clone();
    let done = tokio::task::spawn_blocking(move || {
        let mut purged = 0usize;
        let mut failed = std::collections::HashSet::new();
        // The list is capped, so go round once per page: one listing per round, not per item.
        loop {
            let listed = vault.list_trash()?;
            let truncated = listed.len() >= hq_vault::reorg::MAX_TRASH_LISTED;
            let mut round = 0usize;
            for item in listed
                .iter()
                .filter(|n| worker.path_allowed(&n.original_path))
            {
                if failed.contains(&item.trash_path) {
                    continue;
                }
                // Everything listed goes this round, so only a copy beyond the page can remain.
                match purge_one(&worker, &vault, &item.trash_path, |_| truncated) {
                    Ok(()) => round += 1,
                    // Removed by someone else while we ran: the goal is met.
                    Err(ApiError::NotFound(_)) => {}
                    Err(e) => {
                        tracing::warn!(error = ?e, path = %item.trash_path, "trash empty: could not remove an item");
                        failed.insert(item.trash_path.clone());
                    }
                }
            }
            purged += round;
            if round == 0 {
                return anyhow::Ok((purged, failed.len()));
            }
        }
    })
    .await;
    match done {
        Ok(Ok((purged, failed))) => {
            Json(json!({"success": failed == 0, "purged": purged, "failed": failed}))
                .into_response()
        }
        Ok(Err(e)) => ApiError::internal(e).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
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

    fn trash_path_of(deleted: &Value) -> String {
        deleted["trash_path"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn purge_removes_one_item_for_good_and_its_records() {
        let (vault, app) = vault_with_note();
        let (_, deleted) = call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        let trash_path = trash_path_of(&deleted);
        let body = serde_json::json!({ "path": trash_path }).to_string();
        let (status, purged) = call(&app, "POST", "/api/note/trash/purge", &body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(purged["purged"], 1);
        assert!(!vault.path().join(&trash_path).exists());
        let (_, listed) = call(&app, "GET", "/api/note/trash", "").await;
        assert_eq!(listed["items"].as_array().unwrap().len(), 0);
        // It is gone: restoring or purging it again is not found.
        let restore = serde_json::json!({ "trash_path": trash_path }).to_string();
        let (status, _) = call(&app, "POST", "/api/note/trash/restore", &restore).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(&app, "POST", "/api/note/trash/purge", &body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn purge_clears_provenance_and_leaves_a_line_without_text() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks/Inbox")).unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let app = create_router(state.clone());
        let abs = vault.path().join("Notebooks/Inbox/a.md");
        std::fs::write(&abs, "secret words").unwrap();
        crate::vault_api::record_owner_write(&state, &abs);
        let (_, deleted) = call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        let body = serde_json::json!({ "path": trash_path_of(&deleted) }).to_string();
        call(&app, "POST", "/api/note/trash/purge", &body).await;
        let (records, log): (i64, Vec<String>) = state
            .db
            .with_conn(|c| {
                let records = c.query_row(
                    "SELECT COUNT(*) FROM note_provenance WHERE path = 'Notebooks/Inbox/a.md'",
                    [],
                    |r| r.get(0),
                )?;
                let mut stmt = c.prepare("SELECT path || ' ' || purged_at FROM note_purge_log")?;
                let log = stmt
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok((records, log))
            })
            .unwrap();
        assert_eq!(records, 0);
        assert_eq!(log.len(), 1);
        assert!(log[0].starts_with("Notebooks/Inbox/a.md "));
        assert!(!log[0].contains("secret"));
    }

    #[tokio::test]
    async fn purge_refuses_paths_outside_the_trash() {
        let (vault, app) = vault_with_note();
        for path in [
            "../x.md",
            "/etc/hosts",
            "_trash\\2026-01-01\\Notebooks\\a.md",
        ] {
            let body = serde_json::json!({ "path": path }).to_string();
            let (status, _) = call(&app, "POST", "/api/note/trash/purge", &body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        }
        let body = r#"{"path":"Notebooks/Inbox/a.md"}"#;
        let (status, _) = call(&app, "POST", "/api/note/trash/purge", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(vault.path().join("Notebooks/Inbox/a.md").exists());
    }

    #[tokio::test]
    async fn empty_removes_every_item_and_says_how_many() {
        let (vault, app) = vault_with_note();
        std::fs::write(vault.path().join("Notebooks/Inbox/b.md"), "b").unwrap();
        call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/b.md", "").await;
        let (status, emptied) = call(&app, "POST", "/api/note/trash/empty", "{}").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(emptied["purged"], 2);
        let (_, listed) = call(&app, "GET", "/api/note/trash", "").await;
        assert_eq!(listed["items"].as_array().unwrap().len(), 0);
        let (_, again) = call(&app, "POST", "/api/note/trash/empty", "{}").await;
        assert_eq!(again["purged"], 0);
    }

    #[tokio::test]
    async fn the_trash_says_who_deleted_each_item() {
        let (vault, app) = vault_with_note();
        std::fs::write(vault.path().join("Notebooks/Inbox/b.md"), "b").unwrap();
        call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        // HQ deletes through the same function the tool uses, with the runtime's verdict.
        let client = Arc::new(hq_vault::VaultClient::new(vault.path().to_path_buf()).unwrap());
        hq_tools::vault_reorg::trash_and_forget(
            client,
            Some(Arc::new(
                hq_db::Database::open(&vault.path().join("_data/vault.db")).unwrap(),
            )),
            "Notebooks/Inbox/b.md",
            false,
        )
        .await
        .unwrap();
        let (_, listed) = call(&app, "GET", "/api/note/trash", "").await;
        let by = |name: &str| {
            listed["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["original_path"] == name)
                .unwrap()["deleted_by"]
                .clone()
        };
        assert_eq!(by("Notebooks/Inbox/a.md"), "you");
        assert_eq!(by("Notebooks/Inbox/b.md"), "hq");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn empty_reports_what_it_could_not_remove() {
        use std::os::unix::fs::PermissionsExt;
        let (vault, app) = vault_with_note();
        std::fs::write(vault.path().join("Notebooks/Inbox/b.md"), "b").unwrap();
        let (_, a) = call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/b.md", "").await;
        // A folder that cannot be written into makes its note impossible to remove.
        let locked = vault
            .path()
            .join(trash_path_of(&a))
            .parent()
            .unwrap()
            .to_path_buf();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let denied = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(locked.join("probe"))
            .is_err();
        let (status, emptied) = call(&app, "POST", "/api/note/trash/empty", "{}").await;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(status, StatusCode::OK);
        if denied {
            // Both items share the locked day folder.
            assert_eq!(emptied["failed"], 2, "{emptied}");
            assert_eq!(emptied["purged"], 0, "{emptied}");
            assert_eq!(emptied["success"], false);
        }
    }

    #[tokio::test]
    async fn purging_keeps_the_marks_of_a_live_note_that_has_the_name() {
        let (vault, app) = vault_with_note();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let app2 = create_router(state.clone());
        let _ = app;
        let (_, deleted) = call(&app2, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        // A new note takes the name; HQ has recorded nothing about it, but the owner opened it.
        std::fs::write(vault.path().join("Notebooks/Inbox/a.md"), "new").unwrap();
        state
            .db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO note_seen (path, content_hash, seen_at) VALUES ('Notebooks/Inbox/a.md', 'h', 't')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let body = serde_json::json!({ "path": trash_path_of(&deleted) }).to_string();
        let (status, _) = call(&app2, "POST", "/api/note/trash/purge", &body).await;
        assert_eq!(status, StatusCode::OK);
        let seen: i64 = state
            .db
            .with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM note_seen", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(seen, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_planted_in_the_trash_is_not_followed_by_a_purge() {
        let (vault, app) = vault_with_note();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep.md"), "outside").unwrap();
        let (_, deleted) = call(&app, "DELETE", "/api/note?path=Notebooks/Inbox/a.md", "").await;
        let planted = vault.path().join(trash_path_of(&deleted));
        std::fs::remove_file(&planted).unwrap();
        std::os::unix::fs::symlink(outside.path().join("keep.md"), &planted).unwrap();
        let body = serde_json::json!({ "path": trash_path_of(&deleted) }).to_string();
        let (status, _) = call(&app, "POST", "/api/note/trash/purge", &body).await;
        assert!(status.is_client_error());
        assert!(outside.path().join("keep.md").exists());
    }

    #[tokio::test]
    async fn the_owner_only_routes_refuse_connector_spark_and_anonymous_callers() {
        use crate::mcp_http::McpKeys;
        use crate::tls::tests::support::{WEB_TOKEN, mint_grant, world};
        const SPARK: &str = "spark-secret";
        let w = world();
        let grant = mint_grant(&w.state);
        let through_door = crate::devices_door::guard(
            w.router.clone(),
            McpKeys::with_spark(SPARK),
            w.state.clone(),
        );
        let routes = [
            (
                "POST",
                "/api/note/trash/purge",
                r#"{"path":"_trash/2026-01-01/Notebooks/a.md"}"#,
            ),
            ("POST", "/api/note/trash/empty", "{}"),
            ("GET", "/api/note/version?path=Notebooks/a.md&id=1", ""),
            (
                "POST",
                "/api/note/restore-version",
                r#"{"path":"Notebooks/a.md","id":1}"#,
            ),
        ];
        let connector = format!("Bearer {}", grant.access_token);
        let spark = format!("Bearer {SPARK}");
        let credentials: [(&str, Vec<(&str, &str)>); 4] = [
            ("none", vec![]),
            ("connector grant", vec![("authorization", &connector)]),
            ("spark bearer", vec![("authorization", &spark)]),
            ("spark x-api-key", vec![("x-api-key", SPARK)]),
        ];
        for (method, uri, body) in routes {
            for (who, headers) in &credentials {
                for router in [&w.router, &through_door] {
                    let mut req = Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("content-type", "application/json");
                    for (k, v) in headers {
                        req = req.header(*k, *v);
                    }
                    let res = router
                        .clone()
                        .oneshot(req.body(Body::from(body.to_string())).unwrap())
                        .await
                        .unwrap();
                    let status = res.status();
                    assert!(
                        [401, 403, 404].contains(&status.as_u16()),
                        "{who} reached {method} {uri}: {status}"
                    );
                }
            }
        }
        // The owner's own token is let in (a different answer than a refusal).
        let owner = format!("Bearer {WEB_TOKEN}");
        let req = Request::builder()
            .method("POST")
            .uri("/api/note/trash/empty")
            .header("authorization", owner)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(w.router.clone().oneshot(req).await.unwrap().status(), 200);
    }

    /// The owner's Trash controls and earlier-text routes are for the owner's apps only: nothing
    /// in the tools or the MCP gateway names them, so no model or helper can ask for them.
    #[test]
    fn no_tool_or_mcp_route_reaches_the_owner_only_routes() {
        const ROUTES: [&str; 4] = [
            "note/trash/purge",
            "note/trash/empty",
            "note/restore-version",
            "api/note/version",
        ];
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for krate in ["hq-tools/src", "hq-mcp/src"] {
            let mut stack = vec![crates.join(krate)];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|e| e == "rs") {
                        let text = std::fs::read_to_string(&path).unwrap();
                        for route in ROUTES {
                            assert!(!text.contains(route), "{} names {route}", path.display());
                        }
                    }
                }
            }
        }
    }
}
