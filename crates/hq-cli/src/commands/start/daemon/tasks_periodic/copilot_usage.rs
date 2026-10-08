//! Samples the Copilot credit balance for the burn-rate meter.

use anyhow::Result;
use chrono::{Duration, Utc};
use hq_core::config::{HqConfig, copilot_active};
use hq_db::Database;
use hq_db::copilot_usage_samples as store;
use hq_llm::provider::LlmError;
use hq_tools::copilot_credits::sample_now;
use std::sync::atomic::{AtomicI64, Ordering};
use tracing::{debug, warn};

const MIN_INTERVAL_MINUTES: i64 = 1;
/// After GitHub refuses the token, wait this long before asking again: a refusal is not transient.
const REFUSED_BACKOFF_MINUTES: i64 = 60;

/// Unix seconds before which the sampler does not call GitHub again. 0 = no backoff.
static RETRY_AFTER: AtomicI64 = AtomicI64::new(0);

/// Runs every minute from the scheduler and samples only once `interval_minutes` has passed
/// since the newest stored sample, so the cadence follows config without a dynamic timer.
pub async fn run_copilot_usage(db: &Database, config: &HqConfig) -> Result<()> {
    let cfg = &config.copilot_usage;
    if !cfg.enabled || !copilot_active(config) {
        return Ok(());
    }
    let interval = Duration::minutes((cfg.interval_minutes as i64).max(MIN_INTERVAL_MINUTES));
    let recent = db.with_conn(|c| store::list_samples_since(c, Utc::now() - interval));
    if recent.is_ok_and(|s| !s.is_empty())
        || Utc::now().timestamp() < RETRY_AFTER.load(Ordering::Relaxed)
    {
        return Ok(());
    }
    match sample_now(db).await {
        Ok(_) => RETRY_AFTER.store(0, Ordering::Relaxed),
        Err(e) if matches!(e.downcast_ref::<LlmError>(), Some(LlmError::Auth { .. })) => {
            // Warn on the first refusal only; later ones are the same fact, an hour apart.
            let first = RETRY_AFTER.swap(
                (Utc::now() + Duration::minutes(REFUSED_BACKOFF_MINUTES)).timestamp(),
                Ordering::Relaxed,
            ) == 0;
            if first {
                warn!(error = %e, "copilot-usage: GitHub refused the usage read; pausing the sampler for an hour");
            } else {
                debug!(error = %e, "copilot-usage: usage read still refused");
            }
        }
        Err(e) => warn!(error = %e, "copilot-usage: could not sample the credit balance"),
    }
    Ok(())
}
