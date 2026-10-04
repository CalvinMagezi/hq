//! Memory-related slow tasks: forgetting.

use anyhow::Result;
use std::path::Path;
use tracing::info;

/// Run synaptic homeostasis: tiered decay + pruning of weak memories.
/// - Tier 1: 1.5%/day decay for unconsolidated memories > 7 days old
/// - Tier 2: 5%/day accelerated decay for poorly-linked insights
/// - Tier 3: 0.5%/day protected decay for well-linked insights
/// - Prune: DELETE memories with importance <= 0.05 after 60 days
pub async fn run_memory_forgetting(vault_path: &Path, db: &hq_db::Database) -> Result<()> {
    if !super::super::helpers::has_run_today(vault_path, "memory-forgetting") {
        let forgetter = hq_memory::MemoryForgetter::new(db.clone(), vault_path.to_path_buf());
        match forgetter.run_cycle() {
            Ok(result) => {
                if result.decayed > 0 || result.pruned > 0 {
                    info!(
                        decayed = result.decayed,
                        pruned = result.pruned,
                        total = result.stats_after.total,
                        "memory-forgetting: synaptic homeostasis complete"
                    );
                } else {
                    tracing::debug!("memory-forgetting: nothing to decay or prune");
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
