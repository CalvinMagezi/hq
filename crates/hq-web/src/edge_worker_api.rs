//! `GET /api/edge-worker`: what the phone needs to queue work for the edge worker and to show
//! its runs beside the other helpers (`hq_tools::github::dispatch`).

use axum::{Json, extract::State, response::IntoResponse};
use hq_core::config::HqConfig;
use hq_tools::github::app::AppClient;
use hq_tools::github::dispatch;
use serde_json::json;
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

/// How many of the worker's runs the phone lists.
const RUNS_SHOWN: usize = 10;

pub(crate) async fn edge_worker_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let config = match state.hq_config.as_deref() {
        Some(c) => c.clone(),
        None => HqConfig::load().unwrap_or_default(),
    };
    let runs = state
        .db
        .with_conn(|c| dispatch::recent_runs(c, RUNS_SHOWN))
        .map_err(|e| ApiError::internal(format!("edge worker runs failed: {e}")))?;
    let app = AppClient::standard()
        .map_err(|e| ApiError::internal(format!("GitHub client failed: {e}")))?;
    Ok(Json(json!({
        "enabled": dispatch::armed(&config, &app),
        "tag": dispatch::DISPATCH_TAG,
        "repos": dispatch::allowed_repos(&config, &app),
        "runs": runs,
    })))
}
