//! Putting back the text a note had before HQ changed it. The kept text is the owner's version
//! from before HQ's latest change (`hq_db::note_keep`); the versions list gives the ids.

use crate::WsState;
use crate::error::ApiError;
use crate::vault_api::{
    file_hash, index_web_note, is_rejected_note_ref, record_owner_write, resolve_note_path,
    split_frontmatter,
};
use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use hq_db::provenance::SourceKind;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Deserialize)]
pub(crate) struct VersionQuery {
    path: String,
    id: i64,
}

/// A note the owner may read, as a vault path that is a `.md` file.
fn checked_note(state: &WsState, requested: &str) -> Result<(PathBuf, String), ApiError> {
    let requested = requested.trim();
    let requested = requested.strip_prefix("./").unwrap_or(requested);
    if is_rejected_note_ref(requested) || requested.contains('\\') {
        return Err(ApiError::bad_request("invalid path"));
    }
    if !state.path_allowed(requested) {
        return Err(ApiError::not_found());
    }
    let (abs, effective) =
        resolve_note_path(&state.vault_path, requested).ok_or_else(ApiError::not_found)?;
    if !effective.ends_with(".md")
        || !state.path_allowed(&effective)
        || !state.resolved_allowed(&abs)
        || !abs.is_file()
    {
        return Err(ApiError::not_found());
    }
    Ok((abs, effective))
}

/// How many locks the notes share. A path always maps to the same one, so two writers of one note
/// meet; unrelated notes may share a lock, which only makes them wait a moment.
const NOTE_LOCKS: usize = 32;

/// Held while a note is read, compared and written, so nothing slips in between those steps.
/// Both the owner's save and the restore take it.
pub(crate) fn lock_note(rel: &str) -> std::sync::MutexGuard<'static, ()> {
    use std::hash::{Hash, Hasher};
    static LOCKS: [std::sync::Mutex<()>; NOTE_LOCKS] =
        [const { std::sync::Mutex::new(()) }; NOTE_LOCKS];
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // `a//b` and `./a/b` name the same file, so they must pick the same lock.
    for part in rel.split('/').filter(|p| !p.is_empty() && *p != ".") {
        part.hash(&mut hasher);
    }
    let slot = (hasher.finish() % NOTE_LOCKS as u64) as usize;
    LOCKS[slot]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// HQ's version `id` of `path` and the earlier text it replaced. `None` unless `id` is a version
/// of this note and either HQ's latest change (which the kept text was armed for) or, when
/// `latest_change_only` is off, the kept text's own version. Anything else is not found, never
/// another text.
fn earlier_text(
    state: &WsState,
    rel: &str,
    id: i64,
    latest_change_only: bool,
) -> Result<(hq_db::note_keep::Prev, hq_db::provenance::Provenance), ApiError> {
    let found = state
        .db
        .with_conn(|c| {
            let Some(version) = hq_db::provenance::get(c, id)?.filter(|p| p.path == rel) else {
                return Ok(None);
            };
            let Some(prev) = hq_db::note_keep::get(c, rel)? else {
                return Ok(None);
            };
            let is_ours = version.source_kind == SourceKind::Agent
                && prev.for_hash.as_deref() == Some(version.content_hash.as_str());
            let is_the_text = !latest_change_only && version.content_hash == prev.text_hash;
            Ok((is_ours || is_the_text).then_some((prev, version)))
        })
        .map_err(ApiError::internal)?;
    found.ok_or_else(ApiError::not_found)
}

/// `GET /api/note/version?path=&id=`: the text a note had before HQ's change `id`. Owner-only,
/// not a tool.
pub(crate) async fn note_version_handler(
    State(state): State<Arc<WsState>>,
    Query(query): Query<VersionQuery>,
) -> Response {
    let (_, rel) = match checked_note(&state, &query.path) {
        Ok(found) => found,
        Err(e) => return e.into_response(),
    };
    match earlier_text(&state, &rel, query.id, false) {
        Ok((prev, _)) => Json(json!({
            "id": query.id,
            "path": rel,
            "content": split_frontmatter(&prev.text).1,
            "hash": prev.text_hash,
            "saved_at": prev.saved_at,
        }))
        .into_response(),
        Err(e) => e.into_response(),
    }
}

/// `POST /api/note/restore-version {path, id}`: write that earlier text back as the owner, which
/// clears the `changed` mark, and answer the new hash. Owner-only, not a tool.
pub(crate) async fn restore_version_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let path = body.get("path").and_then(Value::as_str).unwrap_or("");
    let Some(id) = body.get("id").and_then(Value::as_i64) else {
        return ApiError::bad_request("id must be a number").into_response();
    };
    let (abs, rel) = match checked_note(&state, path) {
        Ok(found) => found,
        Err(e) => return e.into_response(),
    };
    let worker = state.clone();
    let done = tokio::task::spawn_blocking(move || {
        let _held = lock_note(&rel);
        let (prev, version) = earlier_text(&worker, &rel, id, true)?;
        // Only HQ's change that is still the note is rolled back; a newer edit is never lost.
        let current = file_hash(&worker.vault_path, &rel);
        if current.as_deref() != Some(version.content_hash.as_str()) {
            return Err(ApiError::Conflict(CHANGED_SINCE.to_string()));
        }
        hq_vault::notes::write_atomic(&abs, prev.text.as_bytes()).map_err(ApiError::internal)?;
        index_web_note(&worker, &abs, &prev.text);
        record_owner_write(&worker, &abs);
        Ok(file_hash(&worker.vault_path, &rel))
    })
    .await;
    match done {
        Ok(Ok(hash)) => Json(json!({"success": true, "hash": hash})).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

const CHANGED_SINCE: &str = "The note changed since HQ's edit, so it was not rolled back.";

#[cfg(test)]
mod tests {
    use crate::{WsState, create_router};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use hq_tools::provenance::Recorder;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    const BODY_LIMIT: usize = 1 << 20;

    #[test]
    fn spellings_of_one_path_share_a_lock() {
        let _held = super::lock_note("Notebooks/Inbox/a.md");
        let (tx, rx) = std::sync::mpsc::channel();
        let other = std::thread::spawn(move || {
            let _again = super::lock_note("./Notebooks//Inbox/a.md");
            tx.send(()).unwrap();
        });
        // The second spelling must wait for the first; if it got another lock it would not.
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err()
        );
        drop(_held);
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        other.join().unwrap();
    }
    const REL: &str = "Notebooks/Inbox/a.md";
    const OWNER_TEXT: &str = "---\ntitle: A\n---\nthe owner wrote this";

    fn setup() -> (tempfile::TempDir, Arc<WsState>, axum::Router) {
        let v = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(v.path().join("Notebooks/Inbox")).unwrap();
        std::fs::write(v.path().join(REL), OWNER_TEXT).unwrap();
        let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
        let app = create_router(state.clone());
        crate::vault_api::record_owner_write(&state, &v.path().join(REL));
        (v, state, app)
    }

    fn helper_args() -> Value {
        json!({ "_hq_native_session": "s1", "_hq_native_agent": "helper" })
    }

    /// What a write tool does: keep the old text, write, record. Returns the id of HQ's version.
    fn hq_edits(v: &tempfile::TempDir, state: &WsState, text: &str) -> i64 {
        let recorder = Recorder::new(Arc::new(state.db.clone()), v.path().to_path_buf());
        recorder.before_write(REL, &helper_args());
        std::fs::write(v.path().join(REL), text).unwrap();
        recorder.after_write(REL, &helper_args());
        let hash = hq_db::search::content_hash(text);
        state
            .db
            .with_conn(|c| hq_db::provenance::current(c, REL, &hash))
            .unwrap()
            .unwrap()
            .id
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

    fn restore_body(id: i64) -> String {
        json!({ "path": REL, "id": id }).to_string()
    }

    #[tokio::test]
    async fn an_hq_edit_can_be_undone_twice_and_the_changed_mark_clears() {
        let (v, state, app) = setup();
        let id = hq_edits(&v, &state, "---\ntitle: A\n---\nHQ rewrote it");
        let (_, note) = call(&app, "GET", &format!("/api/note?path={REL}"), "").await;
        assert_eq!(note["review"], "changed");

        let (status, earlier) = call(
            &app,
            "GET",
            &format!("/api/note/version?path={REL}&id={id}"),
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(earlier["content"], "the owner wrote this");

        let (status, done) =
            call(&app, "POST", "/api/note/restore-version", &restore_body(id)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            std::fs::read_to_string(v.path().join(REL)).unwrap(),
            OWNER_TEXT
        );
        assert_eq!(done["hash"], hq_db::search::content_hash(OWNER_TEXT));
        let (_, note) = call(&app, "GET", &format!("/api/note?path={REL}"), "").await;
        assert_eq!(note["review"], Value::Null);
        assert_eq!(note["written_by"], "you");

        // The owner's write took the kept text with it, so a second restore finds nothing.
        let again = call(&app, "POST", "/api/note/restore-version", &restore_body(id)).await;
        assert_eq!(again.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_owner_edit_after_the_hq_edit_is_never_rolled_back() {
        let (v, state, app) = setup();
        let id = hq_edits(&v, &state, "---\ntitle: A\n---\nHQ rewrote it");
        // The owner saves a newer version through the app.
        let save = json!({ "path": REL, "content": "the owner's newer words" }).to_string();
        assert_eq!(
            call(&app, "PUT", "/api/note", &save).await.0,
            StatusCode::OK
        );
        let before = std::fs::read_to_string(v.path().join(REL)).unwrap();
        let (status, refused) =
            call(&app, "POST", "/api/note/restore-version", &restore_body(id)).await;
        assert!(
            status == StatusCode::CONFLICT || status == StatusCode::NOT_FOUND,
            "{status}"
        );
        assert!(refused["error"].is_string());
        assert_eq!(std::fs::read_to_string(v.path().join(REL)).unwrap(), before);

        // Even if the kept text were still there, a file that is not HQ's change is refused.
        state
            .db
            .with_conn(|c| {
                hq_db::note_keep::keep(c, REL, "stale")?;
                hq_db::note_keep::tie_or_drop(
                    c,
                    REL,
                    &hq_db::provenance::history(c, REL, 10)?
                        .iter()
                        .find(|p| p.id == id)
                        .unwrap()
                        .content_hash,
                )
            })
            .unwrap();
        let (status, refused) =
            call(&app, "POST", "/api/note/restore-version", &restore_body(id)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            refused["error"],
            "The note changed since HQ's edit, so it was not rolled back."
        );
        assert_eq!(std::fs::read_to_string(v.path().join(REL)).unwrap(), before);
    }

    #[tokio::test]
    async fn an_hq_write_that_did_not_keep_serves_no_stale_text() {
        let (v, state, app) = setup();
        let first = hq_edits(&v, &state, "---\ntitle: A\n---\nfirst");
        // A later HQ write through a path that never keeps (no `before_write`).
        let recorder = Recorder::new(Arc::new(state.db.clone()), v.path().to_path_buf());
        std::fs::write(v.path().join(REL), "---\ntitle: A\n---\nsecond").unwrap();
        recorder.after_write(REL, &helper_args());
        let hash = hq_db::search::content_hash("---\ntitle: A\n---\nsecond");
        let second = state
            .db
            .with_conn(|c| hq_db::provenance::current(c, REL, &hash))
            .unwrap()
            .unwrap()
            .id;
        for id in [first, second] {
            let uri = format!("/api/note/version?path={REL}&id={id}");
            assert_eq!(call(&app, "GET", &uri, "").await.0, StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn the_kept_text_is_the_owners_even_after_a_second_hq_edit() {
        let (v, state, app) = setup();
        let first = hq_edits(&v, &state, "---\ntitle: A\n---\nfirst HQ edit");
        let second = hq_edits(&v, &state, "---\ntitle: A\n---\nsecond HQ edit");
        let uri = |id: i64| format!("/api/note/version?path={REL}&id={id}");
        let (status, earlier) = call(&app, "GET", &uri(second), "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(earlier["content"], "the owner wrote this");
        // Only the latest HQ change can be undone.
        assert_eq!(
            call(&app, "GET", &uri(first), "").await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn a_missing_or_foreign_id_is_not_found() {
        let (v, state, app) = setup();
        let id = hq_edits(&v, &state, "---\ntitle: A\n---\nHQ rewrote it");
        for bad in [id + 1000, 0, -1] {
            let uri = format!("/api/note/version?path={REL}&id={bad}");
            assert_eq!(call(&app, "GET", &uri, "").await.0, StatusCode::NOT_FOUND);
            let restore = call(
                &app,
                "POST",
                "/api/note/restore-version",
                &restore_body(bad),
            )
            .await;
            assert_eq!(restore.0, StatusCode::NOT_FOUND, "{bad}");
        }
        // An id that belongs to another note is not this note's version.
        std::fs::write(v.path().join("Notebooks/Inbox/b.md"), "b").unwrap();
        crate::vault_api::record_owner_write(&state, &v.path().join("Notebooks/Inbox/b.md"));
        let other = state
            .db
            .with_conn(|c| {
                hq_db::provenance::current(
                    c,
                    "Notebooks/Inbox/b.md",
                    &hq_db::search::content_hash("b"),
                )
            })
            .unwrap()
            .unwrap()
            .id;
        let uri = format!("/api/note/version?path={REL}&id={other}");
        assert_eq!(call(&app, "GET", &uri, "").await.0, StatusCode::NOT_FOUND);
        assert_eq!(
            std::fs::read_to_string(v.path().join(REL)).unwrap(),
            "---\ntitle: A\n---\nHQ rewrote it"
        );
    }

    #[tokio::test]
    async fn a_note_over_the_cap_keeps_no_earlier_text() {
        let (v, state, app) = setup();
        let big = format!(
            "{OWNER_TEXT}\n{}",
            "x".repeat(hq_db::note_keep::MAX_PREV_BYTES)
        );
        std::fs::write(v.path().join(REL), &big).unwrap();
        crate::vault_api::record_owner_write(&state, &v.path().join(REL));
        let id = hq_edits(&v, &state, "small now");
        let uri = format!("/api/note/version?path={REL}&id={id}");
        assert_eq!(call(&app, "GET", &uri, "").await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn keeping_the_change_or_deleting_the_note_drops_the_earlier_text() {
        let (v, state, app) = setup();
        let id = hq_edits(&v, &state, "---\ntitle: A\n---\nHQ rewrote it");
        let keep = json!({ "path": REL, "verdict": "accepted" }).to_string();
        assert_eq!(
            call(&app, "POST", "/api/note/review", &keep).await.0,
            StatusCode::OK
        );
        let uri = format!("/api/note/version?path={REL}&id={id}");
        assert_eq!(call(&app, "GET", &uri, "").await.0, StatusCode::NOT_FOUND);

        let id = hq_edits(&v, &state, "---\ntitle: A\n---\nHQ again");
        assert_eq!(
            call(
                &app,
                "GET",
                &format!("/api/note/version?path={REL}&id={id}"),
                ""
            )
            .await
            .0,
            StatusCode::OK
        );
        assert_eq!(
            call(&app, "DELETE", &format!("/api/note?path={REL}"), "")
                .await
                .0,
            StatusCode::OK
        );
        let left = state
            .db
            .with_conn(|c| hq_db::note_keep::get(c, REL))
            .unwrap();
        assert!(left.is_none());
    }

    #[tokio::test]
    async fn the_routes_refuse_bad_paths() {
        let (_v, _state, app) = setup();
        for bad in ["../x.md", "/etc/hosts", "a%5Cb.md"] {
            let uri = format!("/api/note/version?path={bad}&id=1");
            assert_eq!(
                call(&app, "GET", &uri, "").await.0,
                StatusCode::BAD_REQUEST,
                "{bad}"
            );
        }
        let body = json!({ "path": "../x.md", "id": 1 }).to_string();
        assert_eq!(
            call(&app, "POST", "/api/note/restore-version", &body)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let no_id = json!({ "path": REL }).to_string();
        assert_eq!(
            call(&app, "POST", "/api/note/restore-version", &no_id)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}
