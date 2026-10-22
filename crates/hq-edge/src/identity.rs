//! The edge's own iroh key. It is the only secret the edge holds: it never has a device's key,
//! and a device's TLS key never leaves the device.

use std::{
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

use anyhow::{Context, Result, anyhow, bail};
use iroh::{EndpointId, SecretKey};

const KEY_BYTES: usize = 32;
const PRIVATE_DIR_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const GROUP_OTHER_BITS: u32 = 0o077;

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
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        file.write_all(&key.to_bytes())?;
        return Ok(key);
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        bail!("{} is not a plain file", path.display());
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    if meta.uid() != unsafe { libc::geteuid() } {
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
    let bytes: [u8; KEY_BYTES] = std::fs::read(path)?
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
    fn a_key_others_can_read_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.key");
        load_or_create_key(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create_key(&path).is_err());
    }
}
