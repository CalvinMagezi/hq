//! Versioned archive, append-only audit trail and revert for live skills.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::skills::{Severity, parse_skill, validate_skill, validate_skill_name};

/// One JSON line per adoption, hold, rejection, revert or delete, at the skills root.
pub const AUDIT_FILE: &str = "_audit.jsonl";
const ARCHIVE_DIR: &str = "archive";
const PROPOSED_DIR: &str = "_proposed";
const SKILL_FILE: &str = "SKILL.md";
const ARCHIVE_PREFIX: &str = "SKILL-v";
const ARCHIVE_SUFFIX: &str = ".md";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Disposition {
    Adopted,
    Held,
    /// Refused outright: nothing written or staged.
    Rejected,
    Reverted,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: String,
    pub name: String,
    pub version: u32,
    pub sha256: String,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub flags: Vec<String>,
    pub disposition: Disposition,
}

impl AuditEntry {
    pub fn new(name: &str, version: u32, content: &str, disposition: Disposition) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            name: name.to_string(),
            version,
            sha256: hex::encode(Sha256::digest(content.as_bytes())),
            run_id: None,
            reason: String::new(),
            flags: Vec::new(),
            disposition,
        }
    }
}

pub fn append_audit(skills_dir: &Path, entry: &AuditEntry) -> Result<()> {
    std::fs::create_dir_all(skills_dir)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(skills_dir.join(AUDIT_FILE))?;
    writeln!(file, "{}", serde_json::to_string(entry)?)?;
    Ok(())
}

/// Audit the skill as it now stands: the `_proposed/` copy when held, the live
/// one otherwise. A deleted skill hashes as empty.
pub fn audit_live(skills_dir: &Path, name: &str, disposition: Disposition, reason: &str) -> Result<()> {
    let root = match disposition {
        Disposition::Held => skills_dir.join(PROPOSED_DIR),
        _ => skills_dir.to_path_buf(),
    };
    let content = std::fs::read_to_string(root.join(name).join(SKILL_FILE)).unwrap_or_default();
    let version = parse_skill(&root, name).map_or(0, |s| s.provenance.version);
    let mut entry = AuditEntry::new(name, version, &content, disposition);
    entry.reason = reason.to_string();
    append_audit(skills_dir, &entry)
}

/// Every audit line for `name`, oldest first. Malformed lines are skipped.
pub fn audit_history(skills_dir: &Path, name: &str) -> Vec<AuditEntry> {
    let Ok(raw) = std::fs::read_to_string(skills_dir.join(AUDIT_FILE)) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<AuditEntry>(l).ok())
        .filter(|e| e.name == name)
        .collect()
}

/// Archived `SKILL-vN.md` files for `name`, lowest N first.
pub fn archived_versions(skills_dir: &Path, name: &str) -> Vec<(u32, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(skills_dir.join(name).join(ARCHIVE_DIR)) else {
        return Vec::new();
    };
    let mut out: Vec<(u32, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().to_string();
            let n = file.strip_prefix(ARCHIVE_PREFIX)?.strip_suffix(ARCHIVE_SUFFIX)?.parse().ok()?;
            Some((n, e.path()))
        })
        .collect();
    out.sort();
    out
}

/// Copy the live SKILL.md to the next free `archive/SKILL-vN.md`. Numbered by
/// archive count, not provenance, because hand-written skills never bump theirs.
pub fn archive_current(skills_dir: &Path, name: &str) -> Result<Option<PathBuf>> {
    validate_skill_name(name)?;
    let live = skills_dir.join(name).join(SKILL_FILE);
    if !live.exists() {
        return Ok(None);
    }
    let next = archived_versions(skills_dir, name).last().map_or(1, |(n, _)| n + 1);
    let dir = skills_dir.join(name).join(ARCHIVE_DIR);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("{ARCHIVE_PREFIX}{next}{ARCHIVE_SUFFIX}"));
    std::fs::copy(&live, &dest)?;
    Ok(Some(dest))
}

/// Undo a live write: restore the archived copy, or drop a SKILL.md that did not exist before.
pub fn rollback(skills_dir: &Path, name: &str, archived: Option<PathBuf>) -> Result<()> {
    let live = skills_dir.join(name).join(SKILL_FILE);
    match archived {
        Some(prev) => std::fs::rename(prev, live)?,
        None => {
            std::fs::remove_file(live)?;
            // Only succeeds when empty, so a deleted skill's archive survives.
            let _ = std::fs::remove_dir(skills_dir.join(name));
        }
    }
    Ok(())
}

/// Keep a just-written live skill only if it validates without errors.
pub fn keep_if_valid(skills_dir: &Path, name: &str, archived: Option<PathBuf>) -> Result<()> {
    let errors: Vec<String> = validate_skill(skills_dir, name)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.message)
        .collect();
    if errors.is_empty() {
        return Ok(());
    }
    rollback(skills_dir, name, archived)?;
    bail!("skill {name:?} failed validation and was reverted: {}", errors.join("; "))
}

/// Archive the live skill, then remove everything but its archive.
pub fn delete_live_skill(skills_dir: &Path, name: &str, reason: &str) -> Result<()> {
    let dir = skills_dir.join(name);
    if !dir.join(SKILL_FILE).exists() {
        bail!("skill does not exist: {}", dir.display());
    }
    archive_current(skills_dir, name)?;
    for entry in std::fs::read_dir(&dir)?.flatten() {
        match entry.path() {
            p if p.file_name().is_some_and(|f| f == ARCHIVE_DIR) => {}
            p if p.is_dir() => std::fs::remove_dir_all(p)?,
            p => std::fs::remove_file(p)?,
        }
    }
    audit_live(skills_dir, name, Disposition::Deleted, reason)
}

/// Put the newest archived version back in place, archiving the current one
/// first so the revert itself can be undone.
pub fn revert_skill(skills_dir: &Path, name: &str) -> Result<PathBuf> {
    validate_skill_name(name)?;
    let Some((version, target)) = archived_versions(skills_dir, name).pop() else {
        bail!("skill '{name}' has no archived version to revert to");
    };
    archive_current(skills_dir, name)?;
    let live = skills_dir.join(name).join(SKILL_FILE);
    std::fs::rename(&target, &live)?;
    audit_live(skills_dir, name, Disposition::Reverted, &format!("restored archive v{version}"))?;
    Ok(live)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(skills_dir: &Path, name: &str, body: &str) {
        let dir = skills_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SKILL_FILE), format!("---\ndescription: \"d\"\n---\n{body}")).unwrap();
    }

    fn live(skills_dir: &Path, name: &str) -> String {
        std::fs::read_to_string(skills_dir.join(name).join(SKILL_FILE)).unwrap()
    }

    #[test]
    fn archives_number_upward_from_one() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "s", "one");
        archive_current(dir.path(), "s").unwrap();
        archive_current(dir.path(), "s").unwrap();
        let versions: Vec<u32> = archived_versions(dir.path(), "s").into_iter().map(|(n, _)| n).collect();
        assert_eq!(versions, vec![1, 2]);
        assert!(archive_current(dir.path(), "missing").unwrap().is_none());
    }

    #[test]
    fn revert_restores_the_newest_archive_and_can_be_undone() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "s", "old body");
        archive_current(dir.path(), "s").unwrap();
        write(dir.path(), "s", "new body");

        revert_skill(dir.path(), "s").unwrap();
        assert!(live(dir.path(), "s").contains("old body"));

        revert_skill(dir.path(), "s").unwrap();
        assert!(live(dir.path(), "s").contains("new body"));

        let history = audit_history(dir.path(), "s");
        assert_eq!(history.len(), 2);
        assert!(history.iter().all(|e| e.disposition == Disposition::Reverted));
        assert_eq!(history[0].sha256.len(), 64);
    }

    #[test]
    fn revert_without_an_archive_fails_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "s", "only");
        assert!(revert_skill(dir.path(), "s").is_err());
        assert!(live(dir.path(), "s").contains("only"));
        assert!(revert_skill(dir.path(), "../x").is_err());
    }

    #[test]
    fn a_deleted_skill_can_be_reverted() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "s", "keep me");
        std::fs::write(dir.path().join("s").join("SUMMARY.md"), "sum").unwrap();

        delete_live_skill(dir.path(), "s", "test").unwrap();
        assert!(!dir.path().join("s").join(SKILL_FILE).exists());
        assert!(!dir.path().join("s").join("SUMMARY.md").exists());
        assert!(parse_skill(dir.path(), "s").is_none());

        revert_skill(dir.path(), "s").unwrap();
        assert!(live(dir.path(), "s").contains("keep me"));
        let dispositions: Vec<Disposition> =
            audit_history(dir.path(), "s").into_iter().map(|e| e.disposition).collect();
        assert_eq!(dispositions, vec![Disposition::Deleted, Disposition::Reverted]);
    }
}
