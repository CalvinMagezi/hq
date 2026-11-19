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
    Ok(Json(
        blocking(move || hq_suite_adapter::blocks::edit(&vault, &b.path, index, &hash, &markdown))
            .await?,
    ))
}

pub(crate) async fn block_insert_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<BlockBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    let markdown = b.markdown.clone().unwrap_or_default();
    let after = b.index.zip(b.hash.clone());
    let path = b.path.clone();
    Ok(Json(
        blocking(move || {
            hq_suite_adapter::blocks::insert(
                &vault,
                &path,
                after.as_ref().map(|(i, h)| (*i, h.as_str())),
                &markdown,
            )
        })
        .await?,
    ))
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
    Ok(Json(
        blocking(move || hq_suite_adapter::blocks::delete(&vault, &b.path, index, &hash)).await?,
    ))
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
