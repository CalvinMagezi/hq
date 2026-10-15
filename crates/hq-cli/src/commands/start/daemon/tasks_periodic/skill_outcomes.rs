use anyhow::Result;
use hq_db::Database;
use tracing::info;

/// Loads older than this are left as they are.
const WINDOW_DAYS: u32 = 30;

/// Settle skill outcomes from what happened to the work (see `hq_db::skill_outcomes`).
pub fn run_skill_outcomes(db: &Database) -> Result<()> {
    let settled = db.with_conn(|conn| hq_db::skill_outcomes::settle(conn, WINDOW_DAYS))?;
    if settled.updated > 0 {
        info!(
            examined = settled.examined,
            updated = settled.updated,
            "skill-outcomes: outcomes settled from task, run and note decisions"
        );
    }
    Ok(())
}
