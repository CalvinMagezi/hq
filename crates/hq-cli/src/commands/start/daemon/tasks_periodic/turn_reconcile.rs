use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use std::path::Path;
use tracing::info;

/// Periodic turn reconcile (6h cadence).
///
/// The startup sweep covers daemon restarts, but a relay listener can also die
/// mid-turn while the daemon stays up, leaving a `running` row behind. This
/// pass runs beside live relays, so it only expires rows older than
/// `relay.background_turn_max_days` and leaves younger, possibly live, turns alone.
pub async fn run_turn_reconcile(_vault_path: &Path, db: &Database, config: &HqConfig) -> Result<()> {
    let expired =
        hq_daemon::turn_reconciler::reconcile_expired_turns(db, config.relay.background_turn_max_days)?;
    if expired > 0 {
        info!(expired, "turn-reconcile: background turns past the age ceiling marked interrupted");
    }
    Ok(())
}
