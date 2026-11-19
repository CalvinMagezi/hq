//! `owner-decisions`: a decision past its deadline takes its default when it is reversible and is
//! held otherwise. See `hq_db::owner_decisions`.

use anyhow::Result;
use hq_db::Database;
use tracing::info;

pub async fn run_owner_decisions(db: &Database) -> Result<()> {
    for d in hq_db::owner_decisions::resolve_due(db, chrono::Utc::now())? {
        info!(id = %d.id, state = %d.state, "owner-decisions: a deadline passed");
    }
    Ok(())
}
