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
        .route("/api/suite/shrink", post(shrink_handler))
        .route("/api/suite/confirm", post(confirm_handler))
        .route("/api/suite/abandon", post(abandon_handler))
        .route("/api/suite/undo", post(undo_handler))
        .route("/api/suite/storage", get(storage_handler))
}

#[derive(Deserialize)]
pub(crate) struct PathQuery {
    path: String,
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

pub(crate) async fn abandon_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<JobBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    Ok(Json(
        blocking(move || hq_suite_adapter::api::abandon(&vault, &b.job)).await?,
    ))
}

pub(crate) async fn undo_handler(
    State(state): State<Arc<WsState>>,
    Json(b): Json<JobBody>,
) -> Result<impl IntoResponse, ApiError> {
    let vault = state.vault_path.clone();
    Ok(Json(
        blocking(move || hq_suite_adapter::api::undo(&vault, &b.job)).await?,
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
    }
}
