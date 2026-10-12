//! `stack-dispatch`: every minute, start a run for each stack specialist that
//! has a task assigned and nothing running (`hq_tools::harness_session::stack`).
//! With no specialists configured it does nothing.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use std::sync::{Arc, Mutex};

/// The skipped lines of the last sweep: the same set is a steady state (a
/// specialist that cannot resolve, a run that cannot start) and is logged at
/// warn once, then at debug until it changes.
static LAST_SKIPPED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub async fn run_stack_dispatch(
    vault_path: &std::path::Path,
    db: &Database,
    config: &HqConfig,
) -> Result<()> {
    if config.stack.specialists.is_empty() {
        return Ok(());
    }
    let db = Arc::new(db.clone());
    let resolve = |name: Option<&str>| hq_tools::agent_host::host(name);
    let report =
        hq_tools::harness_session::stack::dispatch(vault_path, &db, config, &resolve).await?;
    let changed = {
        let mut last = LAST_SKIPPED.lock().unwrap_or_else(|p| p.into_inner());
        let changed = *last != report.skipped;
        last.clone_from(&report.skipped);
        changed
    };
    for line in &report.skipped {
        if changed {
            tracing::warn!(%line, "stack-dispatch: specialist skipped");
        } else {
            tracing::debug!(%line, "stack-dispatch: specialist still skipped");
        }
    }
    if !report.started.is_empty() {
        tracing::info!(
            started = report.started.len(),
            "stack-dispatch: runs started"
        );
    }
    Ok(())
}
