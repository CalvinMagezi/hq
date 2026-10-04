//! Vault self-reorganization: move, trash, mkdir, and frontmatter mutation.
//!
//! These are the primitives that let HQ restructure its own vault. Safety
//! rails live here rather than in callers: system directories are untouchable,
//! deletion is always a move into `_trash/` (30-day retention, purged by the
//! daemon), and moves rewrite inbound wikilinks via the remediation engine.

use anyhow::{Result, bail};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

use crate::remediation;

/// Directories the reorg tools may never touch, in either direction.
const PROTECTED_PREFIXES: &[&str] = &["_system", "_threads", "_data", "_trash"];

#[derive(Debug, Clone, Serialize)]
pub struct MoveReport {
    pub from: String,
    pub to: String,
    pub dry_run: bool,
    /// Notes whose wikilinks reference the moved note's stem.
    pub referrers: Vec<String>,
    /// Number of files whose links were rewritten (0 on dry runs).
    pub links_rewritten: u32,
}

fn guard_rel_path(rel: &str) -> Result<()> {
    if rel.is_empty() || rel.starts_with('/') || rel.contains("..") {
        bail!("invalid vault path '{rel}': must be relative, no traversal");
    }
    for p in PROTECTED_PREFIXES {
        if rel == *p || rel.starts_with(&format!("{p}/")) {
            bail!("'{rel}' is protected: {p}/ cannot be reorganized by tools");
        }
    }
    Ok(())
}

fn require_md(rel: &str) -> Result<()> {
    if Path::new(rel)
        .extension()
        .map(|e| e != "md")
        .unwrap_or(true)
    {
        bail!("'{rel}' is not a markdown (.md) note");
    }
    Ok(())
}

fn stem_of(rel: &str) -> String {
    Path::new(rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Notes under `Notebooks/` whose content wikilinks the given stem.
fn find_referrers(vault_path: &Path, stem: &str) -> Vec<String> {
    let mut referrers = Vec::new();
    let needle = stem.to_lowercase();
    let root = vault_path.join("Notebooks");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let links = remediation::extract_wikilinks(&content);
                if links.iter().any(|l| l.to_lowercase() == needle)
                    && let Ok(rel) = path.strip_prefix(vault_path)
                {
                    referrers.push(rel.to_string_lossy().to_string());
                }
            }
        }
    }
    referrers.sort();
    referrers
}

/// Move a note, rewriting inbound wikilinks to the new stem.
///
/// With `dry_run`, nothing changes on disk; the report shows what would move
/// and which notes reference it. Rewriting covers `[[wikilinks]]` (the vault's
/// link convention); plain relative markdown links are not rewritten.
pub(crate) fn move_note(vault_path: &Path, from: &str, to: &str, dry_run: bool) -> Result<MoveReport> {
    guard_rel_path(from)?;
    guard_rel_path(to)?;
    require_md(from)?;
    require_md(to)?;
    let src = vault_path.join(from);
    let dst = vault_path.join(to);
    if !src.is_file() {
        bail!("source note '{from}' does not exist");
    }
    if dst.exists() {
        bail!("destination '{to}' already exists");
    }

    let old_stem = stem_of(from);
    let new_stem = stem_of(to);
    let referrers = find_referrers(vault_path, &old_stem);

    if dry_run {
        return Ok(MoveReport {
            from: from.to_string(),
            to: to.to_string(),
            dry_run: true,
            referrers,
            links_rewritten: 0,
        });
    }

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&src, &dst)?;

    let links_rewritten = if old_stem != new_stem && !referrers.is_empty() {
        remediation::repoint_wikilinks(vault_path, &old_stem, &new_stem).unwrap_or(0)
    } else {
        0
    };

    Ok(MoveReport {
        from: from.to_string(),
        to: to.to_string(),
        dry_run: false,
        referrers,
        links_rewritten,
    })
}

/// Move a note into `_trash/YYYY-MM-DD/<original path>`. Never a hard delete.
/// Returns the trash-relative destination path.
pub(crate) fn trash_note(vault_path: &Path, rel: &str, dry_run: bool) -> Result<String> {
    guard_rel_path(rel)?;
    require_md(rel)?;
    let src = vault_path.join(rel);
    if !src.is_file() {
        bail!("note '{rel}' does not exist");
    }
    let date = chrono::Local::now().format("%Y-%m-%d");
    let dest_rel = format!("_trash/{date}/{rel}");
    if dry_run {
        return Ok(dest_rel);
    }
    let dst = vault_path.join(&dest_rel);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A same-day re-trash of a same-named note gets a numeric suffix instead
    // of clobbering the earlier trashed copy.
    let mut final_dst = dst.clone();
    let mut final_rel = dest_rel.clone();
    let mut n = 1;
    while final_dst.exists() {
        final_rel = format!("{}.{n}.md", dest_rel.trim_end_matches(".md"));
        final_dst = vault_path.join(&final_rel);
        n += 1;
    }
    std::fs::rename(&src, &final_dst)?;
    Ok(final_rel)
}

/// Set and/or remove frontmatter keys on a note, preserving the body.
pub(crate) fn update_frontmatter(
    vault_path: &Path,
    rel: &str,
    set: HashMap<String, serde_yaml::Value>,
    remove: Vec<String>,
) -> Result<()> {
    guard_rel_path(rel)?;
    require_md(rel)?;
    let path = vault_path.join(rel);
    if !path.is_file() {
        bail!("note '{rel}' does not exist");
    }
    let raw = std::fs::read_to_string(&path)?;
    let (mut fm, body) = crate::frontmatter::parse(&raw)?;
    for key in &remove {
        fm.remove(key);
    }
    for (key, value) in set {
        fm.insert(key, value);
    }
    let out = crate::frontmatter::serialize(&fm, &body)?;
    std::fs::write(&path, &out)?;
    // Verify the write actually landed — `std::fs::write` returning `Ok`
    // is not proof the bytes on disk match what was requested (see
    // FEATURE-REQUESTS.md FR-007, which hit this same silent-write-loss
    // shape in `file_edit_batch`).
    let verify = std::fs::read_to_string(&path)?;
    if verify != out {
        bail!("frontmatter update to '{rel}' did not persist (read-back mismatch)");
    }
    Ok(())
}

/// Delete `_trash/YYYY-MM-DD/` folders older than `retention_days`.
/// Returns the number of dated folders removed.
pub fn purge_trash(vault_path: &Path, retention_days: i64) -> Result<u32> {
    let trash = vault_path.join("_trash");
    let Ok(entries) = std::fs::read_dir(&trash) else {
        return Ok(0);
    };
    let cutoff = chrono::Local::now().date_naive() - chrono::Duration::days(retention_days);
    let mut removed = 0u32;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(date) = chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d") else {
            continue;
        };
        if date < cutoff && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let nb = tmp.path().join("Notebooks/Projects");
        std::fs::create_dir_all(&nb).unwrap();
        std::fs::write(
            nb.join("Alpha.md"),
            "# Alpha\n\nSee [[Beta]] and [[Gamma|the gamma note]].",
        )
        .unwrap();
        std::fs::write(nb.join("Beta.md"), "# Beta\n\nBody.").unwrap();
        tmp
    }

    #[test]
    fn move_rewrites_inbound_wikilinks() {
        let tmp = setup();
        let report = move_note(
            tmp.path(),
            "Notebooks/Projects/Beta.md",
            "Notebooks/Projects/Archive/BetaRenamed.md",
            false,
        )
        .unwrap();
        assert_eq!(report.referrers, vec!["Notebooks/Projects/Alpha.md"]);
        assert_eq!(report.links_rewritten, 1);
        assert!(
            tmp.path()
                .join("Notebooks/Projects/Archive/BetaRenamed.md")
                .is_file()
        );
        let alpha =
            std::fs::read_to_string(tmp.path().join("Notebooks/Projects/Alpha.md")).unwrap();
        assert!(alpha.contains("[[BetaRenamed]]"));
    }

    #[test]
    fn dry_run_changes_nothing() {
        let tmp = setup();
        let report = move_note(
            tmp.path(),
            "Notebooks/Projects/Beta.md",
            "Notebooks/Beta2.md",
            true,
        )
        .unwrap();
        assert!(report.dry_run);
        assert_eq!(report.links_rewritten, 0);
        assert!(tmp.path().join("Notebooks/Projects/Beta.md").is_file());
        assert!(!tmp.path().join("Notebooks/Beta2.md").exists());
    }

    #[test]
    fn protected_prefixes_are_denied() {
        let tmp = setup();
        for bad in [
            "_system/SOUL.md",
            "_threads/cli.jsonl.md",
            "_data/x.md",
            "_trash/2026-01-01/x.md",
        ] {
            assert!(move_note(tmp.path(), bad, "Notebooks/x.md", true).is_err());
            assert!(trash_note(tmp.path(), bad, true).is_err());
        }
        assert!(guard_rel_path("../escape.md").is_err());
    }

    #[test]
    fn trash_moves_and_suffixes_duplicates() {
        let tmp = setup();
        let dest = trash_note(tmp.path(), "Notebooks/Projects/Beta.md", false).unwrap();
        assert!(tmp.path().join(&dest).is_file());
        assert!(!tmp.path().join("Notebooks/Projects/Beta.md").exists());

        std::fs::write(
            tmp.path().join("Notebooks/Projects/Beta.md"),
            "# Beta again",
        )
        .unwrap();
        let dest2 = trash_note(tmp.path(), "Notebooks/Projects/Beta.md", false).unwrap();
        assert_ne!(dest, dest2);
        assert!(tmp.path().join(&dest2).is_file());
    }

    #[test]
    fn frontmatter_set_and_remove_roundtrip() {
        let tmp = setup();
        let rel = "Notebooks/Projects/Beta.md";
        let mut set = HashMap::new();
        set.insert(
            "status".to_string(),
            serde_yaml::Value::String("active".into()),
        );
        update_frontmatter(tmp.path(), rel, set, vec![]).unwrap();
        let raw = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert!(raw.contains("status: active"));
        assert!(raw.contains("# Beta"));

        update_frontmatter(tmp.path(), rel, HashMap::new(), vec!["status".into()]).unwrap();
        let raw = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert!(!raw.contains("status: active"));
    }

    #[test]
    fn purge_removes_only_old_dated_folders() {
        let tmp = setup();
        let old = tmp.path().join("_trash/2020-01-01");
        let recent_date = chrono::Local::now().date_naive().format("%Y-%m-%d");
        let recent = tmp.path().join(format!("_trash/{recent_date}"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&recent).unwrap();
        let removed = purge_trash(tmp.path(), 30).unwrap();
        assert_eq!(removed, 1);
        assert!(!old.exists());
        assert!(recent.exists());
    }
}
