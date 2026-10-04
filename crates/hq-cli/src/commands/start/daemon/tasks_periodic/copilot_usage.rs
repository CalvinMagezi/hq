//! Samples the Copilot credit balance for the burn-rate meter.

use anyhow::Result;
use chrono::{Duration, Utc};
use hq_core::config::{HqConfig, copilot_active};
use hq_db::Database;
use hq_db::copilot_usage_samples as store;
use hq_tools::copilot_credits::sample_now;
use tracing::warn;

const MIN_INTERVAL_MINUTES: i64 = 1;

/// Runs every minute from the scheduler and samples only once `interval_minutes` has passed
/// since the newest stored sample, so the cadence follows config without a dynamic timer.
pub async fn run_copilot_usage(db: &Database, config: &HqConfig) -> Result<()> {
    let cfg = &config.copilot_usage;
    if !cfg.enabled || !copilot_active(config) {
        return Ok(());
    }
    let interval = Duration::minutes((cfg.interval_minutes as i64).max(MIN_INTERVAL_MINUTES));
    let recent = db.with_conn(|c| store::list_samples_since(c, Utc::now() - interval));
    if recent.is_ok_and(|s| !s.is_empty()) {
        return Ok(());
    }
    if let Err(e) = sample_now(db).await {
        warn!(error = %e, "copilot-usage: could not sample the credit balance");
    }
    Ok(())
}
