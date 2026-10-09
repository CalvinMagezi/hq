//! `GET /api/budgets`: every configured budget measured against the ledger.
//! `PUT /api/budgets`: replace the budgets, after validating them.

use std::sync::Arc;

use axum::{Json, extract::State, response::IntoResponse};
use hq_core::config::{BudgetsConfig, HqConfig};
use hq_db::usage_ledger::budget_status;
use serde_json::json;

use crate::WsState;
use crate::error::ApiError;

fn current(state: &WsState) -> BudgetsConfig {
    match state.hq_config.as_deref() {
        Some(c) => c.budgets.clone(),
        None => HqConfig::load().unwrap_or_default().budgets,
    }
}

pub(crate) async fn budgets_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let config = HqConfig::load().map(|c| c.budgets).unwrap_or_else(|_| current(&state));
    let now = chrono::Utc::now().timestamp();
    let statuses = config
        .budgets
        .iter()
        .map(|b| state.db.with_conn(|conn| budget_status(conn, b, now)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(Json(json!({
        "budgets": statuses,
        "background_run_usd": config.background_run_usd,
        "allow_unpriced_models": config.allow_unpriced_models,
        "problems": config.problems(),
    })))
}

pub(crate) async fn put_budgets_handler(
    State(_state): State<Arc<WsState>>,
    Json(new): Json<BudgetsConfig>,
) -> Result<impl IntoResponse, ApiError> {
    let problems = new.problems();
    if !problems.is_empty() {
        return Err(ApiError::bad_request(problems.join("; ")));
    }
    let saved = HqConfig::save_patch(|c| c.budgets = new)?;
    Ok(Json(json!({ "saved": saved.budgets.budgets.len() })))
}
