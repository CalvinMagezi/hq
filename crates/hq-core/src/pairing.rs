//! One-time owner pairing codes for chat relays.
//!
//! A relay with no configured owner refuses everyone. The operator runs
//! `hq pair`, which stores only a SHA-256 of a random code under
//! `_system/.pairing-<platform>.json`. The first sender to present the code
//! within the TTL becomes the owner. The code is single use and burns after a
//! few wrong guesses.

use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const PAIRING_TTL_SECS: i64 = 15 * 60;
pub const MAX_WRONG_ATTEMPTS: u32 = 5;
const CODE_LEN: usize = 10;
/// No 0/O/1/I/L so a code read off a terminal survives being retyped.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairPlatform {
    Telegram,
    Discord,
}

impl PairPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            PairPlatform::Telegram => "telegram",
            PairPlatform::Discord => "discord",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    NoPendingCode,
    Expired,
    WrongCode,
    TooManyAttempts,
    Io(String),
}

#[derive(Serialize, Deserialize)]
struct PairingRecord {
    /// Random per code, so equal codes never share a stored hash.
    salt: String,
    hash: String,
    expires_at: i64,
    wrong_attempts: u32,
}

fn record_path(vault: &Path, platform: PairPlatform) -> PathBuf {
    vault
        .join("_system")
        .join(format!(".pairing-{}.json", platform.as_str()))
}

fn hash_code(salt: &str, code: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(normalize(code).as_bytes());
    hasher.finalize().into()
}

/// Holds an exclusive advisory lock for one platform until dropped, so
/// concurrent redeem attempts cannot lose a wrong-attempt count or both win.
struct PairLock(#[allow(dead_code)] std::fs::File);

#[cfg(unix)]
fn lock(vault: &Path, platform: PairPlatform) -> Result<PairLock, PairError> {
    use std::os::unix::io::AsRawFd;
    let path = vault.join("_system").join(format!(".pairing-{}.lock", platform.as_str()));
    let io = |e: std::io::Error| PairError::Io(e.to_string());
    std::fs::create_dir_all(path.parent().unwrap_or(vault)).map_err(io)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(io)?;
    // SAFETY: flock on a descriptor this function owns.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(PairLock(file))
}

#[cfg(not(unix))]
fn lock(_vault: &Path, _platform: PairPlatform) -> Result<PairLock, PairError> {
    Err(PairError::Io("pairing needs a unix host".into()))
}

fn normalize(code: &str) -> String {
    code.trim().to_ascii_uppercase().replace('-', "")
}

/// Compares every byte regardless of where the first mismatch is.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Writes a fresh pending code (replacing any earlier one) and returns it in clear.
pub fn create_pairing_code(
    vault: &Path,
    platform: PairPlatform,
    now: i64,
) -> Result<String, PairError> {
    let mut rng = rand::thread_rng();
    let code: String = (0..CODE_LEN)
        .map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char)
        .collect();
    let _guard = lock(vault, platform)?;
    let salt = hex::encode(rng.r#gen::<[u8; 16]>());
    let record = PairingRecord {
        hash: hex::encode(hash_code(&salt, &code)),
        salt,
        expires_at: now + PAIRING_TTL_SECS,
        wrong_attempts: 0,
    };
    write_record(vault, platform, &record)?;
    Ok(format!(
        "{}-{}",
        &code[..CODE_LEN / 2],
        &code[CODE_LEN / 2..]
    ))
}

/// True when a code is waiting (unexpired) for this platform.
pub fn has_pending_code(vault: &Path, platform: PairPlatform, now: i64) -> bool {
    read_record(vault, platform).is_some_and(|r| r.expires_at > now)
}

/// Checks `presented` against the pending code. Success consumes it: the record
/// file is renamed away first, so of two concurrent redeemers only one wins.
pub fn redeem_pairing_code(
    vault: &Path,
    platform: PairPlatform,
    presented: &str,
    now: i64,
) -> Result<(), PairError> {
    let path = record_path(vault, platform);
    let _guard = lock(vault, platform)?;
    let mut record = read_record(vault, platform).ok_or(PairError::NoPendingCode)?;
    if record.expires_at <= now {
        let _ = std::fs::remove_file(&path);
        return Err(PairError::Expired);
    }
    let expected = hex::decode(&record.hash).unwrap_or_default();
    if !constant_time_eq(&expected, &hash_code(&record.salt, presented)) {
        record.wrong_attempts += 1;
        if record.wrong_attempts >= MAX_WRONG_ATTEMPTS {
            let _ = std::fs::remove_file(&path);
            return Err(PairError::TooManyAttempts);
        }
        write_record(vault, platform, &record)?;
        return Err(PairError::WrongCode);
    }
    let claimed = path.with_extension("claimed");
    std::fs::rename(&path, &claimed).map_err(|_| PairError::NoPendingCode)?;
    let _ = std::fs::remove_file(&claimed);
    Ok(())
}

fn read_record(vault: &Path, platform: PairPlatform) -> Option<PairingRecord> {
    let raw = std::fs::read_to_string(record_path(vault, platform)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_record(vault: &Path, platform: PairPlatform, record: &PairingRecord) -> Result<(), PairError> {
    use std::io::Write;
    let path = record_path(vault, platform);
    let io = |e: std::io::Error| PairError::Io(e.to_string());
    std::fs::create_dir_all(path.parent().unwrap_or(vault)).map_err(io)?;
    let json = serde_json::to_string(record).map_err(|e| PairError::Io(e.to_string()))?;
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(io)?;
    file.write_all(json.as_bytes()).map_err(io)?;
    file.sync_all().map_err(io)?;
    std::fs::rename(&tmp, &path).map_err(io)
}

/// Parses `/pair CODE` (Telegram, with optional `@botname`) or `!pair CODE`.
pub fn parse_pair_command(text: &str) -> Option<&str> {
    let mut parts = text.split_whitespace();
    let head = parts.next()?;
    let cmd = head.split('@').next()?;
    if !matches!(cmd, "/pair" | "!pair") {
        return None;
    }
    parts.next()
}

/// Current unix time, for callers that do not inject a clock.
pub fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn correct_code_redeems_exactly_once() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Telegram, 1000).unwrap();
        assert_eq!(
            redeem_pairing_code(v.path(), PairPlatform::Telegram, &code, 1001),
            Ok(())
        );
        assert_eq!(
            redeem_pairing_code(v.path(), PairPlatform::Telegram, &code, 1002),
            Err(PairError::NoPendingCode)
        );
    }

    #[test]
    fn code_is_case_and_dash_insensitive() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Discord, 0).unwrap();
        let sloppy = code.to_lowercase().replace('-', " ").replace(' ', "");
        assert_eq!(
            redeem_pairing_code(v.path(), PairPlatform::Discord, &sloppy, 1),
            Ok(())
        );
    }

    #[test]
    fn wrong_code_is_refused_and_does_not_consume() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        assert_eq!(
            redeem_pairing_code(v.path(), PairPlatform::Telegram, "AAAAA-AAAAA", 1),
            Err(PairError::WrongCode)
        );
        assert_eq!(
            redeem_pairing_code(v.path(), PairPlatform::Telegram, &code, 2),
            Ok(())
        );
    }

    #[test]
    fn expired_code_is_refused() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        assert_eq!(
            redeem_pairing_code(v.path(), PairPlatform::Telegram, &code, PAIRING_TTL_SECS),
            Err(PairError::Expired)
        );
        assert!(!has_pending_code(v.path(), PairPlatform::Telegram, 0));
    }

    #[test]
    fn repeated_wrong_guesses_burn_the_code() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        for _ in 0..MAX_WRONG_ATTEMPTS {
            let _ = redeem_pairing_code(v.path(), PairPlatform::Telegram, "ZZZZZ-ZZZZZ", 1);
        }
        assert_eq!(
            redeem_pairing_code(v.path(), PairPlatform::Telegram, &code, 2),
            Err(PairError::NoPendingCode)
        );
    }

    #[test]
    fn code_for_one_platform_does_not_open_the_other() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        assert_eq!(
            redeem_pairing_code(v.path(), PairPlatform::Discord, &code, 1),
            Err(PairError::NoPendingCode)
        );
    }

    #[test]
    fn only_the_hash_is_stored() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        let raw = std::fs::read_to_string(record_path(v.path(), PairPlatform::Telegram)).unwrap();
        assert!(!raw.contains(&code.replace('-', "")));
    }

    #[test]
    fn stored_hash_is_salted_and_file_is_private() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        let first = read_record(v.path(), PairPlatform::Telegram).unwrap();
        assert_ne!(first.hash, hex::encode(Sha256::digest(normalize(&code).as_bytes())));
        create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        assert_ne!(first.salt, read_record(v.path(), PairPlatform::Telegram).unwrap().salt);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(record_path(v.path(), PairPlatform::Telegram)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn concurrent_redeemers_produce_exactly_one_winner() {
        let v = vault();
        let code = create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        let path = std::sync::Arc::new(v.path().to_path_buf());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (path, code) = (path.clone(), code.clone());
                std::thread::spawn(move || redeem_pairing_code(&path, PairPlatform::Telegram, &code, 1).is_ok())
            })
            .collect();
        let wins = handles.into_iter().filter(|_| true).map(|h| h.join().unwrap()).filter(|w| *w).count();
        assert_eq!(wins, 1);
    }

    #[test]
    fn constant_time_eq_matches_slice_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn parses_pair_commands() {
        assert_eq!(parse_pair_command("/pair ABCDE-FGHJK"), Some("ABCDE-FGHJK"));
        assert_eq!(parse_pair_command("/pair@mybot X"), Some("X"));
        assert_eq!(parse_pair_command("!pair X"), Some("X"));
        assert_eq!(parse_pair_command("/pair"), None);
        assert_eq!(parse_pair_command("hello /pair X"), None);
    }
}
