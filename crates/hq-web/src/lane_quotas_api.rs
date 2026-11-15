//! `GET /api/lane-quotas`: each coding-agent lane's remaining allowance, per host. Behind the
//! same owner auth as every other `/api` route; the data goes to no one else.

use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use hq_core::config::HqConfig;
use hq_tools::lane_quotas::{Options, read_all};
use serde::Deserialize;
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

#[derive(Deserialize, Default)]
pub(crate) struct LaneQuotasQuery {
    host: Option<String>,
    lane: Option<String>,
    #[serde(default)]
    refresh: bool,
}

pub(crate) async fn lane_quotas_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<LaneQuotasQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let config = match state.hq_config.as_deref() {
        Some(c) => c.clone(),
        None => HqConfig::load().unwrap_or_default(),
    };
    let named = |v: Option<String>| v.filter(|s| !s.is_empty());
    let opts = Options {
        refresh: q.refresh,
        host: named(q.host),
        lane: named(q.lane),
    };
    let db = state.db.clone();
    // The hosts are reached over blocking calls, so they stay off the runtime threads.
    let report = tokio::task::spawn_blocking(move || read_all(&db, &config, &opts))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn off_answers_an_empty_report_and_the_shape_has_the_thresholds() {
        let vault = tempfile::TempDir::new().unwrap();
        let mut config = HqConfig::default();
        config.quota.enabled = false;
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        state.hq_config = Some(Arc::new(config));
        let state = Arc::new(state);
        let reply = lane_quotas_handler(State(state), Query(LaneQuotasQuery::default()))
            .await
            .map_err(|_| "handler failed")
            .unwrap()
            .into_response();
        let body = to_bytes(reply.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["enabled"], false);
        assert!(v["lanes"].as_array().unwrap().is_empty());
        assert_eq!(v["warn_percent"], 80.0);
    }
}
