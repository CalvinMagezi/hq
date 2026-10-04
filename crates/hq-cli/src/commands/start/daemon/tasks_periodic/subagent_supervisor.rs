use anyhow::Result;
use hq_db::Database;
use tracing::info;

/// Close delegated runs whose process died (a restart interrupts every child
/// the old process owned), flag quiet ones, and escalate work left partial or
/// blocked. Runs whether or not automatic follow-up turns are enabled, so the
/// evidence is never stale: only the wake-up of the parent is gated.
pub fn run_subagent_supervisor(db: &Database) -> Result<()> {
    let report = hq_agent::followup::maintain(db, hq_agent::followup::process_alive);
    if report != hq_agent::followup::Maintenance::default() {
        info!(
            interrupted = report.interrupted,
            stalled = report.stalled,
            escalated = report.escalated,
            "subagent-supervisor: pass recorded changes"
        );
    }
    Ok(())
}
