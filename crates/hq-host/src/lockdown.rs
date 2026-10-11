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
/// cause is the one on record. Returns the lockdown that holds.
pub fn engage(dir: &Path, lockdown: &Lockdown) -> std::io::Result<Lockdown> {
    if let Some(existing) = read(dir) {
        return Ok(existing);
    }
    let path = lockdown_path(dir);
    let tmp = dir.join(format!(".{LOCKDOWN_FILE}.{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(FILE_MODE)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(lockdown).map_err(std::io::Error::other)?)?;
    file.sync_all()?;
    fs::rename(&tmp, &path)?;
    Ok(lockdown.clone())
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
