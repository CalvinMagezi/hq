//! `GET /api/usage/forecast`: where spend is heading this month, per budget, and what drives it.

use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};
use hq_agent::usage_forecast::forecast_report;
use hq_core::config::HqConfig;

use crate::WsState;
use crate::error::ApiError;

pub(crate) async fn usage_forecast_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let budgets = HqConfig::load().map(|c| c.budgets).unwrap_or_default();
    let now = chrono::Utc::now().timestamp();
    let db = state.db.clone();
    let report = tokio::task::spawn_blocking(move || forecast_report(&db, &budgets, now))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;
    Ok(Json(report))
}
