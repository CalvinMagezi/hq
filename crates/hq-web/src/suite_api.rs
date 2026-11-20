//! `/api/suite/*`: the HQ Suite for the apps. One file's state and the four things a person can do
//! with it (make it smaller, agree, say no, undo), plus what the suite holds. The routes sit behind
//! the web token or a device token like every `/api` route, and an MCP key never reaches them.
//! The logic is `hq-suite-adapter::api`; this file only carries requests to it.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use hq_core::config::HqConfig;
use serde::Deserialize;
use tokio::sync::Semaphore;

use crate::WsState;
use crate::error::ApiError;

/// Shrinks at once. Each decodes a whole document and its pictures, so a few at a time is what a
/// phone can carry; the rest wait their turn.
const CONCURRENT_SHRINKS: usize = 2;
static SHRINK_SLOTS: Semaphore = Semaphore::const_new(CONCURRENT_SHRINKS);

pub(crate) fn routes() -> Router<Arc<WsState>> {
    Router::new()
        .route("/api/suite/file", get(file_handler))
        .route("/api/suite/preview", get(preview_handler))
        .route("/api/suite/export", post(export_handler))
        .route("/api/suite/export-formats", get(export_formats_handler))
        .route("/api/suite/shrink", post(shrink_handler))
        .route("/api/suite/confirm", post(confirm_handler))
        .route("/api/suite/offload", post(offload_handler))
        .route("/api/suite/abandon", post(abandon_handler))
        .route("/api/suite/undo", post(undo_handler))
        .route("/api/suite/storage", get(storage_handler))
        .route("/api/suite/sinks", get(sinks_handler))
        .route("/api/suite/sinks/peer", post(sink_peer_handler))
        .route("/api/suite/sinks/check", post(sink_check_handler))
        .route("/api/suite/media", get(media_handler))
        .route("/api/suite/sheet", get(sheet_handler))
        .route("/api/suite/tile", get(tile_handler))
        .route("/api/suite/blocks", get(blocks_handler))
        .route("/api/suite/blocks/edit", post(block_edit_handler))
        .route("/api/suite/blocks/insert", post(block_insert_handler))
        .route("/api/suite/blocks/delete", post(block_delete_handler))
        .route("/api/suite/plot", get(plot_handler))
        .route("/api/suite/sheet/cell", post(sheet_cell_handler))
        .route("/api/suite/cards", get(cards_handler))
        .route("/api/suite/edit", post(edit_handler))
}

#[derive(Deserialize)]
pub(crate) struct PathQuery {
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct SheetQuery {
    path: String,
    /// Which `sheet` block of the note (zero-based, default the first).
    index: Option<usize>,
    row: Option<u32>,
    col: Option<u32>,
}

#[derive(Deserialize)]
pub(crate) struct CellEdit {
    path: String,
    index: usize,
    row: u32,
    col: u32,
    /// What the cell held when the person opened it; the edit is refused if it holds something else now.
    expected: String,
    text: String,
}

#[derive(Deserialize)]
pub(crate) struct SinkPeerBody {
    id: String,
    addr: String,
}

#[derive(Deserialize)]
pub(crate) struct SinkCheckBody {
    id: String,
    /// The recovery code, typed by the person for this one call; nothing keeps it.
    code: String,
    sample: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct BlockBody {
    path: String,
    /// The block's place and the hash the phone saw; the edit is refused if the block no longer hashes to it.
    index: Option<usize>,
    hash: Option<String>,
    markdown: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PathBody {
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct JobBody {
    job: String,
}

/// A refusal or failure comes back as a sentence the app can show.
fn refused(e: anyhow::Error) -> ApiError {
    ApiError::unprocessable(format!("{e:#}"))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(ApiError::internal)?
        .map_err(refused)
}

pub(crate) async fn file_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<PathQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let status = blocking(move || hq_suite_adapter::api::status(&vault, &q.path)).await?;
    Ok(Json(status))
}

#[derive(Deserialize)]
pub(crate) struct ExportBody {
    path: String,
    format: String,
}

/// The formats an Export sheet offers for a note, by the note's own kind.
pub(crate) async fn export_formats_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<PathQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.path_allowed(&q.path) {
        return Err(ApiError::not_found());
    }
    let p = hq_suite_adapter::preview::preview(&state.vault_path, &q.path)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let formats = if p.suite {
        hq_tools_office::author::export_formats(&p.kind)
    } else {
        &[]
    };
    Ok(Json(
        serde_json::json!({ "kind": p.kind, "formats": formats }),
    ))
}

/// Renders a note as a file under `Exports/`, with the same code the agent's `office_export` runs.
pub(crate) async fn export_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<ExportBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.path_allowed(&b.path) || !b.path.ends_with(".md") {
        return Err(ApiError::not_found());
    }
    let p = hq_suite_adapter::preview::preview(&state.vault_path, &b.path)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    if b.format == "md" {
        return Ok(Json(serde_json::json!({
            "files": [hq_tools::deliver::file_entry(&state.vault_path, &b.path)],
            "web_path": b.path,
            "fidelity": "The note as it is written, with its blocks.",
        })));
    }
    if !hq_tools_office::author::export_formats(&p.kind).contains(&b.format.as_str()) {
        return Err(ApiError::bad_request(format!(
            "a {} cannot be exported as {}",
            p.kind, b.format
        )));
    }
    let out = hq_tools_office::author::export_to_exports(&state.vault_path, &b.path, &b.format)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(out))
}

/// A note with `sheet` blocks exports as the suite's own workbook (real formulas, typed columns),
/// the same bytes the agent's `office_export` makes, and says in `X-Export-Note` what was kept.
pub(crate) fn sheet_export_response(
    abs: &std::path::Path,
    format: hq_export::Format,
) -> Option<axum::response::Response> {
    let text = std::fs::read_to_string(abs).ok()?;
    let (ext, mime) = match format {
        hq_export::Format::Csv => ("csv", "text/csv"),
        _ => (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
    };
    let made = hq_suite_adapter::author::sheet_bytes(&text, ext)?;
    let (bytes, kept) = match made {
        Ok(ok) => ok,
        Err(e) => return Some(ApiError::bad_request(e.to_string()).into_response()),
    };
    let stem = abs
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sheet".into());
    let ascii: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let disposition = format!("attachment; filename=\"{ascii}.{ext}\"");
    let mut res = (
        axum::http::StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, mime.to_string()),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
            (axum::http::header::CACHE_CONTROL, "no-store".to_string()),
        ],
        axum::body::Body::from(bytes),
    )
        .into_response();
    if let Ok(v) = axum::http::HeaderValue::from_str(&kept) {
        res.headers_mut().insert("x-export-note", v);
    }
    Some(res)
}

/// One record view of a note or an office file, for every app to draw (content, not layout).
pub(crate) async fn preview_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<PathQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.path_allowed(&q.path) {
        return Err(ApiError::not_found());
    }
    let p = hq_suite_adapter::preview::preview(&state.vault_path, &q.path)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(p))
}

pub(crate) async fn shrink_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<PathBody>,
) -> Result<impl IntoResponse, ApiError> {
    // Detached: a phone that gives up waiting must not cancel a shrink halfway, which would leave
    // pictures stored without a job and make the retry's saving look larger than it is.
    let vault = state.vault_path.clone();
    let status = tokio::spawn(async move {
        let _slot = SHRINK_SLOTS.acquire().await;
        hq_suite_adapter::api::prepare(&vault, &b.path).await
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(refused)?;
    Ok(Json(status))
}

pub(crate) async fn confirm_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<JobBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    Ok(Json(
        blocking(move || hq_suite_adapter::api::confirm(&vault, &b.job)).await?,
    ))
}

#[derive(Deserialize)]
pub(crate) struct OffloadBody {
    job: String,
    sink: String,
    code: String,
}

pub(crate) async fn offload_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<OffloadBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    Ok(Json(
        blocking(move || hq_suite_adapter::api::offload(&vault, &b.job, &b.sink, &b.code)).await?,
    ))
}

pub(crate) async fn abandon_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<JobBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    Ok(Json(
        blocking(move || hq_suite_adapter::api::abandon(&vault, &b.job)).await?,
    ))
}

#[derive(Deserialize)]
pub(crate) struct UndoBody {
    job: String,
    /// The sink's recovery code, for an original sealed away from the phone.
    code: Option<String>,
}

pub(crate) async fn undo_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<UndoBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    Ok(Json(
        blocking(move || match &b.code {
            Some(code) => hq_suite_adapter::api::undo_from_sink(&vault, &b.job, code),
            None => hq_suite_adapter::api::undo(&vault, &b.job),
        })
        .await?,
    ))
}

pub(crate) async fn media_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<PathQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let (bytes, kind) = blocking(move || hq_suite_adapter::api::media(&vault, &q.path)).await?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, kind),
            // Stored by hash: the bytes behind a path never change.
            (
                axum::http::header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        bytes,
    ))
}

pub(crate) async fn sheet_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<SheetQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let view = blocking(move || {
        hq_suite_adapter::sheets::view(
            &vault,
            &q.path,
            q.index.unwrap_or(0),
            q.row.unwrap_or(0),
            q.col.unwrap_or(0),
        )
    })
    .await?;
    Ok(Json(view))
}

pub(crate) async fn sheet_cell_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<CellEdit>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let path = b.path.clone();
    let view = blocking(move || {
        hq_suite_adapter::sheets::edit_cell(
            &vault,
            &b.path,
            b.index,
            b.row,
            b.col,
            &b.expected,
            &b.text,
        )?;
        hq_suite_adapter::sheets::view(&vault, &b.path, b.index, 0, 0)
    })
    .await?;
    announce_written(&state, &path);
    Ok(Json(view))
}

pub(crate) async fn plot_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<SheetQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let chart =
        blocking(move || hq_suite_adapter::sheets::plot(&vault, &q.path, q.index.unwrap_or(0)))
            .await?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        chart,
    ))
}

pub(crate) async fn tile_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<SheetQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let view =
        blocking(move || hq_suite_adapter::sheets::tile(&vault, &q.path, q.index.unwrap_or(0)))
            .await?;
    Ok(Json(view))
}

pub(crate) async fn sinks_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let sinks = blocking(move || hq_suite_adapter::sinks::infos(&vault)).await?;
    Ok(Json(serde_json::json!({ "sinks": sinks })))
}

pub(crate) async fn sink_peer_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<SinkPeerBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let id = b.id.clone();
    let code = blocking(move || hq_suite_adapter::sinks::make_peer(&vault, &b.id, &b.addr)).await?;
    Ok(Json(serde_json::json!({ "id": id, "code": code })))
}

pub(crate) async fn sink_check_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<SinkCheckBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let drill = blocking(move || {
        hq_suite_adapter::sinks::drill_with_code(&vault, &b.id, &b.code, b.sample)
    })
    .await?;
    Ok(Json(serde_json::json!({
        "checked": drill.checked,
        "failed": drill.failed.len(),
        "text": hq_suite_adapter::text::drill(&drill),
    })))
}

pub(crate) async fn blocks_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<PathQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    Ok(Json(
        blocking(move || hq_suite_adapter::blocks::list(&vault, &q.path)).await?,
    ))
}

pub(crate) async fn block_edit_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<BlockBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let (Some(index), Some(hash), Some(markdown)) = (b.index, b.hash.clone(), b.markdown.clone())
    else {
        return Err(ApiError::bad_request(
            "an edit names the block, its hash and the new text",
        ));
    };
    let path = b.path.clone();
    let view =
        blocking(move || hq_suite_adapter::blocks::edit(&vault, &b.path, index, &hash, &markdown))
            .await?;
    announce_written(&state, &path);
    Ok(Json(view))
}

pub(crate) async fn block_insert_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<BlockBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let markdown = b.markdown.clone().unwrap_or_default();
    let after = b.index.zip(b.hash.clone());
    let path = b.path.clone();
    let announced = b.path.clone();
    let view = blocking(move || {
        hq_suite_adapter::blocks::insert(
            &vault,
            &path,
            after.as_ref().map(|(i, h)| (*i, h.as_str())),
            &markdown,
        )
    })
    .await?;
    announce_written(&state, &announced);
    Ok(Json(view))
}

pub(crate) async fn block_delete_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<BlockBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let (Some(index), Some(hash)) = (b.index, b.hash.clone()) else {
        return Err(ApiError::bad_request(
            "a delete names the block and its hash",
        ));
    };
    let path = b.path.clone();
    let view =
        blocking(move || hq_suite_adapter::blocks::delete(&vault, &b.path, index, &hash)).await?;
    announce_written(&state, &path);
    Ok(Json(view))
}

/// Tells open pages the note at `path` was just written, with the revision it is at now.
fn announce_written(state: &WsState, path: &str) {
    if let Some(revision) = hq_suite_adapter::edit::note_revision(&state.vault_path, path) {
        state.announce_note(path, &revision);
    }
}

/// A slides note's cards (number, hash, notes, look), for the card editor.
pub(crate) async fn cards_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<PathQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !state.path_allowed(&q.path) {
        return Err(ApiError::not_found());
    }
    let vault = state.vault_path.clone();
    let args = serde_json::json!({"action": "cards", "note": q.path});
    let out = blocking(move || {
        hq_suite_adapter::edit::run(&vault, &args, hq_suite_adapter::edit::Gate::Owner)
    })
    .await?;
    Ok(Json(out))
}

/// Every `office_edit` action for the apps: the body is the action's arguments with `path` in
/// place of `note`. The owner's edit is one step; `dry_run: true` returns what it would do and
/// writes nothing. A refusal (a stale block, a formula that reads moved rows) comes back as a
/// sentence, and nothing is written.
pub(crate) async fn edit_handler(
    State(state): State<Arc<WsState>>,
    Json(mut body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, ApiError> {
    let path = body
        .get("path")
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .ok_or_else(|| ApiError::bad_request("an edit names the note as `path`"))?;
    if !state.path_allowed(&path) || !path.ends_with(".md") {
        return Err(ApiError::not_found());
    }
    if let Some(o) = body.as_object_mut() {
        o.remove("path");
        // A token belongs to an agent's preview; the owner's call is one step.
        o.remove("token");
        // Nothing the app sends can claim to be another caller.
        o.retain(|k, _| !k.starts_with("_hq_"));
        o.insert("note".into(), serde_json::Value::String(path.clone()));
    }
    let vault = state.vault_path.clone();
    let out = blocking(move || {
        hq_suite_adapter::edit::run(&vault, &body, hq_suite_adapter::edit::Gate::Owner)
    })
    .await?;
    if out["changed"] == true
        && let Some(revision) = out["revision"].as_str()
    {
        state.announce_note(&path, revision);
    }
    Ok(Json(out))
}

pub(crate) async fn storage_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let budget =
        tokio::task::spawn_blocking(|| HqConfig::load().ok().and_then(|c| c.suite.budget_mb))
            .await
            .map_err(ApiError::internal)?;
    let text =
        blocking(move || Ok(hq_suite_adapter::governor::storage_text(&vault, budget))).await?;
    Ok(Json(serde_json::json!({ "text": text })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_json(reply: impl IntoResponse) -> serde_json::Value {
        let bytes = to_bytes(reply.into_response().into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn an_app_edit_is_one_step_and_open_pages_hear_which_note_changed() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        let note = "# S\n\n```sheet name=S\n| a | b |\n|---|---|\n| 1 | 2 |\n```\n";
        std::fs::write(vault.path().join("Notebooks/s.md"), note).unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let mut heard = state.tx.subscribe();

        let listed = body_json(
            blocks_handler(
                State(state.clone()),
                Query(PathQuery {
                    path: "Notebooks/s.md".into(),
                }),
            )
            .await
            .unwrap(),
        )
        .await;
        let hash = listed["blocks"][1]["hash"].clone();

        // A delete is one step for the owner (no token), and the page that has the note open is told.
        let done = body_json(
            edit_handler(
                State(state.clone()),
                Json(serde_json::json!({"path": "Notebooks/s.md", "action": "delete_rows",
                    "sheet": 0, "hash": hash, "row": 1, "token": "ignored", "_hq_caller_session": "x"})),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(done["changed"], true);
        assert!(
            !std::fs::read_to_string(vault.path().join("Notebooks/s.md"))
                .unwrap()
                .contains("| 1 | 2 |")
        );
        let frame: serde_json::Value = serde_json::from_str(&heard.try_recv().unwrap()).unwrap();
        assert_eq!(frame["type"], "note_changed");
        assert_eq!(frame["path"], "Notebooks/s.md");
        assert_eq!(frame["hash"], done["revision"]);

        // A dry run, a refused edit and a stale one say so and announce nothing.
        let stale = serde_json::json!({"path": "Notebooks/s.md", "action": "add_rows", "hash": hash, "rows": [["3", "4"]]});
        assert!(
            edit_handler(State(state.clone()), Json(stale))
                .await
                .is_err(),
            "the sheet changed since the hash was read"
        );
        let dry = body_json(
            edit_handler(
                State(state.clone()),
                Json(serde_json::json!({"path": "Notebooks/s.md", "action": "set_cells", "dry_run": true,
                    "row": 0, "col": 0, "values": [["z"]], "expected": [["a"]]})),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(dry["preview"], true);
        assert!(
            heard.try_recv().is_err(),
            "nothing was written, so nothing is announced"
        );

        // Only notes are edited, and a path with no note names no note.
        for bad in [
            serde_json::json!({"path": "Notebooks/s.docx", "action": "blocks"}),
            serde_json::json!({"action": "blocks"}),
        ] {
            assert!(edit_handler(State(state.clone()), Json(bad)).await.is_err());
        }
    }

    #[tokio::test]
    async fn the_existing_block_routes_announce_the_note_they_wrote() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(vault.path().join("Notebooks/d.md"), "# D\n\nOne.\n").unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let mut heard = state.tx.subscribe();
        let listed = body_json(
            blocks_handler(
                State(state.clone()),
                Query(PathQuery {
                    path: "Notebooks/d.md".into(),
                }),
            )
            .await
            .unwrap(),
        )
        .await;
        block_edit_handler(
            State(state.clone()),
            Json(BlockBody {
                path: "Notebooks/d.md".into(),
                index: Some(1),
                hash: listed["blocks"][1]["hash"].as_str().map(str::to_string),
                markdown: Some("Two.".into()),
            }),
        )
        .await
        .unwrap();
        let frame: serde_json::Value = serde_json::from_str(&heard.try_recv().unwrap()).unwrap();
        assert_eq!(frame["type"], "note_changed");
        assert_eq!(frame["path"], "Notebooks/d.md");
        assert!(frame["hash"].as_str().is_some_and(|h| h.len() >= 8));
    }

    #[tokio::test]
    async fn the_cards_of_a_deck_are_listed_for_the_card_editor() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(
            vault.path().join("Notebooks/k.md"),
            "# T\n\n## A\n\nOne.\n\n## B\n\nTwo.\n",
        )
        .unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let out = body_json(
            cards_handler(
                State(state),
                Query(PathQuery {
                    path: "Notebooks/k.md".into(),
                }),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(out["cards"][1]["title"], "B");
        assert!(out["cards"][0]["hash"].as_str().is_some());
    }

    #[tokio::test]
    async fn a_file_nothing_was_done_to_is_ready_and_a_missing_one_is_refused() {
        let vault = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
        std::fs::write(vault.path().join("Notebooks/a.docx"), b"x").unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let reply = file_handler(
            State(state.clone()),
            Query(PathQuery {
                path: "Notebooks/a.docx".into(),
            }),
        )
        .await
        .unwrap()
        .into_response();
        let body = to_bytes(reply.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["state"], "ready");
        assert_eq!(v["actions"]["shrink"], true);
        assert_eq!(v["actions"]["undo"], false);

        let outside = shrink_handler(
            State(state),
            Json(PathBody {
                path: "/etc/hosts".into(),
            }),
        )
        .await;
        assert!(outside.is_err(), "a file outside the vault is refused");

        let hostile = media_handler(
            State(Arc::new(WsState::new(vault.path().to_path_buf(), None))),
            Query(PathQuery {
                path: "Notebooks/a.docx".into(),
            }),
        )
        .await;
        assert!(hostile.is_err(), "only stored pictures are served");
        let traversal = media_handler(
            State(Arc::new(WsState::new(vault.path().to_path_buf(), None))),
            Query(PathQuery {
                path: "_media/cas/../../Notebooks/a.docx".into(),
            }),
        )
        .await;
        assert!(traversal.is_err());
    }
}
