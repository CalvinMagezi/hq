//! Files that can hold a key or a join text: owner-only, created fresh, never reached through a
//! link. The directory they live in has to be private to the user, because a rename into a shared
//! one could be raced.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub(crate) const PRIVATE_FILE_MODE: u32 = 0o600;
const PRIVATE_DIR_MODE: u32 = 0o700;
/// Any permission bit for the group or others.
const OTHERS_MASK: u32 = 0o077;

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// A fresh private file: any file already at the path is replaced, a link is never followed, and
/// the mode is set on the open file rather than left to the umask.
pub(crate) fn private_file(path: &Path) -> io::Result<File> {
    let _ = fs::remove_file(path);
    create_new_private(path)
}

/// A file that must not exist yet, so an existing one is never overwritten.
pub(crate) fn create_new_private(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
    Ok(file)
}

/// `status.json` writes through `status.json.tmp`, so two files in one directory never share one.
pub(crate) fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}

/// Replaces `path` with `bytes` through a temporary file and a rename, so a reader never sees half.
pub(crate) fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = temp_path(path);
    let mut file = private_file(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)
}

/// Who may have made a file that is about to be trusted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// Any regular file.
    Anyone,
    /// A file this user owns, whatever its mode: one the user wrote by hand.
    Owner,
    /// A file this user owns and nobody else can open: a key, a join text.
    OwnerOnly,
}

/// Reads a regular file of at most `max` bytes, without following a link, after the ownership
/// check `access` asks for.
pub(crate) fn read_private(path: &Path, max: u64, access: Access) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    if access != Access::Anyone && meta.uid() != effective_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the file belongs to another user",
        ));
    }
    if access == Access::OwnerOnly && meta.mode() & OTHERS_MASK != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the file must be closed to everyone else (chmod 600)",
        ));
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        bytes.fill(0);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the file is longer than it can be",
        ));
    }
    Ok(bytes)
}

/// Fails unless `path` is a real directory (not a link) that belongs to this user and that nobody
/// else can enter. A directory that was open to others may hold files they put there, so it is
/// refused, not quietly tightened.
pub(crate) fn check_private_dir(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let refuse = |kind, what: &str| Err(io::Error::new(kind, what.to_string()));
    if meta.file_type().is_symlink() {
        return refuse(
            io::ErrorKind::InvalidInput,
            "the tunnel directory is a link",
        );
    }
    if !meta.is_dir() {
        return refuse(
            io::ErrorKind::InvalidInput,
            "the tunnel directory is not a directory",
        );
    }
    if meta.uid() != effective_uid() {
        return refuse(
            io::ErrorKind::PermissionDenied,
            "the tunnel directory belongs to another user",
        );
    }
    if meta.mode() & OTHERS_MASK != 0 {
        return refuse(
            io::ErrorKind::PermissionDenied,
            "the tunnel directory is open to others (chmod 700)",
        );
    }
    Ok(())
}

/// A directory only this user can enter, made if it is missing.
pub(crate) fn ensure_private_dir(path: &Path) -> io::Result<()> {
    match check_private_dir(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(PRIVATE_DIR_MODE)
            .create(path),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_replaced_whole_private_and_never_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("precious");
        fs::write(&target, "keep me").unwrap();
        let path = dir.path().join("join.conf");
        std::os::unix::fs::symlink(&target, temp_path(&path)).unwrap();
        write_private_atomic(&path, b"fresh").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep me");
        assert_eq!(fs::read_to_string(&path).unwrap(), "fresh");
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, PRIVATE_FILE_MODE);
    }

    #[test]
    fn a_new_file_never_overwrites_one_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        fs::write(&path, "old").unwrap();
        let err = create_new_private(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
    }

    #[test]
    fn reading_refuses_links_other_modes_and_long_files() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::write(&real, "abc").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_private(&real, 16, Access::OwnerOnly).unwrap(), b"abc");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(
            read_private(&link, 16, Access::OwnerOnly).is_err(),
            "a link is refused"
        );
        assert!(
            read_private(&real, 2, Access::OwnerOnly).is_err(),
            "too long"
        );
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(
            read_private(&real, 16, Access::OwnerOnly).is_err(),
            "group readable"
        );
        assert!(
            read_private(&real, 16, Access::Owner).is_ok(),
            "mode is not asked for"
        );
        assert!(
            read_private(&real, 16, Access::Anyone).is_ok(),
            "no check asked for"
        );
        assert!(
            read_private(dir.path(), 16, Access::Anyone).is_err(),
            "a directory"
        );
    }

    #[test]
    fn the_directory_is_made_private_and_a_link_to_one_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("a").join("tunnel");
        ensure_private_dir(&dir).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), PRIVATE_DIR_MODE);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let open = ensure_private_dir(&dir).unwrap_err().to_string();
        assert!(open.contains("chmod 700"), "{open}");
        assert!(check_private_dir(&dir).is_err());
        assert_eq!(
            mode(&dir),
            0o755,
            "a directory that was open is not quietly trusted"
        );
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        check_private_dir(&dir).unwrap();
        let link = root.path().join("elsewhere");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(ensure_private_dir(&link).is_err());
        let file = root.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(ensure_private_dir(&file).is_err());
    }
}
