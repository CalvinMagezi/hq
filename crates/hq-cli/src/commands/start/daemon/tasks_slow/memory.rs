//! Memory-related slow tasks: forgetting.

use anyhow::Result;
use std::path::Path;
use tracing::info;

/// Put away notes nobody has used for a year (never a pinned one), keep no more than 20,000 in
/// use, and clear the old stored turn text. Nothing is deleted: a note put away can be brought
/// back from the owner's memory page.
pub async fn run_memory_forgetting(vault_path: &Path, db: &hq_db::Database) -> Result<()> {
    if !super::super::helpers::has_run_today(vault_path, "memory-forgetting") {
        let forgetter = hq_memory::MemoryForgetter::new(db.clone(), vault_path.to_path_buf());
        match forgetter.run_cycle() {
            Ok(result) => {
                if result.aged_out > 0 || result.capped > 0 || result.scrubbed > 0 {
                    info!(
                        aged_out = result.aged_out,
                        capped = result.capped,
                        scrubbed = result.scrubbed,
                        total = result.stats_after.total,
                        "memory-forgetting: unused notes put away"
                    );
                } else {
                    tracing::debug!("memory-forgetting: nothing to put away");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "memory-forgetting: cycle failed");
            }
        }
        super::super::helpers::mark_run_today(vault_path, "memory-forgetting");
    }
    Ok(())
}
