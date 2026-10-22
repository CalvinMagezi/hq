//! This machine's long-term key: 32 bytes in a file only its owner can read.

use anyhow::{Context, Result, anyhow, bail};
use iroh::SecretKey;
use std::path::Path;

const KEY_BYTES: usize = 32;

pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    if !path.exists() {
        return create(path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode();
        if mode & 0o077 != 0 {
            bail!(
                "{} is readable by others (mode {:o}); chmod 600 it",
                path.display(),
                mode & 0o777
            );
        }
    }
    let bytes: [u8; KEY_BYTES] = std::fs::read(path)?
        .try_into()
        .map_err(|_| anyhow!("{} is not a {KEY_BYTES}-byte key", path.display()))?;
    Ok(SecretKey::from_bytes(&bytes))
}

fn create(path: &Path) -> Result<SecretKey> {
    use std::io::Write;
    let key = SecretKey::generate();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(&key.to_bytes())?;
    Ok(key)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_key_is_made_private_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.key");
        let made = load_or_create_key(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(load_or_create_key(&path).unwrap().public(), made.public());
    }

    #[test]
    fn a_key_others_can_read_or_of_the_wrong_size_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.key");
        load_or_create_key(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(
            load_or_create_key(&path).is_err(),
            "a group-readable key was used"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&path, b"short").unwrap();
        assert!(load_or_create_key(&path).is_err());
    }
}
