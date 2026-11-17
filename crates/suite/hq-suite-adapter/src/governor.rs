//! `hq suite storage` and the governor's pass: the vault, the clock and the budget given to the
//! pure logic in `hq-suite-governor`.

use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use hq_suite_governor::{Outcome, Settings, describe, govern, report};
use hq_suite_store::SuitePaths;

use crate::shrink::VaultNotes;

const BYTES_PER_MB: u64 = 1024 * 1024;

/// The settings as the config states them.
pub fn settings(budget_mb: Option<u64>, cold_window_days: u32) -> Settings {
    Settings {
        budget_bytes: budget_mb.map(|mb| mb.saturating_mul(BYTES_PER_MB)),
        cold_window_days,
    }
}

/// What the suite holds, in words.
pub fn storage_text(vault: &Path, budget_mb: Option<u64>) -> String {
    let paths = SuitePaths::new(vault);
    describe(&report(&paths), settings(budget_mb, 0).budget_bytes)
}

/// One governing pass over the vault.
pub fn run(vault: &Path, settings: &Settings) -> Result<Outcome> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    govern(&paths, &notes, settings, &today, SystemTime::now()).context("the storage pass failed")
}

/// What a pass did, in words. Empty when it did nothing and found nothing wrong.
pub fn outcome_text(o: &Outcome) -> String {
    let mut lines = Vec::new();
    if o.media_removed > 0 {
        lines.push(format!(
            "Removed {} picture(s) no note uses ({} bytes).",
            o.media_removed, o.media_bytes_freed
        ));
    }
    if o.cache_bytes_freed > 0 {
        lines.push(format!(
            "Cleared {} bytes of temporary files.",
            o.cache_bytes_freed
        ));
    }
    if o.originals_expired > 0 {
        lines.push(format!(
            "Let {} old original(s) go to stay within your limit ({} bytes). Their notes say so.",
            o.originals_expired, o.original_bytes_freed
        ));
    }
    if o.marked_gone > 0 {
        lines.push(format!(
            "{} note(s) now say their original is gone, because the trash cleaned it up.",
            o.marked_gone
        ));
    }
    if o.over_budget {
        lines.push(
            "Still over your limit. Only you can free more: undo a shrink or raise the limit."
                .into(),
        );
    }
    for why in &o.skipped {
        lines.push(format!("Skipped tidying: {why}."));
    }
    lines.join("\n")
}
