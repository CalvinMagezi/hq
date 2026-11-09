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

use crate::rel_path::RelPath;
use crate::remediation;

/// Directories the reorg tools may never touch, in either direction: HQ's private folders, the
/// trash, and the folders whose files become instructions (moving a note into `skills/` makes it
/// a skill; their own tools carry the approval and checks).
const PROTECTED_FOLDERS: &[&str] = &[
    "_system",
    "_threads",
    "_data",
    "_trash",
    "skills",
    "_skills",
    "_commands",
    "agents",
    "_mailboxes",
];

/// How long a deleted note stays in the trash before the daemon purges its day folder.
pub const TRASH_RETENTION_DAYS: i64 = 30;
const TRASH_DIR: &str = "_trash";
const TRASH_DAY_FORMAT: &str = "%Y-%m-%d";
/// The trash also holds repair backups (`_trash/<day>/nested-frontmatter/`); those are not notes
/// the owner deleted, so the Trash shows and restores only what came out of `Notebooks/`.
const TRASHED_NOTES_ROOT: &str = "Notebooks";
const MAX_TRASH_LISTED: usize = 1000;

/// A deleted note as the Trash shows it.
#[derive(Debug, Clone, Serialize)]
pub struct TrashedNote {
    pub trash_path: String,
    pub original_path: String,
    pub trashed_on: String,
    pub days_left: i64,
}

/// Where a note lived before it was deleted, and where a restore put it. They differ when
/// something else has taken the old name.
#[derive(Debug, Clone, Serialize)]
pub struct RestoredNote {
    pub original_path: String,
    pub path: String,
}

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

/// The path in its one spelling, refused when it is not a plain path inside the vault or when
/// it is in a protected folder however it is written (`./_system/x`, `_System/x`).
fn guard_rel_path(rel: &str) -> Result<String> {
    let parsed = RelPath::parse(rel)?;
    if parsed.is_root() {
        bail!("invalid vault path '{rel}': names no note");
    }
    if let Some(p) = parsed.top_in(PROTECTED_FOLDERS) {
        bail!("'{rel}' is protected: {p}/ cannot be reorganized by tools");
    }
    Ok(parsed.normalized())
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
pub(crate) fn move_note(
    vault_path: &Path,
    from: &str,
    to: &str,
    dry_run: bool,
) -> Result<MoveReport> {
    let from = &guard_rel_path(from)?;
    let to = &guard_rel_path(to)?;
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
    let rel = &guard_rel_path(rel)?;
    require_md(rel)?;
    let src = vault_path.join(rel);
    if !src.is_file() {
        bail!("note '{rel}' does not exist");
    }
    let date = chrono::Local::now().format(TRASH_DAY_FORMAT);
    let dest_rel = format!("{TRASH_DIR}/{date}/{rel}");
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

/// The notes in the trash, newest day first.
pub(crate) fn list_trash(vault_path: &Path) -> Result<Vec<TrashedNote>> {
    let Ok(days) = std::fs::read_dir(vault_path.join(TRASH_DIR)) else {
        return Ok(Vec::new());
    };
    let today = chrono::Local::now().date_naive();
    let mut notes = Vec::new();
    for day in days.filter_map(|e| e.ok()) {
        let name = day.file_name().to_string_lossy().into_owned();
        let Ok(date) = chrono::NaiveDate::parse_from_str(&name, TRASH_DAY_FORMAT) else {
            continue;
        };
        let day_dir = day.path();
        let days_left = (date + chrono::Duration::days(TRASH_RETENTION_DAYS) - today).num_days();
        for inner in notes_under(&day_dir, TRASHED_NOTES_ROOT) {
            notes.push(TrashedNote {
                trash_path: format!("{TRASH_DIR}/{name}/{inner}"),
                original_path: original_of(&day_dir, &inner),
                trashed_on: name.clone(),
                days_left: days_left.max(0),
            });
        }
    }
    notes.sort_by(|a, b| {
        b.trashed_on
            .cmp(&a.trashed_on)
            .then_with(|| a.original_path.cmp(&b.original_path))
    });
    notes.truncate(MAX_TRASH_LISTED);
    Ok(notes)
}

/// Paths of the `.md` files below `base/sub`, relative to `base`. Symlinks are not followed.
fn notes_under(base: &Path, sub: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![base.join(sub)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file()
                && path.extension().is_some_and(|e| e == "md")
                && let Ok(rel) = path.strip_prefix(base)
            {
                found.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found
}

/// Where a trashed file came from. `trash_note` numbers a copy only when the plain name is taken
/// that day, so a number with no plain sibling is part of the note's real name. Once the plain
/// copy is restored, a numbered one comes back with its number, which loses nothing.
fn original_of(day_dir: &Path, inner: &str) -> String {
    let plain = inner
        .strip_suffix(".md")
        .and_then(|s| s.rsplit_once('.'))
        .filter(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .map(|(base, _)| format!("{base}.md"));
    match plain {
        Some(plain) if day_dir.join(&plain).exists() => plain,
        _ => inner.to_string(),
    }
}

/// A checked trash path: its one spelling and the part below the day folder. Refuses anything
/// that is not a real file under `_trash/<day>/Notebooks/`, so a link planted in the trash
/// cannot be read or restored.
fn checked_trash_path(vault_path: &Path, rel: &str) -> Result<(String, String, String)> {
    let normalized = RelPath::parse(rel)?.normalized();
    let mut parts = normalized.splitn(3, '/');
    let (Some(TRASH_DIR), Some(day), Some(inner)) = (parts.next(), parts.next(), parts.next())
    else {
        bail!("'{rel}' is not a note in the trash");
    };
    if chrono::NaiveDate::parse_from_str(day, TRASH_DAY_FORMAT).is_err()
        || !inner.starts_with(&format!("{TRASHED_NOTES_ROOT}/"))
    {
        bail!("'{rel}' is not a note in the trash");
    }
    require_md(inner)?;
    let src = vault_path.join(&normalized);
    if !std::fs::symlink_metadata(&src)?.is_file() {
        bail!("'{rel}' is not a note in the trash");
    }
    let trash_root = vault_path.join(TRASH_DIR).canonicalize()?;
    if !src.canonicalize()?.starts_with(&trash_root) {
        bail!("'{rel}' is not a note in the trash");
    }
    let (day, inner) = (day.to_string(), inner.to_string());
    Ok((normalized, day, inner))
}

/// The text of a note in the trash.
pub(crate) fn read_trashed(vault_path: &Path, trash_rel: &str) -> Result<String> {
    let (normalized, _, _) = checked_trash_path(vault_path, trash_rel)?;
    Ok(std::fs::read_to_string(vault_path.join(normalized))?)
}

/// Refuses a destination whose real folder is outside the vault or in a protected one, however
/// a symlink inside the vault spells it.
fn require_safe_parent(vault_path: &Path, dest: &Path) -> Result<()> {
    let root = vault_path.canonicalize()?;
    let mut probe = dest
        .parent()
        .ok_or_else(|| anyhow::anyhow!("the destination has no folder"))?;
    while !probe.exists() {
        probe = probe
            .parent()
            .ok_or_else(|| anyhow::anyhow!("the destination has no folder"))?;
    }
    let real = probe.canonicalize()?;
    let inside = real
        .strip_prefix(&root)
        .map_err(|_| anyhow::anyhow!("the destination is outside the vault"))?;
    let top = inside
        .components()
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase());
    if top.is_some_and(|t| PROTECTED_FOLDERS.iter().any(|p| p.to_lowercase() == t)) {
        bail!("the destination is in a protected folder");
    }
    Ok(())
}

/// `rel`, or the first "<name> (restored).md" that is free, so a restore never replaces a note.
fn free_restore_path(vault_path: &Path, rel: &str) -> String {
    let taken = |p: &str| std::fs::symlink_metadata(vault_path.join(p)).is_ok();
    if !taken(rel) {
        return rel.to_string();
    }
    let base = rel.trim_end_matches(".md");
    let mut candidate = format!("{base} (restored).md");
    let mut n = 2;
    while taken(&candidate) {
        candidate = format!("{base} (restored {n}).md");
        n += 1;
    }
    candidate
}

/// Put a trashed note back where it was.
pub(crate) fn restore_note(vault_path: &Path, trash_rel: &str) -> Result<RestoredNote> {
    let (normalized, day, inner) = checked_trash_path(vault_path, trash_rel)?;
    let day_dir = vault_path.join(TRASH_DIR).join(day);
    let original = guard_rel_path(&original_of(&day_dir, &inner))?;
    let dest_rel = free_restore_path(vault_path, &original);
    let dest = vault_path.join(&dest_rel);
    require_safe_parent(vault_path, &dest)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(vault_path.join(normalized), &dest)?;
    Ok(RestoredNote {
        original_path: original,
        path: dest_rel,
    })
}

/// Set frontmatter keys and put `prefix` above the body of `raw`, in one atomic write, so a
/// failure leaves the note as it was.
pub(crate) fn prepend_to_note(
    vault_path: &Path,
    rel: &str,
    raw: &str,
    set: HashMap<String, serde_yaml::Value>,
    prefix: &str,
) -> Result<()> {
    let rel = &guard_rel_path(rel)?;
    require_md(rel)?;
    let (mut fm, body) = crate::frontmatter::parse(raw)?;
    fm.extend(set);
    let out = crate::frontmatter::serialize(&fm, &format!("{prefix}{body}"))?;
    crate::notes::write_atomic(&vault_path.join(rel), out.as_bytes())
}

/// Set and/or remove frontmatter keys on a note, preserving the body.
pub(crate) fn update_frontmatter(
    vault_path: &Path,
    rel: &str,
    set: HashMap<String, serde_yaml::Value>,
    remove: Vec<String>,
) -> Result<()> {
    let rel = &guard_rel_path(rel)?;
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

    fn write(vault: &Path, rel: &str, text: &str) {
        let path = vault.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

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

    /// The same protected file written another way is still protected: a model that could
    /// replace `_system/SOUL.md` through `./_system/SOUL.md` would rewrite the system prompt of
    /// every later session.
    #[test]
    fn protected_folders_hold_however_the_path_is_spelled() {
        let tmp = setup();
        let sys = tmp.path().join("_system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(sys.join("SOUL.md"), "# Soul").unwrap();
        for bad in [
            "./_system/SOUL.md",
            ".//./_system/SOUL.md",
            "_system//SOUL.md",
            "_System/SOUL.md",
            "./skills/x/SKILL.md",
            "Skills/x/SKILL.md",
            "_commands/deploy.md",
            "Agents/coder.md",
            "_mailboxes/relay/m.md",
        ] {
            assert!(
                move_note(tmp.path(), "Notebooks/Projects/Beta.md", bad, true).is_err(),
                "move to {bad}"
            );
            assert!(
                move_note(tmp.path(), bad, "Notebooks/x.md", true).is_err(),
                "move from {bad}"
            );
            assert!(trash_note(tmp.path(), bad, true).is_err(), "trash {bad}");
            assert!(
                update_frontmatter(tmp.path(), bad, HashMap::new(), vec![]).is_err(),
                "frontmatter {bad}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(sys.join("SOUL.md")).unwrap(),
            "# Soul"
        );
        // Ordinary notes still move however they are written.
        let report = move_note(
            tmp.path(),
            "./Notebooks/Projects/Beta.md",
            "Notebooks//Beta.md",
            false,
        )
        .unwrap();
        assert_eq!(report.from, "Notebooks/Projects/Beta.md");
        assert_eq!(report.to, "Notebooks/Beta.md");
        assert!(tmp.path().join("Notebooks/Beta.md").is_file());
    }

    #[test]
    fn restore_brings_a_note_back_and_never_replaces_one() {
        let tmp = setup();
        let trash = trash_note(tmp.path(), "Notebooks/Projects/Beta.md", false).unwrap();
        let listed = list_trash(tmp.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].original_path, "Notebooks/Projects/Beta.md");
        assert_eq!(listed[0].trash_path, trash);
        assert_eq!(listed[0].days_left, TRASH_RETENTION_DAYS);
        assert!(read_trashed(tmp.path(), &trash).is_ok());

        write(
            tmp.path(),
            "Notebooks/Projects/Beta.md",
            "a new note took the name",
        );
        let back = restore_note(tmp.path(), &trash).unwrap();
        assert_eq!(back.path, "Notebooks/Projects/Beta (restored).md");
        assert_eq!(back.original_path, "Notebooks/Projects/Beta.md");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("Notebooks/Projects/Beta.md")).unwrap(),
            "a new note took the name"
        );
        assert!(list_trash(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn a_numbered_copy_restores_under_its_plain_name_and_a_real_number_stays() {
        let tmp = setup();
        trash_note(tmp.path(), "Notebooks/Projects/Beta.md", false).unwrap();
        write(tmp.path(), "Notebooks/Projects/Beta.md", "second");
        let numbered = trash_note(tmp.path(), "Notebooks/Projects/Beta.md", false).unwrap();
        assert!(numbered.ends_with("Beta.1.md"));
        write(tmp.path(), "Notebooks/v2.1.md", "real number");
        let real = trash_note(tmp.path(), "Notebooks/v2.1.md", false).unwrap();

        assert_eq!(
            restore_note(tmp.path(), &numbered).unwrap().path,
            "Notebooks/Projects/Beta.md"
        );
        assert_eq!(
            restore_note(tmp.path(), &real).unwrap().path,
            "Notebooks/v2.1.md"
        );
    }

    #[test]
    fn restore_refuses_anything_but_a_real_note_from_notebooks_in_the_trash() {
        let tmp = setup();
        let day = chrono::Local::now().format(TRASH_DAY_FORMAT).to_string();
        write(tmp.path(), &format!("_trash/{day}/skills/evil.md"), "x");
        write(
            tmp.path(),
            &format!("_trash/{day}/nested-frontmatter/Notebooks/a.md"),
            "x",
        );
        write(tmp.path(), "Notebooks/live.md", "x");
        for bad in [
            "Notebooks/live.md".to_string(),
            format!("_trash/{day}/skills/evil.md"),
            format!("_trash/{day}/nested-frontmatter/Notebooks/a.md"),
            format!("_trash/{day}/../../Notebooks/live.md"),
            "_trash/not-a-day/Notebooks/live.md".to_string(),
            "/etc/hosts".to_string(),
            "../outside.md".to_string(),
            format!("_trash/{day}/Notebooks/missing.md"),
        ] {
            assert!(restore_note(tmp.path(), &bad).is_err(), "restore {bad}");
            assert!(read_trashed(tmp.path(), &bad).is_err(), "read {bad}");
        }
        assert!(list_trash(tmp.path()).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn restore_will_not_follow_a_link_into_a_protected_folder_or_out_of_the_trash() {
        let tmp = setup();
        let day = chrono::Local::now().format(TRASH_DAY_FORMAT).to_string();
        std::fs::create_dir_all(tmp.path().join("skills")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("skills"), tmp.path().join("Notebooks/link"))
            .unwrap();
        write(
            tmp.path(),
            &format!("_trash/{day}/Notebooks/link/evil.md"),
            "x",
        );
        let trash = format!("_trash/{day}/Notebooks/link/evil.md");
        assert!(restore_note(tmp.path(), &trash).is_err());
        assert!(!tmp.path().join("skills/evil.md").exists());

        write(tmp.path(), "Notebooks/secret.md", "x");
        let planted = format!("_trash/{day}/Notebooks/planted.md");
        std::os::unix::fs::symlink(
            tmp.path().join("Notebooks/secret.md"),
            tmp.path().join(&planted),
        )
        .unwrap();
        assert!(read_trashed(tmp.path(), &planted).is_err());
        assert!(restore_note(tmp.path(), &planted).is_err());
    }

    #[test]
    fn prepend_writes_body_and_keys_together_and_refuses_protected_folders() {
        let tmp = setup();
        write(tmp.path(), "Notebooks/n.md", "---\nkept: 1\n---\nbody");
        let raw = std::fs::read_to_string(tmp.path().join("Notebooks/n.md")).unwrap();
        let set = HashMap::from([("source_url".to_string(), "https://example.com".into())]);
        prepend_to_note(tmp.path(), "Notebooks/n.md", &raw, set.clone(), "top\n\n").unwrap();
        let out = std::fs::read_to_string(tmp.path().join("Notebooks/n.md")).unwrap();
        assert!(
            out.contains("kept: 1") && out.contains("source_url"),
            "{out}"
        );
        assert!(out.contains("top\n\nbody"), "{out}");
        assert!(prepend_to_note(tmp.path(), "skills/x.md", "", set, "t").is_err());
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
