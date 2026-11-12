//! Skills HQ ships in its binary. A built-in is active when the process configured it
//! ([`configure`]): its prerequisites hold and the owner did not turn it off. An active built-in
//! is always loaded from the text embedded here, so nothing written into the vault changes what
//! runs; the vault copy is a mirror for people to read. An inactive one releases its name, which
//! then behaves like any skill the owner writes.

use std::fs;
use std::path::Path;
use std::sync::RwLock;

use anyhow::{Context, Result, bail};
use hq_core::config::HqConfig;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::skill_audit::{Disposition, archive_current, audit_live};

const SKILL_FILE: &str = "SKILL.md";
/// Kept when a copy is brought back in step, so earlier versions stay recoverable.
const ARCHIVE_DIR: &str = "archive";
/// Sync's own temporary files; another process's are left alone mid-write.
const TMP_PREFIX: &str = ".builtin-";
const TMP_SUFFIX: &str = ".tmp";
/// The audit reason for a mirror removed because its built-in stopped being active.
const NO_LONGER_ACTIVE: &str = "no longer active";

/// One shipped skill: its name, every file it is made of, and when it applies.
pub struct Shipped {
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static str)],
    /// Whether this install can use the skill; one that cannot keeps it out of every prompt.
    pub applies: fn(&HqConfig) -> bool,
}

/// The skill `calendar_create_event` refuses to run without.
pub const CALENDAR_CREATION_SKILL: &str = "calendar-creation";

/// The skill `calendar_list_events` refuses to run without.
pub const CALENDAR_READING_SKILL: &str = "calendar-reading";

/// The skill `calendar_update_event` and `calendar_delete_event` refuse to run without.
pub const CALENDAR_EDITING_SKILL: &str = "calendar-editing";

/// The set is decided once at start, and GitHub can be connected any time after, so the worker
/// is always offered. Whether it acts is `dispatch::allowed_repos`.
fn every_install(_: &HqConfig) -> bool {
    true
}

/// Every built-in skill. A test checks this list against `assets/skills/`.
pub const BUILTIN_SKILLS: &[Shipped] = &[
    Shipped {
        name: "edge-coding-worker",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/edge-coding-worker/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-task-brief",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-task-brief/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-engineering-core",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-engineering-core/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-engineering-edge",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-engineering-edge/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: "hq-wayfinder",
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/hq-wayfinder/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: CALENDAR_CREATION_SKILL,
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/calendar-creation/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: CALENDAR_READING_SKILL,
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/calendar-reading/SKILL.md"),
        )],
        applies: every_install,
    },
    Shipped {
        name: CALENDAR_EDITING_SKILL,
        files: &[(
            SKILL_FILE,
            include_str!("../../assets/skills/calendar-editing/SKILL.md"),
        )],
        applies: every_install,
    },
];

/// The active built-ins, set once from the config the process loaded. `None` until
/// [`configure`] runs: nothing is active, so a library user or a test sees no built-in by default.
static ACTIVE: RwLock<Option<Vec<&'static str>>> = RwLock::new(None);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// Per-thread override for tests, so each test decides what is active without a global.
    static TEST_ACTIVE: std::cell::RefCell<Option<Vec<&'static str>>> =
        const { std::cell::RefCell::new(None) };
}

/// Decides which built-ins this process offers, from the config it loaded. Returns the names in
/// `skills.builtin.off` that no built-in has, which are most likely typos.
pub fn configure(config: &HqConfig) -> Vec<String> {
    let active = active_for(config);
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(active);
    unknown_off(config)
}

/// The built-ins `config` makes active.
pub fn active_for(config: &HqConfig) -> Vec<&'static str> {
    let off = &config.skills.builtin.off;
    BUILTIN_SKILLS
        .iter()
        .filter(|s| (s.applies)(config) && !off.iter().any(|o| o.eq_ignore_ascii_case(s.name)))
        .map(|s| s.name)
        .collect()
}

fn unknown_off(config: &HqConfig) -> Vec<String> {
    let off = &config.skills.builtin.off;
    off.iter()
        .filter(|o| shipped(o).is_none())
        .cloned()
        .collect()
}

/// Sets what is active for the calling thread until the returned guard is dropped. For tests,
/// here and in crates that build sessions (feature `test-support`).
#[cfg(any(test, feature = "test-support"))]
pub fn set_active_for_this_thread(names: Option<&[&'static str]>) -> ActiveGuard {
    let previous = TEST_ACTIVE.with(|t| t.replace(names.map(<[&str]>::to_vec)));
    ActiveGuard(previous)
}

/// Restores the calling thread's previous override when dropped.
#[cfg(any(test, feature = "test-support"))]
pub struct ActiveGuard(Option<Vec<&'static str>>);

#[cfg(any(test, feature = "test-support"))]
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        TEST_ACTIVE.with(|t| *t.borrow_mut() = previous);
    }
}

fn active_names() -> Vec<&'static str> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(names) = TEST_ACTIVE.with(|t| t.borrow().clone()) {
        return names;
    }
    ACTIVE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Names compare without case: on a case-insensitive file system `Edge-Coding-Worker` is the
/// same folder as `edge-coding-worker`, so a variant must never escape the built-in rules.
fn shipped(name: &str) -> Option<&'static Shipped> {
    BUILTIN_SKILLS
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
}

/// Whether `name` is an active built-in: loaded from HQ's text and closed to every change.
pub fn is_active(name: &str) -> bool {
    shipped(name).is_some_and(|s| active_names().contains(&s.name))
}

/// The active built-in's own spelling of `name`.
pub fn canonical_name(name: &str) -> Option<&'static str> {
    shipped(name).filter(|s| is_active(s.name)).map(|s| s.name)
}

/// The active built-ins, in the order they ship.
pub fn active_builtins() -> impl Iterator<Item = &'static Shipped> {
    let active = active_names();
    BUILTIN_SKILLS
        .iter()
        .filter(move |s| active.contains(&s.name))
}

/// The embedded text of one of an active built-in's files.
pub fn shipped_file(name: &str, file: &str) -> Option<&'static str> {
    canonical_name(name)?;
    shipped(name)?
        .files
        .iter()
        .find(|(f, _)| *f == file)
        .map(|(_, text)| *text)
}

/// SHA-256 of an active built-in's embedded SKILL.md: what a session actually ran.
pub fn shipped_hash(name: &str) -> Option<String> {
    shipped_file(name, SKILL_FILE).map(|text| hex::encode(Sha256::digest(text.as_bytes())))
}

/// A skill file as a session should see it: the embedded text for an active built-in, never what
/// its vault copy holds; the vault file for any other skill.
pub fn read_skill_file(skills_dir: &Path, name: &str, file: &str) -> Option<String> {
    if is_active(name) {
        return shipped_file(name, file).map(str::to_string);
    }
    fs::read_to_string(skills_dir.join(name).join(file)).ok()
}

/// Refuses a change to an active built-in, saying what to do instead.
pub fn refuse_builtin(name: &str, action: &str) -> Result<()> {
    if let Some(canonical) = canonical_name(name) {
        bail!(
            "{canonical} ships with HQ, so it cannot be {action}; to change it, copy it under \
             another name, or add {canonical} to skills.builtin.off in the config to use your own"
        );
    }
    Ok(())
}

/// What one sync did.
#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct SyncReport {
    pub installed: Vec<String>,
    /// Copies that differed from the shipped text (an older HQ, or an edit) and were replaced;
    /// the previous SKILL.md is in the skill's `archive/`.
    pub replaced: Vec<String>,
    /// Mirrors of built-ins that are no longer active, removed. A folder of the owner's own under
    /// that name is never touched.
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
}

/// Brings the vault mirror of every built-in in step: written for an active one (`active`),
/// replaced when it differs, removed when the built-in is no longer active.
pub fn sync_builtin_skills(skills_dir: &Path, active: &[&str]) -> Result<SyncReport> {
    fs::create_dir_all(skills_dir)?;
    let mut report = SyncReport::default();
    for skill in BUILTIN_SKILLS {
        let name = skill.name.to_string();
        let dir = skills_dir.join(skill.name);
        if !active.contains(&skill.name) {
            if remove_mirror(skills_dir, skill)? {
                report.removed.push(name);
            }
            continue;
        }
        if in_step(&dir, skill)? {
            report.unchanged.push(name);
            continue;
        }
        let had_copy = has_copy(&dir)?;
        write_copy(skills_dir, skill)?;
        let reason = if had_copy {
            "brought in step with HQ"
        } else {
            "installed by HQ"
        };
        audit_live(skills_dir, skill.name, Disposition::Synced, reason)?;
        match had_copy {
            true => report.replaced.push(name),
            false => report.installed.push(name),
        }
    }
    Ok(report)
}

/// Removes, from a built-in that is no longer active, only the files that are exactly a text HQ
/// shipped: this binary's, or one an earlier sync recorded. Anything else in the folder, such
/// as an owner's own skill under the released name, is left as it is. Returns whether a mirror
/// was removed.
fn remove_mirror(skills_dir: &Path, skill: &Shipped) -> Result<bool> {
    let dir = skills_dir.join(skill.name);
    let live = dir.join(SKILL_FILE);
    let Some(bytes) = fs::symlink_metadata(&live)
        .ok()
        .filter(|m| m.is_file())
        .and_then(|_| fs::read(&live).ok())
    else {
        return Ok(false);
    };
    let hash = hex::encode(Sha256::digest(&bytes));
    if !shipped_hashes(skills_dir, skill).contains(&hash) {
        return Ok(false);
    }
    audit_live(
        skills_dir,
        skill.name,
        Disposition::Synced,
        NO_LONGER_ACTIVE,
    )?;
    for (file, _) in skill.files {
        let _ = fs::remove_file(dir.join(file));
    }
    // Only succeeds when nothing else is left; an archive or the owner's files keep the folder.
    let _ = fs::remove_dir(&dir);
    Ok(true)
}

/// The SKILL.md hashes HQ is known to have written for `skill`: this binary's and every earlier
/// sync's from the audit trail.
fn shipped_hashes(skills_dir: &Path, skill: &Shipped) -> Vec<String> {
    let current = skill.files.iter().find(|(f, _)| *f == SKILL_FILE);
    let mut hashes: Vec<String> = current
        .map(|(_, text)| hex::encode(Sha256::digest(text.as_bytes())))
        .into_iter()
        .collect();
    let synced = crate::skill_audit::audit_history(skills_dir, skill.name)
        .into_iter()
        .filter(|e| e.disposition == Disposition::Synced && e.reason != NO_LONGER_ACTIVE)
        .map(|e| e.sha256);
    hashes.extend(synced);
    hashes
}

fn write_copy(skills_dir: &Path, skill: &Shipped) -> Result<()> {
    let dir = skills_dir.join(skill.name);
    let keep: Vec<&str> = skill.files.iter().map(|(f, _)| *f).collect();
    clear_copy(skills_dir, skill, &keep)?;
    fs::create_dir_all(&dir)?;
    for (file, text) in skill.files {
        write_atomic(&dir.join(file), text)?;
    }
    Ok(())
}

fn is_sync_tmp(name: &str) -> bool {
    name.starts_with(TMP_PREFIX) && name.ends_with(TMP_SUFFIX)
}

/// Whether anything but an archive or a sync's temporary file is where the copy would be.
fn has_copy(dir: &Path) -> Result<bool> {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return Ok(false);
    };
    if !meta.is_dir() {
        return Ok(true);
    }
    Ok(fs::read_dir(dir)?.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        name != ARCHIVE_DIR && !is_sync_tmp(&name)
    }))
}

/// Whether the copy holds exactly the shipped files as plain files, and nothing else but its
/// archive and another sync's temporary files.
fn in_step(dir: &Path, skill: &Shipped) -> Result<bool> {
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => {}
        _ => return Ok(false),
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type()?;
        let ok = match skill.files.iter().find(|(f, _)| *f == name) {
            Some((_, text)) => kind.is_file() && fs::read(entry.path())? == text.as_bytes(),
            None => (name == ARCHIVE_DIR && kind.is_dir()) || is_sync_tmp(&name),
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(skill.files.iter().all(|(f, _)| dir.join(f).is_file()))
}

/// Whether the plain SKILL.md in `dir` already is the shipped text, so archiving it keeps nothing.
fn holds_shipped_skill(dir: &Path, skill: &Shipped) -> bool {
    let shipped = skill
        .files
        .iter()
        .find(|(f, _)| *f == SKILL_FILE)
        .map(|(_, t)| *t);
    let live = dir.join(SKILL_FILE);
    let plain = fs::symlink_metadata(&live).is_ok_and(|m| m.is_file());
    plain && shipped.is_some_and(|t| fs::read(&live).is_ok_and(|b| b == t.as_bytes()))
}

/// Removes everything in a built-in's folder except its archive, another sync's temporary files
/// and the `keep` files that are plain files, archiving a plain SKILL.md first unless it already
/// is the shipped text. Links are removed, never followed or archived.
fn clear_copy(skills_dir: &Path, skill: &Shipped, keep: &[&str]) -> Result<()> {
    let dir = skills_dir.join(skill.name);
    let Ok(meta) = fs::symlink_metadata(&dir) else {
        return Ok(());
    };
    if !meta.is_dir() {
        fs::remove_file(&dir).with_context(|| format!("cannot remove {}", dir.display()))?;
        return Ok(());
    }
    let archive = dir.join(ARCHIVE_DIR);
    if fs::symlink_metadata(&archive).is_ok_and(|m| !m.is_dir()) {
        fs::remove_file(&archive)?;
    }
    if !holds_shipped_skill(&dir, skill) {
        archive_current(skills_dir, skill.name)?;
    }
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let file = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type()?;
        let kept = keep.contains(&file.as_str()) && kind.is_file();
        if kept || (file == ARCHIVE_DIR && kind.is_dir()) || is_sync_tmp(&file) {
            continue;
        }
        match kind.is_dir() {
            true => fs::remove_dir_all(entry.path())?,
            false => fs::remove_file(entry.path())?,
        }
    }
    if keep.is_empty() {
        // Only an archive may be left; an empty folder is removed, a folder with one is kept.
        let _ = fs::remove_dir(&dir);
    }
    Ok(())
}

/// Writes `text` through a temporary file in the same folder, so a reader never sees half a file
/// and two HQ processes starting together cannot interleave.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().context("a skill file has no folder")?;
    let tmp = dir.join(format!(
        "{TMP_PREFIX}{}{TMP_SUFFIX}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::write(&tmp, text)?;
    if let Ok(meta) = fs::symlink_metadata(path)
        && meta.is_dir()
    {
        fs::remove_dir_all(path)?;
    }
    fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "builtin_tests.rs"]
mod tests;
