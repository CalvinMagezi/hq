//! Memory consolidation with smart gating, inspired by Claude Code's Kairos daemon.
//!
//! A cycle runs only when at least MIN_HOURS have passed since the last
//! consolidation and MIN_NEW_MEMORIES are waiting. The stamp file's mtime is
//! the "last consolidated at" time, which `hq memory status` reads.

use anyhow::Result;
use hq_db::Database;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{debug, info, warn};

use hq_memory::MemoryConsolidator;

/// 2 hours balances freshness against the LLM cost of each consolidation pass.
const MIN_HOURS_BETWEEN_CONSOLIDATION: f64 = 2.0;

/// Require meaningful activity before paying for an LLM pass.
const MIN_NEW_MEMORIES: usize = 5;

/// Stamp file inside vault `_data/`. The name predates the lock it used to be.
const CONSOLIDATION_STAMP_FILE: &str = ".consolidate-lock";

const SECS_PER_HOUR: f64 = 3600.0;

fn stamp_path(vault_path: &Path) -> PathBuf {
    vault_path.join("_data").join(CONSOLIDATION_STAMP_FILE)
}

/// Hours since the last consolidation, or infinity if there never was one.
fn hours_since_last(stamp: &Path) -> f64 {
    stamp_mtime(stamp)
        .and_then(|t| t.elapsed().ok())
        .map_or(f64::INFINITY, |d| d.as_secs_f64() / SECS_PER_HOUR)
}

fn stamp_mtime(stamp: &Path) -> Option<SystemTime> {
    std::fs::metadata(stamp).and_then(|m| m.modified()).ok()
}

/// Rewriting the file bumps its mtime, and creates it on the first run.
fn touch(stamp: &Path) {
    if let Some(parent) = stamp.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(stamp, "") {
        warn!(error = %e, "failed to update consolidation stamp");
    }
}

/// A failed cycle must not count as a consolidation, so the time gate reopens.
fn rewind(stamp: &Path, prior: Option<SystemTime>) {
    let _ = match prior {
        Some(t) => filetime::set_file_mtime(stamp, filetime::FileTime::from_system_time(t)),
        None => std::fs::remove_file(stamp),
    };
}

/// Run a full memory maintenance cycle with smart gating.
///
/// No lock is needed: the only caller is the daemon's sequential task loop,
/// and `DaemonLock` keeps a second daemon from starting. The stamp is touched
/// before the LLM pass, so a run the scheduler's timeout kills still holds the
/// time gate shut instead of retrying every tick.
///
/// Forgetting is the daily `memory-forgetting` task, not part of this cycle.
pub async fn run_memory_cycle(
    db: &Database,
    vault_path: &Path,
    provider: Option<std::sync::Arc<dyn hq_llm::LlmProvider>>,
) -> Result<()> {
    let stamp = stamp_path(vault_path);
    let prior = stamp_mtime(&stamp);
    let hours_since = hours_since_last(&stamp);
    if hours_since < MIN_HOURS_BETWEEN_CONSOLIDATION {
        debug!(
            hours_since = format!("{:.1}", hours_since),
            min_hours = MIN_HOURS_BETWEEN_CONSOLIDATION,
            "consolidation skipped: time gate"
        );
        return Ok(());
    }

    let unconsolidated = hq_memory::get_unconsolidated_memories(db, 30)?;
    if unconsolidated.len() < MIN_NEW_MEMORIES {
        debug!(
            count = unconsolidated.len(),
            min = MIN_NEW_MEMORIES,
            "consolidation skipped: activity gate"
        );
        return Ok(());
    }

    info!(
        hours_since = format!("{:.1}", hours_since),
        unconsolidated = unconsolidated.len(),
        "consolidation gates passed, starting cycle"
    );

    touch(&stamp);
    // "dream" is the shared router alias for memory consolidation; T1 has no alias of its own.
    let consolidator = match provider {
        Some(p) => {
            MemoryConsolidator::new_with_provider(db.clone(), vault_path.to_path_buf(), p, "dream".to_string())
        }
        None => MemoryConsolidator::new(db.clone(), vault_path.to_path_buf()),
    };
    match consolidator.run_cycle().await {
        Ok(Some(insight)) => {
            let preview: String = insight.chars().take(100).collect();
            info!(insight = %preview, "consolidation produced insight");
        }
        Ok(None) => info!("no consolidation needed this cycle"),
        Err(e) => {
            warn!(error = %e, "consolidation cycle failed, rewinding the stamp");
            rewind(&stamp, prior);
            return Err(e);
        }
    }
    touch(&stamp);

    // Refresh MEMORY.md (post-consolidation)
    if let Err(e) = consolidator.refresh_memory_file() {
        warn!(error = %e, "failed to refresh MEMORY.md");
    }

    Ok(())
}

