//! `verify-briefing`: hourly, raises the one alert a day when the verification queue needs the
//! owner (too many items waiting on them, one that has aged, a queue that will not clear, a spent
//! budget with tasks waiting for it) and, at the hour the owner set, posts the "while you were
//! away" briefing. Both are web-only items, deduplicated per day.

use anyhow::Result;
use chrono::Timelike;
use hq_core::config::{HqConfig, VerificationMode};
use hq_db::Database;
use std::path::Path;

pub async fn run_verify_briefing(
    vault_path: &Path,
    db: &Database,
    config: &HqConfig,
) -> Result<()> {
    let cfg = &config.tasks.verification;
    if cfg.mode == VerificationMode::Off {
        return Ok(());
    }
    let now = chrono::Utc::now();
    let since = (now - chrono::Duration::hours(24))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let briefing = db.with_conn(|c| hq_db::tasks::briefing(c, cfg, &since))?;
    if !briefing.health.alerts.is_empty() {
        tracing::info!(
            alerts = briefing.health.alerts.len(),
            "verify-briefing: the queue needs attention"
        );
        hq_tools::tasks::notify_queue_alert(vault_path.to_path_buf(), &briefing.health.alerts)
            .await;
    }
    if cfg
        .briefing_hour_utc
        .is_some_and(|h| u32::from(h) == now.hour())
    {
        hq_tools::tasks::notify_briefing(
            vault_path.to_path_buf(),
            hq_db::tasks::briefing_text(&briefing),
        )
        .await;
    }
    Ok(())
}
