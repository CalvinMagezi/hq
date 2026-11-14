//! Periodic check for usage quota windows crossing configured alert thresholds.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_tools::quota_windows::{CopilotQuotaReader, check_quota_alerts};

/// Checks configured quota windows and emits one inbox item per crossed threshold per reset cycle.
pub async fn run_quota_alerts(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.quota_alerts.is_active() {
        return Ok(());
    }
    let reader = CopilotQuotaReader::new(db.clone());
    check_quota_alerts(db, &config.quota_alerts, &reader).await?;
    Ok(())
}
