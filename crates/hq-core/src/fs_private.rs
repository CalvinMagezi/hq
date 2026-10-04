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
}
