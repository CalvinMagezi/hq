//! Note ids (web wave 4 W22, resolutions C-10): `note_id:` in a note's frontmatter, so a copied link
//! (`/notes/n/{id}`, `hq://notes/n/{id}`) still opens the note after it moves, in HQ or outside it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::WsState;
use crate::error::ApiError;
use crate::vault_api::{index_web_note, resolve_note_path, split_frontmatter};

const ID_KEY: &str = "note_id";
const ID_LEN: usize = 16;
/// Frontmatter sits at the top, so the lookup reads only this much of each note.
const HEAD_BYTES: u64 = 8 * 1024;
const NOTE_EXT: &str = "md";
/// A trashed note counts as deleted, so its old link shows the gone page.
const SKIP_DIRS: &[&str] = &["node_modules", "_trash"];

/// Ids HQ mints are lowercase hex; anything else in the field is not one of ours.
fn valid_id(id: &str) -> bool {
    id.len() == ID_LEN && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..ID_LEN].to_string()
}

/// The note's id, when its frontmatter has one HQ could have minted.
pub(crate) fn id_in(raw: &str) -> Option<&str> {
    let (fm, _) = split_frontmatter(raw);
    fm?.lines()
        .find(|l| is_id_line(l))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v)
        .map(|v| v.trim().trim_matches(['\'', '"']))
        .filter(|v| valid_id(v))
}

/// `raw` with `note_id: {id}` as its frontmatter's id line: an existing `note_id:` line HQ cannot
/// read is replaced (two would make the frontmatter unreadable), otherwise the line goes first.
/// Every other byte stays as it was.
fn with_id(raw: &str, id: &str) -> String {
    let line = format!("{ID_KEY}: {id}");
    let Some(fm) = split_frontmatter(raw).0 else {
        return format!("---\n{line}\n---\n{raw}");
    };
    let open = raw.find('\n').map_or(raw.len(), |i| i + 1);
    let mut offset = open;
    for existing in fm.split_inclusive('\n') {
        if is_id_line(existing) {
            let end = offset + existing.trim_end_matches(['\r', '\n']).len();
            return format!("{}{line}{}", &raw[..offset], &raw[end..]);
        }
        offset += existing.len();
    }
    format!("{}{line}\n{}", &raw[..open], &raw[open..])
}

fn is_id_line(line: &str) -> bool {
    line.strip_prefix(ID_KEY)
        .is_some_and(|rest| rest.trim_start().starts_with(':'))
}

fn read_head(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Walks the notes for the one carrying `id`. Never follows symlinks and skips hidden folders;
/// Lite also skips HQ's own (`_system` and the like).
// ponytail: a linear scan of note heads per lookup; an id-to-path table kept by the vault watcher
// if large vaults make opening a link slow.
fn find_by_id(dir: &Path, id: &str, lite: bool) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        if kind.is_dir() {
            if name.starts_with('.')
                || SKIP_DIRS.contains(&name.as_ref())
                || (lite && name.starts_with('_'))
            {
                continue;
            }
            if let Some(hit) = find_by_id(&path, id, lite) {
                return Some(hit);
            }
        } else if kind.is_file()
            && path.extension().is_some_and(|e| e == NOTE_EXT)
            && read_head(&path).is_some_and(|head| id_in(&head) == Some(id))
        {
            return Some(path);
        }
    }
    None
}

/// `GET /api/note/by-id/{id}`: `{path}` of the note with that id. An unknown id and one Lite
/// keeps out of the notes answer the same 404.
pub(crate) async fn note_by_id_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    if !valid_id(&id) {
        return ApiError::not_found().into_response();
    }
    let vault = state.vault_path.clone();
    let lite = state.profile().is_lite();
    let found = tokio::task::spawn_blocking(move || find_by_id(&vault, &id, lite)).await;
    let Ok(Some(abs)) = found else {
        return ApiError::not_found().into_response();
    };
    let rel = abs
        .strip_prefix(&state.vault_path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if rel.is_empty() || !state.path_allowed(&rel) || !state.resolved_allowed(&abs) {
        return ApiError::not_found().into_response();
    }
    Json(json!({ "path": rel })).into_response()
}

/// `POST /api/note/link {path}`: the note's id, minted into its frontmatter the first time.
/// Answers `{id, path, hash}`; `hash` is the version on disk afterwards, the editor's next `base`.
/// Adding the id is not the owner writing the note, so it is not recorded as their version.
pub(crate) async fn note_link_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let asked = body.get("path").and_then(Value::as_str).unwrap_or("");
    if !state.path_allowed(asked) {
        return ApiError::not_found().into_response();
    }
    let Some((abs, rel)) = resolve_note_path(&state.vault_path, asked) else {
        return ApiError::not_found().into_response();
    };
    if !state.resolved_allowed(&abs) {
        return ApiError::not_found().into_response();
    }
    if abs.extension().is_none_or(|e| e != NOTE_EXT) {
        return ApiError::bad_request("only notes have a link").into_response();
    }
    let raw = match std::fs::read_to_string(&abs) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::not_found().into_response();
        }
        Err(e) => return ApiError::internal(e).into_response(),
    };
    if let Some(id) = id_in(&raw) {
        let hash = hq_db::search::content_hash(&raw);
        return Json(json!({ "id": id, "path": rel, "hash": hash })).into_response();
    }
    let id = new_id();
    let updated = with_id(&raw, &id);
    if let Err(e) = hq_vault::notes::write_atomic(&abs, updated.as_bytes()) {
        return ApiError::internal(e).into_response();
    }
    index_web_note(&state, &abs, &updated);
    let hash = hq_db::search::content_hash(&updated);
    let before = hq_db::search::content_hash(&raw);
    let carried = state
        .db
        .with_conn(|c| hq_db::provenance::carry_version(c, &rel, &before, &hash));
    if let Err(e) = carried {
        tracing::warn!("notes: could not carry the note's record to its linked version: {e:#}");
    }
    Json(json!({ "id": id, "path": rel, "hash": hash })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::Profile;

    fn state(vault: &Path, profile: Profile) -> Arc<WsState> {
        Arc::new(WsState::new(vault.to_path_buf(), None).with_profile(profile))
    }

    async fn body_of(res: Response) -> (axum::http::StatusCode, Value) {
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn link(state: &Arc<WsState>, path: &str) -> (axum::http::StatusCode, Value) {
        body_of(note_link_handler(State(state.clone()), Json(json!({ "path": path }))).await).await
    }

    async fn by_id(state: &Arc<WsState>, id: &str) -> (axum::http::StatusCode, Value) {
        body_of(note_by_id_handler(State(state.clone()), AxumPath(id.to_string())).await).await
    }

    #[test]
    fn the_id_goes_first_and_leaves_every_other_byte_alone() {
        let raw = "---\n# owner's comment\ntitle: Plan\ntags: [a, b]\n---\nBody\n";
        assert_eq!(
            with_id(raw, "0123456789abcdef"),
            "---\nnote_id: 0123456789abcdef\n# owner's comment\ntitle: Plan\ntags: [a, b]\n---\nBody\n"
        );
        assert_eq!(
            with_id("Just text", "0123456789abcdef"),
            "---\nnote_id: 0123456789abcdef\n---\nJust text"
        );
        assert_eq!(
            id_in(&with_id(raw, "0123456789abcdef")),
            Some("0123456789abcdef")
        );
        assert_eq!(
            id_in("---\nnote_id: \"0123456789abcdef\"\n---\n"),
            Some("0123456789abcdef")
        );
        assert_eq!(id_in("---\nnote_id: ../etc\n---\n"), None);
        assert_eq!(id_in("note_id: 0123456789abcdef\n"), None);
        for odd in ["12345", "ABCDEF0123456789"] {
            let raw = format!("---\ntitle: A\nnote_id: {odd}\n---\nB");
            assert_eq!(
                with_id(&raw, "0123456789abcdef"),
                "---\ntitle: A\nnote_id: 0123456789abcdef\n---\nB",
                "an id HQ cannot read is replaced, never doubled"
            );
        }
        assert_eq!(
            with_id(
                "---\r\nnote_id: x\r\ntitle: A\r\n---\r\nB",
                "0123456789abcdef"
            ),
            "---\r\nnote_id: 0123456789abcdef\r\ntitle: A\r\n---\r\nB"
        );
    }

    #[tokio::test]
    async fn adding_the_id_keeps_who_wrote_the_note() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        let raw = "---\npinned: true\n---\nAgent text\n";
        std::fs::write(vault.path().join("Notebooks/A.md"), raw).unwrap();
        let st = state(vault.path(), Profile::Full);
        let agent = hq_db::provenance::Attribution {
            actor_id: Some("hs-1"),
            source_kind: Some(hq_db::provenance::SourceKind::Agent),
            ..Default::default()
        };
        st.db
            .with_conn(|c| {
                hq_db::provenance::record(
                    c,
                    "Notebooks/A.md",
                    &hq_db::search::content_hash(raw),
                    &agent,
                )
            })
            .unwrap();
        assert_eq!(
            link(&st, "Notebooks/A.md").await.0,
            axum::http::StatusCode::OK
        );
        let now = std::fs::read_to_string(vault.path().join("Notebooks/A.md")).unwrap();
        let suggested = st
            .db
            .with_conn(|c| hq_db::provenance::pin_suggested_by(c, "Notebooks/A.md", &now))
            .unwrap();
        assert_eq!(
            suggested.as_deref(),
            Some("hs-1"),
            "still the agent's suggestion"
        );
    }

    #[tokio::test]
    async fn a_copied_link_opens_the_note_after_it_moves() {
        let vault = tempfile::TempDir::new().unwrap();
        let dir = vault.path().join("Notebooks/Projects");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Plan.md"), "---\ntitle: Plan\n---\nBody\n").unwrap();
        let st = state(vault.path(), Profile::Full);

        let (status, first) = link(&st, "Notebooks/Projects/Plan.md").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let id = first["id"].as_str().unwrap().to_string();
        let on_disk = std::fs::read_to_string(dir.join("Plan.md")).unwrap();
        assert_eq!(first["hash"], hq_db::search::content_hash(&on_disk));
        let (_, again) = link(&st, "Notebooks/Projects/Plan.md").await;
        assert_eq!(again["id"], id.as_str(), "a second copy keeps the same id");

        std::fs::create_dir_all(vault.path().join("Notebooks/Archive")).unwrap();
        std::fs::rename(
            dir.join("Plan.md"),
            vault.path().join("Notebooks/Archive/Plan.md"),
        )
        .unwrap();
        let (status, found) = by_id(&st, &id).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(found["path"], "Notebooks/Archive/Plan.md");

        assert_eq!(
            by_id(&st, "ffffffffffffffff").await.0,
            axum::http::StatusCode::NOT_FOUND
        );
        assert_eq!(by_id(&st, "..").await.0, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn lite_never_links_or_finds_a_note_in_hqs_own_folders() {
        let vault = tempfile::TempDir::new().unwrap();
        let system = vault.path().join("_system");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(
            system.join("Soul.md"),
            "---\nnote_id: 0123456789abcdef\n---\nx",
        )
        .unwrap();
        let full = state(vault.path(), Profile::Full);
        assert_eq!(
            by_id(&full, "0123456789abcdef").await.0,
            axum::http::StatusCode::OK
        );

        let lite = state(vault.path(), Profile::Lite);
        assert_eq!(
            by_id(&lite, "0123456789abcdef").await.0,
            axum::http::StatusCode::NOT_FOUND
        );
        assert_eq!(
            link(&lite, "_system/Soul.md").await.0,
            axum::http::StatusCode::NOT_FOUND
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_inside_the_notes_never_leads_lite_into_hqs_folders() {
        let vault = tempfile::TempDir::new().unwrap();
        let system = vault.path().join("_system");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(system.join("Soul.md"), "x").unwrap();
        std::os::unix::fs::symlink(
            system.join("Soul.md"),
            vault.path().join("Notebooks/Soul.md"),
        )
        .unwrap();
        let lite = state(vault.path(), Profile::Lite);
        assert_eq!(
            link(&lite, "Notebooks/Soul.md").await.0,
            axum::http::StatusCode::NOT_FOUND
        );
        assert_eq!(
            std::fs::read_to_string(system.join("Soul.md")).unwrap(),
            "x"
        );
    }
}
