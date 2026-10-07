//! What the host remembers across a restart: the agents that were running and
//! how to start each one again. The file holds commands the host will run, so
//! it is trusted only when it is a regular file owned by this user that no one
//! else can read or write, in a directory that passes the same check.

use crate::token;
use nix::unistd::geteuid;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Error, ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const STATE_FILE: &str = "session.json";
const FORMAT_VERSION: u32 = 1;
const FILE_MODE: u32 = 0o600;
const GROUP_OTHER_BITS: u32 = 0o077;

/// One agent to bring back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneRecord {
    pub name: String,
    /// Replaces the original command on restore, so a fresh process continues
    /// the old session (for example `claude --continue`).
    pub resume_argv: Vec<String>,
    pub agent: Option<String>,
    pub cwd: PathBuf,
    /// Names of the extra variables the agent was started with. Values are
    /// never written: whoever started the agent supplies them again on resume.
    #[serde(default)]
    pub env_keys: Vec<String>,
    pub rows: u16,
    pub cols: u16,
    pub scrollback_rows: usize,
    /// The sandbox the agent ran under, applied again on restore. Absent in
    /// files written before sandboxing, which read as unsandboxed.
    #[serde(default)]
    pub sandbox: Option<crate::sandbox::SandboxSpec>,
}

#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    panes: Vec<PaneRecord>,
}

pub(crate) struct StateFile {
    dir: PathBuf,
    path: PathBuf,
    write_lock: Mutex<()>,
    /// Once set, nothing is written: the host is shutting down and the file
    /// must keep describing what was running before the processes are stopped.
    frozen: AtomicBool,
}

impl StateFile {
    pub(crate) fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            path: dir.join(STATE_FILE),
            write_lock: Mutex::new(()),
            frozen: AtomicBool::new(false),
        }
    }

    pub(crate) fn freeze(&self) {
        self.frozen.store(true, Ordering::SeqCst);
    }

    /// Replaces the file with `records`, atomically.
    pub(crate) fn write(&self, records: &[PaneRecord]) -> std::io::Result<()> {
        let _guard = self.write_lock.lock().unwrap_or_else(|p| p.into_inner());
        if self.frozen.load(Ordering::SeqCst) {
            return Ok(());
        }
        token::ensure_dir(&self.dir)?;
        let doc = Document {
            version: FORMAT_VERSION,
            panes: records.to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&doc).map_err(Error::other)?;
        let tmp = self.dir.join(format!("{STATE_FILE}.tmp"));
        let _ = fs::remove_file(&tmp);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, &self.path)
    }

    /// The saved agents. A missing file is an empty list. A file that is not
    /// safe to trust is an error, and one that does not parse is moved aside
    /// (so it is not lost) and read as empty.
    pub(crate) fn read(&self) -> std::io::Result<Vec<PaneRecord>> {
        token::ensure_dir(&self.dir)?;
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        if !meta.is_file()
            || meta.uid() != geteuid().as_raw()
            || meta.mode() & GROUP_OTHER_BITS != 0
        {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                format!(
                    "{} must be a regular file owned by you with mode 0600; it lists commands the host would run",
                    self.path.display()
                ),
            ));
        }
        let text = fs::read_to_string(&self.path)?;
        match serde_json::from_str::<Document>(&text) {
            Ok(doc) if doc.version == FORMAT_VERSION => Ok(doc.panes),
            _ => {
                let aside = self.dir.join(format!("{STATE_FILE}.unreadable"));
                fs::rename(&self.path, &aside)?;
                Ok(Vec::new())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn record(name: &str) -> PaneRecord {
        PaneRecord {
            name: name.into(),
            resume_argv: vec!["claude".into(), "--continue".into()],
            agent: Some("claude".into()),
            cwd: "/tmp".into(),
            env_keys: vec!["HQ_SESSION_ID".into()],
            rows: 40,
            cols: 120,
            scrollback_rows: 1000,
            sandbox: None,
        }
    }

    #[test]
    fn records_round_trip_through_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateFile::new(&dir.path().join("run"));
        assert!(state.read().unwrap().is_empty());
        state.write(&[record("a"), record("b")]).unwrap();
        assert_eq!(state.read().unwrap(), vec![record("a"), record("b")]);
        let mode = fs::metadata(&state.path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, FILE_MODE);
    }

    #[test]
    fn a_file_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateFile::new(&dir.path().join("run"));
        state.write(&[record("a")]).unwrap();
        fs::set_permissions(&state.path, fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(
            state.read().unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn a_file_that_does_not_parse_is_moved_aside_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateFile::new(&dir.path().join("run"));
        state.write(&[]).unwrap();
        fs::write(&state.path, "{ not json").unwrap();
        fs::set_permissions(&state.path, fs::Permissions::from_mode(FILE_MODE)).unwrap();
        assert!(state.read().unwrap().is_empty());
        assert!(!state.path.exists());
        assert_eq!(
            fs::read_to_string(state.dir.join("session.json.unreadable")).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn a_frozen_file_stops_changing() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateFile::new(&dir.path().join("run"));
        state.write(&[record("a")]).unwrap();
        state.freeze();
        state.write(&[]).unwrap();
        assert_eq!(state.read().unwrap().len(), 1);
    }
}
