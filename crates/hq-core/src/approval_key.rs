//! Per-install secret that signs owner approvals, kept outside the vault and
//! the database so a database write alone cannot forge one.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

const KEY_FILE: &str = "approval.key";
const KEY_BYTES: usize = 32;
const HMAC_BLOCK: usize = 64;

/// `<config dir>/approval.key`, next to the HQ config file.
pub fn key_path() -> PathBuf {
    let config = crate::config::HqConfig::config_file_path();
    config
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default()
        .join(KEY_FILE)
}

/// Reads the key, creating it (mode 0600, exclusive create) on first use.
pub fn load_or_create_key() -> Result<Vec<u8>> {
    load_or_create_key_at(&key_path())
}

pub fn load_or_create_key_at(path: &std::path::Path) -> Result<Vec<u8>> {
    use std::io::Write;
    if let Ok(existing) = std::fs::read(path)
        && existing.len() >= KEY_BYTES
    {
        return Ok(existing);
    }
    std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))?;
    let key: Vec<u8> = (0..KEY_BYTES).map(|_| rand::random::<u8>()).collect();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(&key).context("write approval key")?;
            Ok(key)
        }
        // Another process created it first; use theirs.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::read(path).context("read approval key")
        }
        Err(e) => Err(e).context("create approval key"),
    }
}

/// HMAC-SHA256 as lowercase hex.
pub fn hmac_hex(key: &[u8], message: &str) -> String {
    let mut block = [0u8; HMAC_BLOCK];
    if key.len() > HMAC_BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let mut inner = Sha256::new();
    inner.update(pad(0x36));
    inner.update(message.as_bytes());
    let mut outer = Sha256::new();
    outer.update(pad(0x5c));
    outer.update(inner.finalize());
    hex::encode(outer.finalize())
}

pub fn verify_hex(key: &[u8], message: &str, mac: &str) -> bool {
    crate::pairing::constant_time_eq(hmac_hex(key, message).as_bytes(), mac.as_bytes())
}

/// Signs `engaged` approval of one value item.
pub fn value_approval_message(item_id: &str) -> String {
    format!("value-approval|{item_id}")
}

/// Signs the owner's approval of one self-update run at a given artifact.
pub fn self_update_message(run_id: i64, tree: &str, binary_sha256: &str) -> String {
    format!("self-update|{run_id}|{tree}|{binary_sha256}")
}

/// SHA-256 of a file as lowercase hex.
pub fn file_sha256(path: &std::path::Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc4231_case_2() {
        assert_eq!(
            hmac_hex(b"Jefe", "what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn key_is_created_once_private_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/approval.key");
        let first = load_or_create_key_at(&path).unwrap();
        assert_eq!(first.len(), KEY_BYTES);
        assert_eq!(first, load_or_create_key_at(&path).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn verify_rejects_wrong_key_message_or_mac() {
        let mac = hmac_hex(b"k1", "m");
        assert!(verify_hex(b"k1", "m", &mac));
        assert!(!verify_hex(b"k2", "m", &mac));
        assert!(!verify_hex(b"k1", "other", &mac));
        assert!(!verify_hex(b"k1", "m", ""));
    }
}
