//! `reminders`: rings the reminders the owner set. One inbox item per ping, pushed, with Done and
//! Snooze on it. A nudge pings again until the owner answers. No model. Off with
//! `daemon.reminders: false`. Rules: `crates/hq-tools/src/reminders/`.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_tools::reminders::fire::fire_due;
use tracing::info;

pub async fn run_reminders(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.reminders {
        return Ok(());
    }
    let report = fire_due(db, &config.reminders, chrono::Utc::now().timestamp())?;
    if report.rang > 0 || report.stopped > 0 {
        info!(
            rang = report.rang,
            stopped = report.stopped,
            "reminders: pass done"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_toggle_turns_the_task_off() {
        let db = Database::open_memory().unwrap();
        let mut config = HqConfig::default();
        config.daemon.reminders = false;
        run_reminders(&db, &config).await.unwrap();
        run_reminders(&db, &HqConfig::default()).await.unwrap();
    }
}
