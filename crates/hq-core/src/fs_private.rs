//! File writes for anything that can hold a credential: owner-only on unix.

use std::io;
use std::path::Path;

#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;
#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

/// Like `create_dir_all`, but directories this call creates are mode 0700.
/// Existing directories keep their mode.
pub fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(PRIVATE_DIR_MODE)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Writes `contents` to `path` as mode 0600, tightening a file that already
/// existed with a looser mode. The file is created private, so there is no
/// window where it is world-readable.
pub fn write_private(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(PRIVATE_FILE_MODE);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
    }
    file.write_all(contents.as_ref())
}

/// Copies `from` to `to` as mode 0600 (a plain `fs::copy` keeps the source mode).
pub fn copy_private(from: &Path, to: &Path) -> io::Result<u64> {
    let data = std::fs::read(from)?;
    write_private(to, &data)?;
    Ok(data.len() as u64)
}

/// Whether `path` belongs to this process's effective user. Files another user creates next to
/// a database (SQLite's WAL and shared-memory files) can lock its owner out.
pub fn owned_by_current_user(path: &Path) -> io::Result<bool> {
    let meta = std::fs::metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions.
        Ok(meta.uid() == unsafe { libc::geteuid() })
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Ok(true)
    }
}

/// Longest socket path the OS takes: `sun_path` holds 104 bytes on macOS and the BSDs and 108 on
/// Linux and Android, one of them the terminating NUL.
#[cfg(unix)]
pub const MAX_SOCKET_PATH: usize = if cfg!(any(target_os = "linux", target_os = "android")) {
    107
} else {
    103
};

/// Binds a unix socket only this user can reach. Its directory must be one only this user can
/// change (created `0700` when missing and `create_dir` is set), a stale socket from a run that
/// did not clean up is replaced, a socket another process still answers on is `AddrInUse`, and
/// anything else at the path is refused, so nothing is removed
/// that is not a socket. The socket itself is `0600`.
#[cfg(unix)]
pub fn bind_private_socket(path: &Path, create_dir: bool) -> io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    const GROUP_OTHER_WRITE: u32 = 0o022;
    let refuse = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, why);
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is longer than the {MAX_SOCKET_PATH} bytes a socket path may have",
                path.display()
            ),
        ));
    }
    let dir = path
        .parent()
        .ok_or_else(|| refuse(format!("{} has no directory", path.display())))?;
    if create_dir && !dir.exists() {
        create_private_dir_all(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(refuse(format!("{} is not a directory", dir.display())));
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    if meta.uid() != unsafe { libc::geteuid() } {
        return Err(refuse(format!("{} belongs to another user", dir.display())));
    }
    if meta.permissions().mode() & GROUP_OTHER_WRITE != 0 {
        return Err(refuse(format!(
            "{} can be written by others; chmod 700 it",
            dir.display()
        )));
    }
    match std::fs::symlink_metadata(path) {
        // A socket someone still answers on belongs to a running process: leave it to that one.
        // ponytail: blocking connect; a live owner with a full backlog would stall this, use a
        // nonblocking connect with a timeout if that is ever seen.
        Ok(existing) if existing.file_type().is_socket() => {
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("{} is in use by another process", path.display()),
                ));
            }
            std::fs::remove_file(path)?
        }
        Ok(_) => {
            return Err(refuse(format!(
                "{} exists and is not a socket; not touching it",
                path.display()
            )));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
    Ok(listener)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn new_files_are_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.yaml");
        write_private(&f, "k: v").unwrap();
        assert_eq!(mode(&f), 0o600);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "k: v");
    }

    #[test]
    fn a_loose_existing_file_is_tightened_and_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("config.yaml");
        std::fs::write(&f, "a long previous value").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&f, "short").unwrap();
        assert_eq!(mode(&f), 0o600);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "short");
    }

    #[test]
    fn copies_do_not_inherit_a_loose_source_mode() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a");
        let dst = dir.path().join("a.bak");
        std::fs::write(&src, "secret").unwrap();
        std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o644)).unwrap();
        copy_private(&src, &dst).unwrap();
        assert_eq!(mode(&dst), 0o600);
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "secret");
    }

    #[test]
    fn a_file_this_process_made_is_its_own() {
        let dir = tempfile::tempdir().unwrap();
        assert!(owned_by_current_user(dir.path()).unwrap());
        assert!(owned_by_current_user(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn created_directories_are_0700_and_existing_ones_untouched() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let nested = dir.path().join("a/b");
        create_private_dir_all(&nested).unwrap();
        assert_eq!(mode(&nested), 0o700);
        assert_eq!(mode(&dir.path().join("a")), 0o700);
        assert_eq!(mode(dir.path()), 0o755);
        create_private_dir_all(&nested).unwrap();
    }
    #[tokio::test]
    async fn a_socket_and_the_directory_made_for_it_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let door = dir.path().join("door");
        let socket = door.join("door.sock");
        assert!(
            bind_private_socket(&socket, false).is_err(),
            "no directory made unasked"
        );
        let _listener = bind_private_socket(&socket, true).unwrap();
        assert_eq!(mode(&door), 0o700);
        assert_eq!(mode(&socket), 0o600);
    }

    #[tokio::test]
    async fn a_directory_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("door.sock");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let err = bind_private_socket(&socket, false).unwrap_err();
        assert!(
            err.to_string().contains("can be written by others"),
            "{err}"
        );
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(
            bind_private_socket(&socket, false).is_err(),
            "group write counts too"
        );
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(bind_private_socket(&socket, false).is_ok());
    }

    #[tokio::test]
    async fn a_stale_socket_is_replaced_and_anything_else_at_the_path_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = dir.path().join("door.sock");
        drop(bind_private_socket(&socket, false).unwrap());
        // The socket file is still there after the listener is gone: a crash leaves it so.
        assert!(socket.exists());
        let again = bind_private_socket(&socket, false).expect("a stale socket is replaced");
        let err = bind_private_socket(&socket, false).unwrap_err();
        assert_eq!(
            err.kind(),
            io::ErrorKind::AddrInUse,
            "a live one is not taken over"
        );
        drop(again);
        let file = dir.path().join("precious");
        std::fs::write(&file, b"keep me").unwrap();
        let err = bind_private_socket(&file, false).unwrap_err();
        assert!(err.to_string().contains("not a socket"), "{err}");
        assert_eq!(std::fs::read(&file).unwrap(), b"keep me");
    }

    #[tokio::test]
    async fn a_directory_that_is_a_link_is_refused() {
        let real = tempfile::tempdir().unwrap();
        let holder = tempfile::tempdir().unwrap();
        std::fs::set_permissions(holder.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = holder.path().join("door");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        let err = bind_private_socket(&link.join("door.sock"), false).unwrap_err();
        assert!(err.to_string().contains("not a directory"), "{err}");
    }

    #[tokio::test]
    async fn a_socket_path_too_long_for_the_os_is_refused_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("x".repeat(MAX_SOCKET_PATH));
        let err = bind_private_socket(&long, false).unwrap_err();
        assert!(err.to_string().contains("longer than"), "{err}");
    }
}
