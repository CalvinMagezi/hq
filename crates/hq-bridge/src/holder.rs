//! The record `hq bridge serve` keeps in HQ's private run folder, naming the forwards it listens
//! on, so HQ Hub can tell HQ's own bridge from another program that took the same port. Agents
//! HQ launches cannot write under `~/.hq`, so the record is HQ's word; the hub still asks the OS
//! whether that process really holds the port (`hq_desktop::bridge_holder`).

use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const RUN_DIR: &str = "run";
const RECORD_FILE: &str = "bridge.json";
const RECORD_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderRecord {
    pub pid: u32,
    /// Loopback addresses this process listens on for its forwards.
    pub listens: Vec<SocketAddr>,
}

pub fn record_path(hq_dir: &Path) -> PathBuf {
    hq_dir.join(RUN_DIR).join(RECORD_FILE)
}

/// Writes the record for this process: private folder, private file, replaced in one rename.
pub fn write(hq_dir: &Path, listens: Vec<SocketAddr>) -> Result<()> {
    let path = record_path(hq_dir);
    let dir = path.parent().expect("the record has a folder");
    hq_core::fs_private::create_private_dir_all(dir)?;
    let record = HolderRecord {
        pid: std::process::id(),
        listens,
    };
    let tmp = dir.join(format!(".{RECORD_FILE}.{}", std::process::id()));
    hq_core::fs_private::write_private(&tmp, serde_json::to_vec(&record)?)?;
    std::fs::rename(&tmp, &path).with_context(|| format!("could not write {}", path.display()))
}

/// Removes the record when it is this process's own.
pub fn remove_own(hq_dir: &Path) {
    let path = record_path(hq_dir);
    if read(&path).is_ok_and(|r| r.pid == std::process::id()) {
        let _ = std::fs::remove_file(path);
    }
}

/// Reads a record only when nobody but this user could have written it: a regular file (never a
/// symlink) owned by this user that neither group nor others can change, in a folder the same
/// holds for.
pub fn read(path: &Path) -> Result<HolderRecord> {
    let dir = path.parent().context("the record has no folder")?;
    only_this_user_writes(&std::fs::symlink_metadata(dir)?, true)?;
    let mut file = open_no_follow(path)?;
    let meta = file.metadata()?;
    only_this_user_writes(&meta, false)?;
    if meta.len() > RECORD_MAX_BYTES {
        bail!("the bridge record is too large");
    }
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(serde_json::from_str(&text)?)
}

/// When the record was last written, which a process named in it must have started before.
pub fn written_at(path: &Path) -> Result<std::time::SystemTime> {
    Ok(std::fs::symlink_metadata(path)?.modified()?)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?)
}

#[cfg(not(unix))]
fn open_no_follow(_: &Path) -> Result<std::fs::File> {
    bail!("the bridge record is read only on unix")
}

#[cfg(unix)]
fn only_this_user_writes(meta: &std::fs::Metadata, dir: bool) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    const GROUP_OR_OTHER_WRITE: u32 = 0o022;
    let kind_ok = if dir { meta.is_dir() } else { meta.is_file() };
    // SAFETY: geteuid has no preconditions.
    let mine = meta.uid() == unsafe { libc::geteuid() };
    if !kind_ok || !mine || meta.permissions().mode() & GROUP_OR_OTHER_WRITE != 0 {
        bail!("the bridge record or its folder is not this user's alone");
    }
    Ok(())
}

#[cfg(not(unix))]
fn only_this_user_writes(_: &std::fs::Metadata, _: bool) -> Result<()> {
    bail!("the bridge record is read only on unix")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_record_is_read_back_only_while_this_user_alone_can_write_it() {
        let home = tempfile::TempDir::new().unwrap();
        let addr: SocketAddr = "127.0.0.1:47480".parse().unwrap();
        write(home.path(), vec![addr]).unwrap();
        let path = record_path(home.path());
        let rec = read(&path).unwrap();
        assert_eq!((rec.pid, rec.listens), (std::process::id(), vec![addr]));

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o622)).unwrap();
        assert!(read(&path).is_err(), "others could have written it");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let dir = path.parent().unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(read(&path).is_err(), "others could have swapped it");
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();

        remove_own(home.path());
        assert!(!path.exists());
    }

    #[test]
    fn a_symlinked_record_is_refused() {
        let home = tempfile::TempDir::new().unwrap();
        write(home.path(), vec![]).unwrap();
        let path = record_path(home.path());
        let real = path.with_file_name("elsewhere.json");
        std::fs::rename(&path, &real).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();
        assert!(read(&path).is_err());
    }
}
