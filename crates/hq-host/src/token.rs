//! The operator token: a random secret in a 0600 file next to the socket, so
//! only processes of the same user can talk to the host.

use nix::unistd::geteuid;
use std::fs::{self, OpenOptions};
use std::io::{Error, ErrorKind, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const TOKEN_BYTES: usize = 32;
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const GROUP_OTHER_BITS: u32 = 0o077;
/// A starter that finds the token file still being written waits this long,
/// this many times, before giving up.
const RETRY_PAUSE: Duration = Duration::from_millis(20);
const RETRIES: u32 = 100;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn token_path(dir: &Path) -> PathBuf {
    dir.join("operator.token")
}

/// Creates `dir` (0700) if needed. An existing path must be a real directory
/// (not a symlink) owned by this user.
pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("{} is not a plain directory", dir.display()),
                ));
            }
            if meta.uid() != geteuid().as_raw() {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    format!("{} is owned by another user", dir.display()),
                ));
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => fs::create_dir_all(dir)?,
        Err(e) => return Err(e),
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))
}

pub(crate) fn random_hex() -> std::io::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The token in `path`, if the file exists and is safe to trust: a regular
/// file owned by this user that no one else can read.
fn read_token(path: &Path) -> std::io::Result<Option<String>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !meta.is_file() || meta.uid() != geteuid().as_raw() || meta.mode() & GROUP_OTHER_BITS != 0 {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "{} must be a regular file owned by you with mode 0600; delete it to create a new one",
                path.display()
            ),
        ));
    }
    let token = fs::read_to_string(path)?;
    let token = token.trim();
    Ok((!token.is_empty()).then(|| token.to_string()))
}

/// The existing token, or a new one. A new token is written to a private
/// temporary file and linked into place, so a reader never sees a partial
/// file and two starters agree on whichever link won.
pub fn load_or_create(dir: &Path) -> std::io::Result<String> {
    ensure_dir(dir)?;
    let path = token_path(dir);
    for _ in 0..RETRIES {
        if let Some(token) = read_token(&path)? {
            return Ok(token);
        }
        let tmp = dir.join(format!(
            "operator.token.tmp.{}.{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let token = random_hex()?;
        let _ = fs::remove_file(&tmp);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(FILE_MODE)
            .open(&tmp)?;
        file.write_all(token.as_bytes())?;
        drop(file);
        let linked = fs::hard_link(&tmp, &path);
        let _ = fs::remove_file(&tmp);
        match linked {
            Ok(()) => return Ok(token),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => std::thread::sleep(RETRY_PAUSE),
            Err(e) => return Err(e),
        }
    }
    Err(Error::new(
        ErrorKind::TimedOut,
        format!("{} stayed empty; delete it and start again", path.display()),
    ))
}

/// Compares without stopping at the first differing byte.
pub fn matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    let mut diff = u8::from(a.len() != b.len());
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
    fn padding_that_is_a_multiple_of_256_does_not_match() {
        let token = "a".repeat(64);
        let padded = format!("{token}{}", "b".repeat(256));
        assert!(!matches(&token, &padded));
    }

    #[test]
    fn a_token_file_with_loose_permissions_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        let token = load_or_create(&run).unwrap();
        fs::set_permissions(token_path(&run), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            load_or_create(&run).is_err(),
            "a world-readable token must not be trusted: {token}"
        );
    }

    #[test]
    fn a_symlinked_run_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(load_or_create(&link).is_err());
    }

    #[test]
    fn concurrent_first_starts_agree_on_one_token() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        let tokens: Vec<String> = (0..8)
            .map(|_| {
                let run = run.clone();
                std::thread::spawn(move || load_or_create(&run).unwrap())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect();
        assert!(tokens.windows(2).all(|w| w[0] == w[1]), "{tokens:?}");
        assert_eq!(
            fs::read_to_string(token_path(&run)).unwrap().trim(),
            tokens[0]
        );
    }

    #[test]
    fn comparison_is_exact() {
        assert!(matches("abc", "abc"));
        assert!(!matches("abc", "abd"));
        assert!(!matches("abc", "ab"));
        assert!(!matches("", "x"));
    }
}
