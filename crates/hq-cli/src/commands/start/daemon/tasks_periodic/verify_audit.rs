//! `verify-audit`: keeps the weekly audit floor (docs/plans/task-verification.md, "Calibration").
//! A random sample of the tasks HQ completed on its own is queued for the owner's agree or not,
//! and the owner hears of it once, as one digest. Safe to run as often as wanted: a week that
//! already has its floor selects nothing and says nothing.

use anyhow::Result;
use hq_core::config::{HqConfig, VerificationMode};
use hq_db::Database;
use std::path::Path;

pub async fn run_verify_audit(vault_path: &Path, db: &Database, config: &HqConfig) -> Result<()> {
    let cfg = &config.tasks.verification;
    if cfg.mode == VerificationMode::Off {
        return Ok(());
    }
    let seed = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(1) as u64;
    let picked = db.with_conn(|c| hq_db::tasks::push_weekly_audits(c, cfg, seed))?;
    if picked.is_empty() {
        return Ok(());
    }
    let open = db.with_conn(|c| hq_db::tasks::open_audits(c, 500))?.len();
    tracing::info!(
        selected = picked.len(),
        open,
        "verify-audit: weekly audits queued"
    );
    hq_tools::tasks::notify_audit_digest(vault_path.to_path_buf(), picked.len(), open).await;
    Ok(())
}
