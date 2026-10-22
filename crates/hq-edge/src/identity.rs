//! The edge's own iroh key. It is the only secret the edge holds: it never has a device's key,
//! and a device's TLS key never leaves the device.

use std::{
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

use anyhow::{Context, Result, anyhow, bail};
use iroh::{EndpointId, SecretKey};

const KEY_BYTES: usize = 32;
const PRIVATE_DIR_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const GROUP_OTHER_BITS: u32 = 0o077;
const GROUP_OTHER_WRITE_BITS: u32 = 0o022;

/// Whether `dir` can be trusted to hold a secret or a list: nobody else can write to it (they
/// could swap what is inside) and it belongs to `owner` or to root. A directory that does not
/// exist yet counts as trusted, because nothing is in it.
pub(crate) fn dir_is_trusted(dir: &Path, owner: u32) -> std::io::Result<bool> {
    let dir = Some(dir)
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let meta = match std::fs::metadata(dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    let owned = meta.uid() == owner || meta.uid() == 0;
    Ok(owned && meta.permissions().mode() & GROUP_OTHER_WRITE_BITS == 0)
}

/// Creates `dir` (and parents) readable by its owner only, if it is missing.
pub(crate) fn ensure_private_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))
        .with_context(|| format!("restricting {}", dir.display()))
}

/// Loads the key at `path`, or makes one. A key file that anyone else can read, or that another
/// user owns, is refused: the edge would be signing as an identity others could copy.
pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    if let Some(dir) = path.parent() {
        ensure_private_dir(dir)?;
    }
    if !path.exists() {
        let key = SecretKey::generate();
        // Written beside the key and renamed into place, so a crash leaves no half-written key
        // that the next start would refuse.
        let tmp = path.with_extension("key.tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(&key.to_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path).with_context(|| format!("creating {}", path.display()))?;
        return Ok(key);
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    let owner = unsafe { libc::geteuid() };
    if let Some(dir) = path.parent()
        && !dir_is_trusted(dir, owner)?
    {
        bail!(
            "{} can be written by others or belongs to another user",
            dir.display()
        );
    }
    // Opened without following a symlink and checked on the open file, so nothing can be swapped
    // in between the check and the read.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        bail!("{} is not a plain file", path.display());
    }
    if meta.uid() != owner {
        bail!(
            "{} is not owned by the user running the edge",
            path.display()
        );
    }
    if meta.permissions().mode() & GROUP_OTHER_BITS != 0 {
        bail!(
            "{} is readable by others (mode {:o}); chmod 600 it",
            path.display(),
            meta.permissions().mode() & 0o777
        );
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let bytes: [u8; KEY_BYTES] = bytes
        .try_into()
        .map_err(|_| anyhow!("{} is not a {KEY_BYTES}-byte key", path.display()))?;
    Ok(SecretKey::from_bytes(&bytes))
}

/// The edge's public endpoint id, which a device pins so only the edge may open edge streams.
pub fn edge_id(key_path: &Path) -> Result<EndpointId> {
    Ok(load_or_create_key(key_path)?.public())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_made_once_and_loaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("edge.key");
        let first = load_or_create_key(&path).unwrap();
        let second = load_or_create_key(&path).unwrap();
        assert_eq!(first.public(), second.public());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            PRIVATE_FILE_MODE
        );
    }

    #[test]
    fn a_key_in_a_directory_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.key");
        load_or_create_key(&path).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(load_or_create_key(&path).is_err());
    }

    #[test]
    fn a_symlinked_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.key");
        load_or_create_key(&real).unwrap();
        let link = dir.path().join("edge.key");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(load_or_create_key(&link).is_err());
    }

    #[test]
    fn a_key_others_can_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.key");
        load_or_create_key(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create_key(&path).is_err());
    }
}
