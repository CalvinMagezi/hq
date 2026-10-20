//! Names for scheduled backups and keeping only the newest few.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, NaiveDateTime, Utc};

use crate::error::{Result, refused};

const PREFIX: &str = "hq-backup-";
pub const EXTENSION: &str = "hqbak";
const PARTIAL_SUFFIX: &str = ".partial";
const STAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";
/// A name dated further ahead than this is a clock that was wrong when it was written, or a file
/// someone planted. It is never listed, so it can neither pass for the newest backup nor push real
/// ones out of the kept few. Names step a couple of minutes ahead when two backups share a second.
const FUTURE_SLACK: chrono::Duration = chrono::Duration::days(1);
/// An unfinished backup older than this was left by a run that died.
const STALE_PARTIAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The file name a backup made at `when` gets. Names sort by time.
pub fn file_name(when: DateTime<Utc>) -> String {
    format!("{PREFIX}{}.{EXTENSION}", when.format(STAMP_FORMAT))
}

fn stamp_of(name: &str) -> Option<DateTime<Utc>> {
    let stamp = name
        .strip_prefix(PREFIX)?
        .strip_suffix(&format!(".{EXTENSION}"))?;
    let digits_only = stamp.bytes().filter(|b| *b != b'T' && *b != b'Z');
    if stamp.len() != 16 || !digits_only.clone().all(|b| b.is_ascii_digit()) {
        return None;
    }
    NaiveDateTime::parse_from_str(stamp, STAMP_FORMAT)
        .ok()
        .map(|t| t.and_utc())
}

/// Whether `name` is exactly what [`file_name`] produces. Nothing else in a backup folder is
/// ever listed or deleted.
pub fn is_backup_name(name: &str) -> bool {
    stamp_of(name).is_some()
}

/// The backups in `dir`, oldest first, plain files only, leaving out names dated in the future.
pub fn list(dir: &Path) -> Result<Vec<PathBuf>> {
    let latest = Utc::now() + FUTURE_SLACK;
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(stamp) = name.to_str().and_then(stamp_of) else {
            continue;
        };
        if stamp <= latest && entry.file_type()?.is_file() {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

/// The newest backup in `dir` and when it was written, if there is one.
pub fn newest(dir: &Path) -> Result<Option<(PathBuf, SystemTime)>> {
    let Some(path) = list(dir)?.pop() else {
        return Ok(None);
    };
    let modified = fs::metadata(&path)?.modified()?;
    Ok(Some((path, modified)))
}

/// Deletes all but the newest `keep` backups in `dir` and returns what it deleted. `protect`, the
/// backup that was just written, is never deleted whatever its name sorts as. Keeping none is
/// refused: a retention setting must never be able to erase the only copy. Unfinished backups
/// that a dead run left behind are swept too.
pub fn prune(dir: &Path, keep: usize, protect: Option<&Path>) -> Result<Vec<PathBuf>> {
    if keep == 0 {
        return refused("keep at least one backup");
    }
    sweep_partials(dir);
    let all = list(dir)?;
    let excess = all.len().saturating_sub(keep);
    let mut removed = Vec::new();
    for path in all.into_iter().take(excess) {
        if Some(path.as_path()) == protect {
            continue;
        }
        fs::remove_file(&path)?;
        removed.push(path);
    }
    Ok(removed)
}

/// Removes `*.hqbak.partial` files, only those named exactly like an unfinished backup, that have
/// not been touched for a day.
pub fn sweep_partials(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let finished = name
            .to_str()
            .and_then(|n| n.strip_suffix(PARTIAL_SUFFIX))
            .is_some_and(is_backup_name);
        let stale = entry.metadata().ok().is_some_and(|m| {
            m.is_file()
                && m.modified()
                    .ok()
                    .and_then(|at| at.elapsed().ok())
                    .is_some_and(|age| age >= STALE_PARTIAL)
        });
        if finished && stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn names_round_trip_and_sort_by_time() {
        let early = file_name(Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap());
        let late = file_name(Utc.with_ymd_and_hms(2026, 11, 30, 23, 59, 59).unwrap());
        assert_eq!(early, "hq-backup-20260102T030405Z.hqbak");
        assert!(is_backup_name(&early) && is_backup_name(&late) && early < late);
    }

    #[test]
    fn only_exact_backup_names_count() {
        for bad in [
            "hq-backup-20260102T030405Z.hqbak.partial",
            "hq-backup-20260102T030405Z.hqbak.sig",
            "hq-backup-2026010T030405Z.hqbak",
            "hq-backup-20261301T030405Z.hqbak",
            "hq-backup-20260102T0304056.hqbak",
            "hq-backup-abcdefghTijklmnZ.hqbak",
            "backup-20260102T030405Z.hqbak",
            "notes.md",
            "",
        ] {
            assert!(!is_backup_name(bad), "{bad}");
        }
    }

    #[test]
    fn prune_keeps_the_newest_and_touches_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let mut names = Vec::new();
        for day in 1..=5 {
            let name = file_name(Utc.with_ymd_and_hms(2026, 3, day, 1, 0, 0).unwrap());
            fs::write(dir.path().join(&name), b"x").unwrap();
            names.push(name);
        }
        for other in [
            "notes.md",
            "hq-backup-latest.hqbak",
            "hq-backup-20260301T010000Z.hqbak.partial",
        ] {
            fs::write(dir.path().join(other), b"keep me").unwrap();
        }
        fs::create_dir(dir.path().join(file_name(
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        )))
        .unwrap();

        let removed = prune(dir.path(), 2, None).unwrap();
        assert_eq!(removed.len(), 3);
        let left = list(dir.path()).unwrap();
        assert_eq!(
            left,
            vec![dir.path().join(&names[3]), dir.path().join(&names[4])]
        );
        for other in [
            "notes.md",
            "hq-backup-latest.hqbak",
            "hq-backup-20260301T010000Z.hqbak.partial",
        ] {
            assert!(dir.path().join(other).exists(), "{other}");
        }
        assert!(prune(dir.path(), 0, None).is_err());
        assert_eq!(list(dir.path()).unwrap().len(), 2);
    }

    #[test]
    fn newest_is_the_latest_name() {
        let dir = tempfile::tempdir().unwrap();
        assert!(newest(dir.path()).unwrap().is_none());
        for day in [3, 9, 5] {
            let name = file_name(Utc.with_ymd_and_hms(2026, 3, day, 1, 0, 0).unwrap());
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        let (path, _) = newest(dir.path()).unwrap().unwrap();
        assert!(path.to_str().unwrap().contains("20260309"));
    }

    #[test]
    fn the_backup_just_written_is_never_pruned_and_future_names_are_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let day = |d| Utc.with_ymd_and_hms(2026, 3, d, 1, 0, 0).unwrap();
        for d in 1..=4 {
            fs::write(dir.path().join(file_name(day(d))), b"x").unwrap();
        }
        // A clock that was set back: the backup just written sorts as the oldest.
        let fresh = dir.path().join(file_name(day(1)));
        let removed = prune(dir.path(), 2, Some(&fresh)).unwrap();
        assert!(fresh.exists() && !removed.contains(&fresh), "{removed:?}");

        let planted = dir
            .path()
            .join(file_name(Utc::now() + chrono::Duration::days(400)));
        fs::write(&planted, b"x").unwrap();
        let listed = list(dir.path()).unwrap();
        assert!(!listed.contains(&planted), "{listed:?}");
        let (newest_path, _) = newest(dir.path()).unwrap().unwrap();
        assert_ne!(newest_path, planted);
        prune(dir.path(), 1, None).unwrap();
        assert!(planted.exists(), "a future-dated name is left alone");
    }

    #[test]
    fn a_partial_left_by_a_dead_run_is_swept_but_a_fresh_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let name = file_name(Utc.with_ymd_and_hms(2026, 3, 1, 1, 0, 0).unwrap());
        let old = dir.path().join(format!("{name}{PARTIAL_SUFFIX}"));
        let fresh_name = file_name(Utc.with_ymd_and_hms(2026, 3, 2, 1, 0, 0).unwrap());
        let fresh = dir.path().join(format!("{fresh_name}{PARTIAL_SUFFIX}"));
        let other = dir.path().join("notes.hqbak.partial");
        for p in [&old, &fresh, &other] {
            fs::write(p, b"x").unwrap();
        }
        let aged = SystemTime::now() - 3 * STALE_PARTIAL;
        for p in [&old, &other] {
            fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(aged)
                .unwrap();
        }
        prune(dir.path(), 1, None).unwrap();
        assert!(!old.exists() && fresh.exists() && other.exists());
    }
}
