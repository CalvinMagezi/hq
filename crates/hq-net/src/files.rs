//! Files `hq net` keeps for itself: owned by the user, private, checked on the open handle, and
//! replaced whole so a reader (the applier's path unit) never sees half a file.

use rand::RngCore;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const PRIVATE_DIR_MODE: u32 = 0o700;
pub const PRIVATE_FILE_MODE: u32 = 0o600;
const GROUP_OTHER_ANY: u32 = 0o077;
const GROUP_OTHER_WRITE: u32 = 0o022;
const MAX_FILE_BYTES: u64 = 1_000_000;
const TEMP_SUFFIX_BYTES: usize = 6;

pub type IoResult<T> = std::io::Result<T>;

fn refuse<T>(path: &Path, why: &str) -> IoResult<T> {
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("{} {why}", path.display()),
    ))
}

pub fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

pub fn effective_gid() -> u32 {
    // SAFETY: getegid has no preconditions and cannot fail.
    unsafe { libc::getegid() }
}

/// Makes the directory (and parents) if needed. The directory itself must end up private and
/// owned by the current user; a directory someone else can enter is refused.
pub fn ensure_private_dir(dir: &Path) -> IoResult<()> {
    std::fs::create_dir_all(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.file_type().is_dir() {
        return refuse(dir, "is not a directory");
    }
    if meta.uid() != effective_uid() {
        return refuse(dir, "is not owned by the user running hq");
    }
    if meta.mode() & GROUP_OTHER_ANY != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))?;
    }
    Ok(())
}

/// The directory the applier's input goes in: it must already exist, be a real directory owned by
/// the current user, and be closed to writes by others, since a name swapped in there is what the
/// root applier would read.
pub fn check_spool_dir(dir: &Path) -> IoResult<()> {
    let meta = match std::fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return refuse(
                dir,
                "does not exist yet; step 4 of the `hq net init` output creates it",
            );
        }
        Err(e) => return Err(e),
    };
    if !meta.file_type().is_dir() {
        return refuse(dir, "is not a directory");
    }
    if meta.uid() != effective_uid() {
        return refuse(dir, "is not owned by the user running hq");
    }
    if meta.mode() & GROUP_OTHER_WRITE != 0 {
        return refuse(dir, "can be written by other users");
    }
    Ok(())
}

/// Reads a private file, or `None` if there is none. It must be a plain file (not a link),
/// owned by the current user, with no group or other access; the checks run on the open handle.
pub fn read_private(path: &Path) -> IoResult<Option<String>> {
    let before = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !before.file_type().is_file() {
        return refuse(path, "is not a plain file");
    }
    let file = File::open(path)?;
    let meta = file.metadata()?;
    if meta.dev() != before.dev() || meta.ino() != before.ino() {
        return refuse(path, "changed while it was being opened");
    }
    if meta.uid() != effective_uid() {
        return refuse(path, "is not owned by the user running hq");
    }
    if meta.mode() & GROUP_OTHER_ANY != 0 {
        return refuse(path, "can be read by other users; run chmod 600 on it");
    }
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return refuse(path, "is too large");
    }
    Ok(Some(text))
}

fn temp_sibling(path: &Path) -> PathBuf {
    let mut suffix = [0u8; TEMP_SUFFIX_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut suffix);
    let hex: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{hex}.tmp"))
}

/// Writes `text` to `path` by creating a private temporary file next to it and renaming it into
/// place. A reader sees the old file or the new one, never a partial one.
pub fn write_private_atomic(path: &Path, text: &str) -> IoResult<()> {
    let temp = temp_sibling(path);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// Creates a private file that must not exist yet.
pub fn create_private_new(path: &Path, text: &str) -> IoResult<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)?;
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_none_and_a_written_one_reads_back_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.toml");
        assert_eq!(read_private(&path).unwrap(), None);
        write_private_atomic(&path, "x = 1\n").unwrap();
        assert_eq!(read_private(&path).unwrap().as_deref(), Some("x = 1\n"));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_FILE_MODE);
        write_private_atomic(&path, "x = 2\n").unwrap();
        assert_eq!(read_private(&path).unwrap().as_deref(), Some("x = 2\n"));
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file may stay behind");
    }

    #[test]
    fn links_open_files_and_foreign_modes_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        write_private_atomic(&real, "x\n").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(read_private(&link).is_err());
        assert!(read_private(dir.path()).is_err());
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private(&real).is_err());
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(read_private(&real).is_err());
    }

    #[test]
    fn create_new_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub.key");
        create_private_new(&path, "first\n").unwrap();
        assert!(create_private_new(&path, "second\n").is_err());
        assert_eq!(read_private(&path).unwrap().as_deref(), Some("first\n"));
    }

    #[test]
    fn a_failed_atomic_write_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dir-in-the-way");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("child"), "x").unwrap();
        assert!(write_private_atomic(&target, "x").is_err());
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn a_state_directory_becomes_private() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("a/b/net");
        ensure_private_dir(&state).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&state).unwrap();
        let mode = std::fs::metadata(&state).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_DIR_MODE);
    }
}
