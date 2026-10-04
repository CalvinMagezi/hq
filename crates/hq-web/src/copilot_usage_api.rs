//! `GET /api/copilot-usage`: the Copilot credit balance, burn report and recent sample points.

use axum::{Json, extract::State, response::IntoResponse};
use chrono::Utc;
use hq_core::config::{HqConfig, copilot_active};
use hq_llm::copilot_usage::{CopilotQuota, fetch_quota};
use hq_tools::copilot_credits::{NOT_METERED_NOTE, burn_view};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::WsState;
use crate::error::ApiError;

/// A request burst shares one GitHub read.
const LIVE_CACHE_TTL: Duration = Duration::from_secs(60);
const CHART_POINTS: usize = 48;

type Live = Result<CopilotQuota, String>;

static LIVE_CACHE: Mutex<Option<(Instant, Live)>> = Mutex::const_new(None);

async fn cached_live_quota() -> Live {
    let mut guard = LIVE_CACHE.lock().await;
    if let Some((at, live)) = guard.as_ref()
        && at.elapsed() < LIVE_CACHE_TTL
    {
        return live.clone();
    }
    let live = fetch_quota().await.map_err(|e| e.to_string());
    *guard = Some((Instant::now(), live.clone()));
    live
}

fn usage_json(db: &hq_db::Database, live: Live) -> anyhow::Result<Value> {
    let quota = match live {
        Ok(q) => q,
        Err(e) => {
            return Ok(
                json!({ "active": true, "error": format!("Could not read the Copilot balance: {e}") }),
            );
        }
    };
    if !quota.is_metered() {
        return Ok(json!({ "active": true, "quota": quota, "note": NOT_METERED_NOTE }));
    }
    let view = burn_view(db, &quota, Utc::now(), CHART_POINTS)?;
    let samples: Vec<Value> = view
        .points
        .iter()
        .map(|s| json!({ "ts": s.ts, "credits_used": s.credits_used }))
        .collect();
    Ok(json!({ "active": true, "quota": quota, "burn": view.burn, "samples": samples }))
}

pub(crate) async fn copilot_usage_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let config = match state.hq_config.as_deref() {
        Some(c) => c.clone(),
        None => HqConfig::load().unwrap_or_default(),
    };
    if !copilot_active(&config) {
        return Ok(Json(json!({ "active": false })));
    }
    let live = cached_live_quota().await;
    let body = usage_json(&state.db, live)
        .map_err(|e| ApiError::internal(format!("copilot usage failed: {e}")))?;
    Ok(Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quota(has_quota: bool) -> CopilotQuota {
        CopilotQuota {
            login: Some("seat".into()),
            plan: Some("business".into()),
            sku: None,
            token_based_billing: true,
            entitlement: if has_quota { 50000.0 } else { 0.0 },
            credits_used: 37009.0,
            remaining: 12940.0,
            percent_remaining: 25.8,
            overage_permitted: true,
            unlimited: false,
            has_quota,
            reset_at: None,
            fetched_at: Utc::now(),
        }
    }

    #[test]
    fn shapes_metered_unmetered_and_failed_reads() {
        let db = hq_db::Database::open_memory().unwrap();
        let ok = usage_json(&db, Ok(quota(true))).unwrap();
        assert!(ok["burn"].is_object() && ok["samples"].is_array());
        assert_eq!(ok["quota"]["entitlement"], 50000.0);
        let free = usage_json(&db, Ok(quota(false))).unwrap();
        assert!(free["note"].is_string() && free.get("burn").is_none());
        let bad = usage_json(&db, Err("boom".into())).unwrap();
        assert!(bad["error"].as_str().unwrap().contains("boom"));
        assert!(bad.get("quota").is_none());
    }
}
