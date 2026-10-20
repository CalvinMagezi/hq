//! File helpers that keep what a backup touches private to the user running it.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

use crate::error::{Result, refused};

/// Creates a new file only the current user can read, failing if it exists. Never follows a link.
pub(crate) fn create_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

/// Creates `dir` and any missing parents, readable only by the current user.
pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Creates one new folder, readable only by the current user, and fails if the name is taken, so a
/// name someone else planted is never reused. The parent must exist.
pub(crate) fn create_private_dir_new(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Whether the current user owns what `meta` describes. Always true where owners are not tracked.
pub(crate) fn owned_by_me(meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid takes no arguments and cannot fail.
        meta.uid() == unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        true
    }
}

/// A secret file must be a plain file, not a link, that only its owner can read.
fn check_private_meta(meta: &fs::Metadata, path: &Path) -> Result<()> {
    if !meta.file_type().is_file() {
        return refused(format!(
            "{} must be a plain file, not a link or a folder",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return refused(format!(
                "{} is readable by other users; run `chmod 600 {}` first",
                path.display(),
                path.display()
            ));
        }
    }
    if !owned_by_me(meta) {
        return refused(format!("{} is not owned by you", path.display()));
    }
    Ok(())
}

/// Opens a secret file after checking, on the open file itself, that it is a plain file only its
/// owner can read. The open refuses a link, so there is no gap between the look and the read in
/// which the name could be pointed somewhere else.
pub(crate) fn open_private(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Non-blocking so that a FIFO planted at the path is refused by the type check below
        // instead of hanging the open.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| {
        #[cfg(unix)]
        if e.raw_os_error() == Some(libc::ELOOP) {
            return crate::BackupError::Refused(format!(
                "{} must be a plain file, not a link or a folder",
                path.display()
            ));
        }
        crate::BackupError::Refused(format!("{}: {e}", path.display()))
    })?;
    check_private_meta(&file.metadata()?, path)?;
    Ok(file)
}
