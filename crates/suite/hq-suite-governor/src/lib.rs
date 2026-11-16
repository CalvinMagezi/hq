//! Keeps the suite's storage in check.
//!
//! One pass, [`govern`], does four things in a fixed order, and each is safe to repeat:
//!
//! 1. Notes whose original has left the trash (the vault's 30-day clean-up took it) are marked
//!    `suite_original_state: gone`, so the note stops promising a file that is not there.
//! 2. Pictures in the media store that no note links to, and no unfinished job needs, are
//!    removed. Only after a complete scan of every note: one unreadable folder or note means
//!    nothing is removed this time.
//! 3. When a budget is set and the suite is over it, disposable cache goes first.
//! 4. Still over, originals set aside by a shrink that are older than the cold window go, and
//!    their notes are marked `gone`. Past that the pass only reports, because what is left is
//!    the record itself and only the person may decide about that.
//!
//! The clock and the budget come in as values; this crate never looks at either.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::NaiveDate;
use hq_suite_md::frontmatter;
use hq_suite_md::meta::KEY_ORIGINAL_STATE;
use hq_suite_shrink::{Notes, set_original_state};
use hq_suite_store::cas::references_in;
use hq_suite_store::journal::Disposition;
use hq_suite_store::{Cas, CasRef, Job, Journal, Step, SuitePaths, trash};

/// A picture newer than this is left alone even if nothing links to it yet, because a shrink
/// stores pictures before it records its job.
pub const MEDIA_GRACE: Duration = Duration::from_secs(24 * 60 * 60);

/// Days an original set aside by a shrink is kept before the budget may reclaim it. The vault's
/// own trash keeps files this long.
pub const DEFAULT_COLD_WINDOW_DAYS: u32 = 30;

/// Folders no note scan looks in: media, the suite's own state and every other `_data`.
const SCAN_SKIP: &[&str] = &["_media", "_data"];

#[derive(Debug, Clone)]
pub struct Settings {
    /// What the suite may hold (record pictures, cache and set-aside originals), if limited.
    pub budget_bytes: Option<u64>,
    pub cold_window_days: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            budget_bytes: None,
            cold_window_days: DEFAULT_COLD_WINDOW_DAYS,
        }
    }
}

/// What the suite holds, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Pictures in the media store.
    pub media_bytes: u64,
    /// Disposable: rebuildable previews, thumbnails and statistics.
    pub cache_bytes: u64,
    /// Originals a shrink set aside and that are still there.
    pub cold_bytes: u64,
}

impl Report {
    pub fn total(&self) -> u64 {
        self.media_bytes + self.cache_bytes + self.cold_bytes
    }
}

/// What a pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub marked_gone: u32,
    pub media_removed: u32,
    pub media_bytes_freed: u64,
    pub cache_bytes_freed: u64,
    pub originals_expired: u32,
    pub original_bytes_freed: u64,
    /// The suite is still over its budget; only the person can free more.
    pub over_budget: bool,
    /// The pass skipped the steps that need a trustworthy journal or a complete scan.
    pub skipped: Vec<&'static str>,
}

#[derive(Debug, thiserror::Error)]
pub enum GovernError {
    #[error("{0}")]
    Notes(String),
    #[error("today is not a date: {0}")]
    BadDay(String),
}

impl From<hq_suite_shrink::ShrinkError> for GovernError {
    fn from(e: hq_suite_shrink::ShrinkError) -> Self {
        GovernError::Notes(e.to_string())
    }
}

fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn cold_target(job: &Job) -> Option<&str> {
    match job.disposition.as_ref()? {
        Disposition::Trash { target } => Some(target),
        // A cold sink (milestone M3) is the owner's own storage, not the vault's to clean.
        _ => None,
    }
}

fn is_set_aside(job: &Job) -> bool {
    matches!(job.step, Step::OriginalMoved | Step::Done)
}

fn file_len(paths: &SuitePaths, rel: &str) -> u64 {
    paths
        .resolve(rel)
        .ok()
        .and_then(|p| fs::metadata(p).ok())
        .map_or(0, |m| m.len())
}

/// What the suite holds right now.
pub fn report(paths: &SuitePaths) -> Report {
    let (jobs, _) = Journal::new(paths).list();
    let cold_bytes = jobs
        .iter()
        .filter(|j| is_set_aside(j))
        .filter_map(cold_target)
        .map(|t| file_len(paths, t))
        .sum();
    Report {
        media_bytes: Cas::new(paths.clone()).total_bytes(),
        cache_bytes: dir_bytes(&paths.cache_dir()),
        cold_bytes,
    }
}

fn mib(bytes: u64) -> String {
    let m = bytes as f64 / (1024.0 * 1024.0);
    if m >= 100.0 {
        format!("{m:.0} MB")
    } else {
        format!("{m:.1} MB")
    }
}

/// The report in the words shown to a person.
pub fn describe(r: &Report, budget: Option<u64>) -> String {
    let mut out = format!(
        "Your documents take {} of pictures, plus {} of saved-aside originals and {} of temporary files.",
        mib(r.media_bytes),
        mib(r.cold_bytes),
        mib(r.cache_bytes),
    );
    if let Some(b) = budget {
        if r.total() > b {
            out.push_str(&format!(
                " That is over the {} you set aside, so temporary files go first and then old originals.",
                mib(b)
            ));
        } else {
            out.push_str(&format!(" That is within the {} you set aside.", mib(b)));
        }
    }
    out
}

fn parse_day(day: &str) -> Result<NaiveDate, GovernError> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").map_err(|_| GovernError::BadDay(day.to_string()))
}

/// Every `.md` note under the vault (trashed ones included, since a restore brings them back),
/// with a flag that is false if anything could not be read.
fn scan_references(vault: &Path) -> (HashSet<CasRef>, bool) {
    let mut refs = HashSet::new();
    let mut complete = true;
    let mut stack = vec![vault.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            complete = false;
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else {
                complete = false;
                continue;
            };
            if ft.is_symlink() {
                // A link could lead to a note this scan cannot account for.
                complete = false;
            } else if ft.is_dir() {
                let top = dir == vault && SCAN_SKIP.contains(&name.as_str());
                if !top {
                    stack.push(path);
                }
            } else if ft.is_file() && name.ends_with(".md") {
                match fs::read_to_string(&path) {
                    Ok(t) => refs.extend(references_in(&t)),
                    Err(_) => complete = false,
                }
            }
        }
    }
    (refs, complete)
}

/// Marks the note of every job whose set-aside original is no longer there.
fn mark_gone(
    paths: &SuitePaths,
    notes: &dyn Notes,
    jobs: &[Job],
    out: &mut Outcome,
) -> Result<(), GovernError> {
    for job in jobs.iter().filter(|j| is_set_aside(j)) {
        let Some(target) = cold_target(job) else {
            continue;
        };
        if trash::exists(paths, target) || trash::exists(paths, &job.source) {
            continue;
        }
        let Some(text) = notes.read(&job.note)? else {
            continue;
        };
        let already = frontmatter::get(
            hq_suite_md::Document::parse(&text).frontmatter(),
            KEY_ORIGINAL_STATE,
        )
        .is_some_and(|v| v == "gone");
        if !already {
            set_original_state(notes, &job.note, "gone", 0)?;
            out.marked_gone += 1;
        }
    }
    Ok(())
}

fn collect_media(paths: &SuitePaths, now: SystemTime, out: &mut Outcome) -> Option<()> {
    let (mut refs, complete) = scan_references(paths.vault());
    if !complete {
        return None;
    }
    // Listed after the scan, so a job made while it ran is not missed.
    let (jobs, damaged) = Journal::new(paths).list();
    if !damaged.is_empty() {
        return None;
    }
    for job in jobs.iter().filter(|j| !j.step.is_terminal()) {
        refs.extend(job.media.iter().map(|m| m.cas.clone()));
    }
    let cas = Cas::new(paths.clone());
    for r in cas.unreferenced(&refs) {
        let Ok(path) = paths.resolve(&r.rel()) else {
            continue;
        };
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let young = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_none_or(|age| age < MEDIA_GRACE);
        if young {
            continue;
        }
        if cas.remove(&r).is_ok() {
            out.media_removed += 1;
            out.media_bytes_freed += meta.len();
        }
    }
    Some(())
}

fn clear_cache(paths: &SuitePaths, out: &mut Outcome) {
    let dir = paths.cache_dir();
    out.cache_bytes_freed = dir_bytes(&dir);
    let _ = fs::remove_dir_all(&dir);
}

/// The day an original was set aside, from the `_trash/<day>/...` path it went to.
fn set_aside_day(target: &str) -> Option<NaiveDate> {
    let day = target.strip_prefix("_trash/")?.split('/').next()?;
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

/// Whether the record that stands in for an original is still whole: its note is there and links
/// every picture the job stored, and each picture is intact. Only then may the original go.
fn record_is_whole(paths: &SuitePaths, notes: &dyn Notes, job: &Job) -> bool {
    let Ok(Some(text)) = notes.read(&job.note) else {
        return false;
    };
    let linked = references_in(&text);
    let cas = Cas::new(paths.clone());
    job.media
        .iter()
        .all(|m| linked.contains(&m.cas) && cas.verify(&m.cas).is_ok())
}

fn expire_originals(
    paths: &SuitePaths,
    notes: &dyn Notes,
    jobs: &[Job],
    today: NaiveDate,
    settings: &Settings,
    budget: u64,
    out: &mut Outcome,
) -> Result<(), GovernError> {
    let mut aged: Vec<(&Job, &str, NaiveDate)> = jobs
        .iter()
        .filter(|j| is_set_aside(j))
        .filter_map(|j| {
            let target = cold_target(j)?;
            Some((j, target, set_aside_day(target)?))
        })
        .filter(|(_, _, day)| (today - *day).num_days() >= i64::from(settings.cold_window_days))
        .collect();
    aged.sort_by_key(|(_, _, day)| *day);
    for (job, target, _) in aged {
        if report(paths).total() <= budget {
            break;
        }
        let size = file_len(paths, target);
        if size == 0 || trash::exists(paths, &job.source) {
            continue;
        }
        if !record_is_whole(paths, notes, job) {
            if !out.skipped.contains(&"a record is not whole") {
                out.skipped.push("a record is not whole");
            }
            continue;
        }
        let Ok(path) = paths.resolve(target) else {
            continue;
        };
        if fs::remove_file(path).is_ok() {
            out.originals_expired += 1;
            out.original_bytes_freed += size;
            // The file is gone either way; a note that cannot be marked now is marked next pass.
            let _ = set_original_state(notes, &job.note, "gone", 0);
        }
    }
    Ok(())
}

/// One governing pass. `today` is `YYYY-MM-DD`; `now` is used only to leave very new pictures
/// alone.
pub fn govern(
    paths: &SuitePaths,
    notes: &dyn Notes,
    settings: &Settings,
    today: &str,
    now: SystemTime,
) -> Result<Outcome, GovernError> {
    let today = parse_day(today)?;
    let mut out = Outcome::default();
    let (jobs, damaged) = Journal::new(paths).list();
    if !damaged.is_empty() {
        // A damaged job file may be the only record of where an original went.
        out.skipped.push("a job file is damaged");
        return Ok(out);
    }
    mark_gone(paths, notes, &jobs, &mut out)?;
    if collect_media(paths, now, &mut out).is_none() {
        out.skipped.push("a note could not be read");
    }
    let Some(budget) = settings.budget_bytes else {
        return Ok(out);
    };
    if report(paths).total() > budget {
        clear_cache(paths, &mut out);
    }
    if report(paths).total() > budget {
        expire_originals(paths, notes, &jobs, today, settings, budget, &mut out)?;
    }
    out.over_budget = report(paths).total() > budget;
    Ok(out)
}

#[cfg(test)]
mod tests;
