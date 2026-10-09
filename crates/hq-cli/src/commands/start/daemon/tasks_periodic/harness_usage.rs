//! Every few minutes, record what the coding agents on this machine report about their own token use.

use anyhow::Result;
use hq_db::Database;
use tracing::{debug, warn};

pub async fn run_harness_usage(db: &Database) -> Result<()> {
    let Some(home) = dirs::home_dir() else {
        return Ok(());
    };
    let db = db.clone();
    // Reads files and SQLite databases, so it stays off the runtime threads.
    match tokio::task::spawn_blocking(move || hq_tools::harness_usage_collect::scan(&db, &home)).await {
        Ok(Ok(report)) if report.calls_recorded > 0 => {
            debug!(calls = report.calls_recorded, sources = report.sources_read, "harness usage recorded");
        }
        Ok(Ok(_)) => {}
        Ok(Err(e)) => warn!(error = %e, "harness usage scan failed"),
        Err(e) => warn!(error = %e, "harness usage scan was cancelled"),
    }
    Ok(())
}
