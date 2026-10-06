//! The operator token: a random secret in a 0600 file next to the socket, so
//! only processes of the same user can talk to the host.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const TOKEN_BYTES: usize = 32;
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

pub fn token_path(dir: &Path) -> PathBuf {
    dir.join("operator.token")
}

/// Creates `dir` (0700) if needed.
pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))
}

fn random_hex() -> std::io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The existing token, or a new one written with mode 0600.
pub fn load_or_create(dir: &Path) -> std::io::Result<String> {
    ensure_dir(dir)?;
    let path = token_path(dir);
    match fs::read_to_string(&path) {
        Ok(token) if !token.trim().is_empty() => return Ok(token.trim().to_string()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let token = random_hex()?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(FILE_MODE)
        .open(&path)?;
    file.write_all(token.as_bytes())?;
    Ok(token)
}

/// Compares without stopping at the first differing byte.
pub fn matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().min(b.len()) {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_created_once_with_private_modes() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("run");
        let first = load_or_create(&sub).unwrap();
        assert_eq!(first.len(), TOKEN_BYTES * 2);
        assert_eq!(load_or_create(&sub).unwrap(), first);
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&sub), DIR_MODE);
        assert_eq!(mode(&token_path(&sub)), FILE_MODE);
    }

    #[test]
    fn comparison_is_exact() {
        assert!(matches("abc", "abc"));
        assert!(!matches("abc", "abd"));
        assert!(!matches("abc", "ab"));
        assert!(!matches("", "x"));
    }
}
