//! One-time repairs of note files written by older versions, run when the daemon starts. A
//! phone instance has no shell to run `hq vault repair`, so the daemon applies them itself,
//! keeps every original under `_trash/`, and says what it did in the owner's inbox.

use std::path::Path;

use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use tracing::{info, warn};

/// Present once the repair has run, so a later start never touches the notes again.
const NESTED_FRONTMATTER_MARKER: &str = "_system/.repairs/nested-frontmatter-v1";
const SOURCE: &str = "note-repair";
const EXAMPLES_SHOWN: usize = 10;

pub fn repair_nested_frontmatter_once(vault_path: &Path, db: &Database) {
    let marker = vault_path.join(NESTED_FRONTMATTER_MARKER);
    if marker.exists() {
        return;
    }
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let backup = vault_path
        .join("_trash")
        .join(&date)
        .join("nested-frontmatter");
    let report = match hq_vault::remediation::repair_nested_frontmatter(vault_path, &backup, true) {
        Ok(report) => report,
        Err(e) => {
            warn!(error = %e, "note-repair: nested frontmatter repair failed; will retry next start");
            return;
        }
    };
    if let Some(parent) = marker.parent()
        && let Err(e) =
            std::fs::create_dir_all(parent).and_then(|()| std::fs::write(&marker, &date))
    {
        warn!(error = %e, "note-repair: could not write the marker");
    }
    info!(
        scanned = report.scanned,
        repaired = report.nested.len(),
        "note-repair: nested frontmatter"
    );
    if report.nested.is_empty() {
        return;
    }
    let item = ValueItem::new(
        SOURCE,
        ValueKind::Fyi,
        format!("Repaired tags on {} notes", report.nested.len()),
        summary(&report, &backup, vault_path),
    )
    .with_dedup_key("note-repair-nested-frontmatter-v1".to_string());
    if let Err(e) = hq_db::value_items::emit(db, &item) {
        warn!(error = %e, "note-repair: could not post the report");
    }
}

fn summary(
    report: &hq_vault::remediation::NestedFrontmatterReport,
    backup: &Path,
    vault_path: &Path,
) -> String {
    let shown: Vec<&str> = report
        .nested
        .iter()
        .take(EXAMPLES_SHOWN)
        .map(String::as_str)
        .collect();
    let more = report.nested.len().saturating_sub(EXAMPLES_SHOWN);
    let backup = backup.strip_prefix(vault_path).unwrap_or(backup);
    let mut text = format!(
        "These notes had their frontmatter (and tags) inside the body, left there by an older \
         vault_write_note. It is now the note's own frontmatter, so the tags show up in search \
         and triage. Originals are in {}.\n\n{}",
        backup.display(),
        shown.join("\n")
    );
    if more > 0 {
        text.push_str(&format!("\n...and {more} more"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repair_runs_once_and_reports_to_the_inbox() {
        let vault = tempfile::tempdir().unwrap();
        let note = vault.path().join("Notebooks/a.md");
        std::fs::create_dir_all(note.parent().unwrap()).unwrap();
        std::fs::write(
            &note,
            "---\ntitle: A\n---\n\n---\ntags:\n  - x\n---\nbody\n",
        )
        .unwrap();
        let db = Database::open_memory().unwrap();

        repair_nested_frontmatter_once(vault.path(), &db);
        assert!(vault.path().join(NESTED_FRONTMATTER_MARKER).exists());
        let raw = std::fs::read_to_string(&note).unwrap();
        assert_eq!(
            hq_core::frontmatter_utils::extract_tags_from_frontmatter(&raw),
            ["x"]
        );
        let pending =
            hq_db::value_items::list_by_state(&db, hq_core::types::ValueState::Pending).unwrap();
        assert_eq!(pending.len(), 1);

        // A note written in the old shape after the marker exists is not touched.
        std::fs::write(
            &note,
            "---\ntitle: A\n---\n\n---\ntags:\n  - y\n---\nbody\n",
        )
        .unwrap();
        repair_nested_frontmatter_once(vault.path(), &db);
        assert!(
            std::fs::read_to_string(&note)
                .unwrap()
                .contains("  - y\n---\nbody")
        );
    }
}
