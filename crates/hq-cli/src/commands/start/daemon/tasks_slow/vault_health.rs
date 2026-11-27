//! The vault health check: reads every note's links and properties, keeps the result for the
//! Notes screen, `hq doctor` and the agent tool, and mirrors it into `_system/VAULT-HEALTH.md`.
//!
//! Nothing here changes a note. It reads notes on this device only: no model, no network.
//! Turn it off with `daemon.vault_health: false`.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use std::path::Path;
use tracing::info;

use super::super::helpers::ensure_dir;

/// Scan, store, and write the readable copy.
pub async fn run_vault_health(vault_path: &Path, db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.vault_health {
        return Ok(());
    }
    let (vault, job_db) = (vault_path.to_path_buf(), db.clone());
    // Reads every note, so it stays off the runtime threads.
    let stored = tokio::task::spawn_blocking(move || {
        hq_tools::vault_health::scan_and_store(&vault, &job_db)
    })
    .await??;

    let sys_dir = vault_path.join("_system");
    ensure_dir(&sys_dir);
    std::fs::write(
        sys_dir.join("VAULT-HEALTH.md"),
        hq_tools::vault_health::render_markdown(&stored),
    )?;

    let c = &stored.report.counts;
    info!(
        notes = c.notes,
        links = c.links.total,
        problems = c.problems(),
        dead = c.links.dead,
        ambiguous = c.links.ambiguous,
        anchors = c.links.broken_anchors,
        duplicates = c.duplicates,
        frontmatter = c.frontmatter,
        "vault-health: check complete"
    );
    Ok(())
}
