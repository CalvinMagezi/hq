//! What the last scheduled backup did, kept beside the backups so `hq backup status` can say. The
//! daemon runs a backup in the background, so this file is the only place a failure shows up
//! besides the log.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

const STATUS_FILE: &str = "backup-status.json";
const STATUS_TMP: &str = ".backup-status.tmp";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// When the run ended, RFC 3339.
    pub finished: String,
    pub ok: bool,
    /// Why it failed, or a one-line summary of what it wrote.
    pub message: String,
    pub path: Option<String>,
    pub files: usize,
    pub left_out: usize,
}

/// Records `status` in `dir`. The file is replaced whole, never half written.
pub fn write(dir: &Path, status: &Status) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(status).map_err(io::Error::other)?;
    let tmp = dir.join(STATUS_TMP);
    // A name left by a dead run, or a link planted there, is removed (the link itself, not what it
    // points at) and the file made new and exclusively, so nothing else is ever written through it.
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut file = crate::fsutil::create_private_file(&tmp)?;
    file.write_all(&json)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, dir.join(STATUS_FILE))
}

/// The last recorded run in `dir`, if any. A file that cannot be read is treated as none.
pub fn read(dir: &Path) -> Option<Status> {
    let bytes = fs::read(dir.join(STATUS_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_round_trips_and_a_missing_or_broken_one_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path()).is_none());
        let status = Status {
            finished: "2026-10-20T10:00:00+00:00".into(),
            ok: false,
            message: "the backup key file cannot be used".into(),
            path: None,
            files: 0,
            left_out: 0,
        };
        write(dir.path(), &status).unwrap();
        assert_eq!(read(dir.path()), Some(status));
        fs::write(dir.path().join(STATUS_FILE), b"{ broken").unwrap();
        assert!(read(dir.path()).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_planted_where_the_status_is_written_is_replaced_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, "precious").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join(STATUS_TMP)).unwrap();
        let status = Status {
            finished: "now".into(),
            ok: true,
            message: String::new(),
            path: None,
            files: 0,
            left_out: 0,
        };
        write(dir.path(), &status).unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        assert_eq!(read(dir.path()), Some(status));
    }
}
