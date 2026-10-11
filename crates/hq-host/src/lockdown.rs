//! The host's kill switch. While `lockdown.json` sits in the run directory the
//! host starts no agent: not a new one, not a resumed one, not one restored at
//! startup. The file is the only truth, so it holds across restarts, `hq host
//! lockdown` can set it while the host is wedged, and only someone at the
//! machine can lift it by deleting the file (`hq host unlock`): no socket method
//! clears it, so a remote caller that can lock the host cannot unlock it.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const LOCKDOWN_FILE: &str = "lockdown.json";
const FILE_MODE: u32 = 0o600;
/// Longest reason kept, so a caller cannot make the host store a large blob.
const MAX_REASON_CHARS: usize = 500;

/// Why and when the host was locked, and by what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockdown {
    pub reason: String,
    /// `operator` (the socket), `cli` (`hq host lockdown`) or `breaker` (too many starts).
    pub source: String,
    /// Seconds since the Unix epoch.
    pub at: u64,
}

impl Lockdown {
    pub fn new(reason: &str, source: &str) -> Self {
        let reason: String = reason.trim().chars().take(MAX_REASON_CHARS).collect();
        let reason = if reason.is_empty() {
            "no reason given".to_string()
        } else {
            reason
        };
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self {
            reason,
            source: source.to_string(),
            at,
        }
    }
}

pub fn lockdown_path(dir: &Path) -> PathBuf {
    dir.join(LOCKDOWN_FILE)
}

/// The lockdown in force, if any. A file that is there but cannot be read or
/// parsed still locks the host: failing open would let a damaged file lift it.
pub fn read(dir: &Path) -> Option<Lockdown> {
    match fs::read(lockdown_path(dir)) {
        Ok(bytes) => Some(serde_json::from_slice(&bytes).unwrap_or_else(|_| Lockdown {
            reason: "the lockdown file is there but unreadable".into(),
            source: "unknown".into(),
            at: 0,
        })),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => Some(Lockdown {
            reason: format!("the lockdown file cannot be read: {e}"),
            source: "unknown".into(),
            at: 0,
        }),
    }
}

/// Locks the host. A lockdown already in force is kept as it is, so the first
/// cause is the one on record: the file is published with a hard link, which
/// fails when one is already there, so two callers at once cannot overwrite each
/// other. Returns the lockdown that holds.
pub fn engage(dir: &Path, lockdown: &Lockdown) -> std::io::Result<Lockdown> {
    if let Some(existing) = read(dir) {
        return Ok(existing);
    }
    let tmp = unique_tmp(dir);
    let written =
        write_private(&tmp, lockdown).and_then(|()| fs::hard_link(&tmp, lockdown_path(dir)));
    let _ = fs::remove_file(&tmp);
    match written {
        Ok(()) => Ok(lockdown.clone()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            Ok(read(dir).unwrap_or_else(|| lockdown.clone()))
        }
        Err(e) => Err(e),
    }
}

fn unique_tmp(dir: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{LOCKDOWN_FILE}.{}.{n}", std::process::id()))
}

fn write_private(path: &Path, lockdown: &Lockdown) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)?;
    file.write_all(&serde_json::to_vec_pretty(lockdown).map_err(std::io::Error::other)?)?;
    file.sync_all()
}

/// Lifts the lockdown. Returns whether there was one.
pub fn clear(dir: &Path) -> std::io::Result<bool> {
    match fs::remove_file(lockdown_path(dir)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engage_keeps_the_first_cause_and_clear_lifts_it() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()), None);
        let first = engage(dir.path(), &Lockdown::new("burst", "breaker")).unwrap();
        let again = engage(dir.path(), &Lockdown::new("later", "operator")).unwrap();
        assert_eq!(first, again);
        assert_eq!(read(dir.path()).unwrap().reason, "burst");
        assert!(clear(dir.path()).unwrap());
        assert!(!clear(dir.path()).unwrap());
        assert_eq!(read(dir.path()), None);
    }

    #[test]
    fn concurrent_engages_agree_on_one_cause() {
        let dir = tempfile::tempdir().unwrap();
        let held: Vec<Lockdown> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|n| {
                    let dir = dir.path();
                    scope.spawn(move || {
                        engage(dir, &Lockdown::new(&format!("cause {n}"), "test")).unwrap()
                    })
                })
                .collect();
            workers.into_iter().map(|w| w.join().unwrap()).collect()
        });
        let on_disk = read(dir.path()).unwrap();
        assert!(held.iter().all(|h| h.reason == on_disk.reason), "{held:?}");
        let leftovers = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1, "only lockdown.json remains");
    }

    #[test]
    fn a_damaged_file_still_locks() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(lockdown_path(dir.path()), b"not json").unwrap();
        assert!(read(dir.path()).is_some());
    }

    #[test]
    fn a_reason_is_trimmed_bounded_and_never_empty() {
        assert_eq!(Lockdown::new("  ", "cli").reason, "no reason given");
        let long = "x".repeat(MAX_REASON_CHARS * 2);
        assert_eq!(
            Lockdown::new(&long, "cli").reason.chars().count(),
            MAX_REASON_CHARS
        );
    }
}
