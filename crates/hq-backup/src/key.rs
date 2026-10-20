//! What opens a backup: a passphrase, or a key file made by `Secret::generate_key_file`.

use std::io::{Read, Write};
use std::path::Path;

use ring::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroizing;

use crate::error::{BackupError, Result, refused};

pub(crate) const KEY_LEN: usize = 32;
/// Shorter passphrases are refused. A key file is stronger and needs no thinking up.
const MIN_PASSPHRASE_CHARS: usize = 10;
/// A key or passphrase file larger than this is something else.
const MAX_SECRET_FILE_BYTES: u64 = 4096;

/// Argon2id settings for a passphrase. They are stored in the backup, so a backup made with one
/// setting opens with any build that accepts that setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Cost {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

impl Default for Argon2Cost {
    /// 64 MiB, 3 passes, 1 lane: about half a second on a mid-range phone, and a real cost to
    /// anyone guessing passphrases offline.
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024,
            passes: 3,
            lanes: 1,
        }
    }
}

/// What a backup file may ask of the machine that opens it. A damaged or hostile header must not
/// be able to demand gigabytes of memory.
pub(crate) const MAX_MEMORY_KIB: u32 = 256 * 1024;
pub(crate) const MAX_PASSES: u32 = 10;
pub(crate) const MAX_LANES: u32 = 4;
const MIN_MEMORY_KIB_PER_LANE: u32 = 8;

impl Argon2Cost {
    pub(crate) fn check(self) -> Result<()> {
        let ok = self.lanes >= 1
            && self.lanes <= MAX_LANES
            && self.passes >= 1
            && self.passes <= MAX_PASSES
            && self.memory_kib >= MIN_MEMORY_KIB_PER_LANE * self.lanes
            && self.memory_kib <= MAX_MEMORY_KIB;
        if ok {
            Ok(())
        } else {
            Err(BackupError::Corrupt(
                "the key derivation settings in the backup are outside what this build accepts"
                    .into(),
            ))
        }
    }
}

enum Inner {
    Passphrase(Zeroizing<String>),
    Key(Zeroizing<[u8; KEY_LEN]>),
}

/// A passphrase or a key. The bytes are wiped when it is dropped and never printed.
pub struct Secret(Inner);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.0 {
            Inner::Passphrase(_) => "Secret(passphrase)",
            Inner::Key(_) => "Secret(key file)",
        })
    }
}

impl Secret {
    /// A passphrase. Trailing line breaks are dropped, so one read from a file works.
    pub fn passphrase(text: &str) -> Result<Self> {
        let text = text.trim_end_matches(['\r', '\n']);
        if text.chars().count() < MIN_PASSPHRASE_CHARS {
            return refused(format!(
                "a passphrase needs at least {MIN_PASSPHRASE_CHARS} characters; a key file \
                 (hq backup keygen) is stronger and needs no thinking up"
            ));
        }
        Ok(Self(Inner::Passphrase(Zeroizing::new(text.to_string()))))
    }

    /// A passphrase read from the environment variable `var`. The copy this makes is wiped when
    /// it is dropped, which a plain `std::env::var` result is not.
    pub fn passphrase_from_env(var: &str) -> Result<Self> {
        let text = Zeroizing::new(
            std::env::var(var).map_err(|_| BackupError::Refused(format!("{var} is not set")))?,
        );
        Self::passphrase(&text)
    }

    /// A key read from a file made by [`Secret::generate_key_file`]. The file must be a plain
    /// file that only you can read.
    pub fn key_file(path: &Path) -> Result<Self> {
        let text = read_private_file(path)?;
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        hex::decode_to_slice(text.trim(), key.as_mut()).map_err(|_| {
            BackupError::Refused(format!(
                "{} is not a key file (expected {} hex characters)",
                path.display(),
                KEY_LEN * 2
            ))
        })?;
        Ok(Self(Inner::Key(key)))
    }

    /// Writes a new random key to `path`, readable only by you. Refuses to replace a file.
    pub fn generate_key_file(path: &Path) -> Result<()> {
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        SystemRandom::new()
            .fill(key.as_mut())
            .map_err(|_| BackupError::Refused("the system has no source of randomness".into()))?;
        let mut file = crate::fsutil::create_private_file(path)?;
        let mut line = Zeroizing::new(String::with_capacity(KEY_LEN * 2 + 1));
        line.push_str(&hex::encode(key.as_ref()));
        line.push('\n');
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    pub(crate) fn derive(
        &self,
        kdf: &crate::crypto::KdfParams,
    ) -> Result<Zeroizing<[u8; KEY_LEN]>> {
        use crate::crypto::KdfParams;
        match (&self.0, kdf) {
            (Inner::Key(key), KdfParams::None) => Ok(Zeroizing::new(**key)),
            (Inner::Passphrase(text), KdfParams::Argon2id { cost, salt }) => {
                cost.check()?;
                let params =
                    argon2::Params::new(cost.memory_kib, cost.passes, cost.lanes, Some(KEY_LEN))
                        .map_err(|e| {
                            BackupError::Corrupt(format!("key derivation settings: {e}"))
                        })?;
                let argon = argon2::Argon2::new(
                    argon2::Algorithm::Argon2id,
                    argon2::Version::V0x13,
                    params,
                );
                let mut out = Zeroizing::new([0u8; KEY_LEN]);
                argon
                    .hash_password_into(text.as_bytes(), salt, out.as_mut())
                    .map_err(|e| BackupError::Corrupt(format!("key derivation: {e}")))?;
                Ok(out)
            }
            (Inner::Key(_), KdfParams::Argon2id { .. }) => refused(
                "this backup was made with a passphrase; give the passphrase, not a key file",
            ),
            (Inner::Passphrase(_), KdfParams::None) => {
                refused("this backup was made with a key file; give the key file, not a passphrase")
            }
        }
    }

    pub(crate) fn is_passphrase(&self) -> bool {
        matches!(self.0, Inner::Passphrase(_))
    }
}

/// Reads a small secret file after checking nobody else can read it. On Unix that means a plain
/// file (not a link) owned by the current user with no group or world access.
pub fn read_private_file(path: &Path) -> Result<Zeroizing<String>> {
    let file = crate::fsutil::open_private(path)?;
    // Room for the longest allowed file up front, so reading never reallocates and leaves a
    // second copy of the secret behind in freed memory.
    let mut text = Zeroizing::new(String::with_capacity(MAX_SECRET_FILE_BYTES as usize + 1));
    file.take(MAX_SECRET_FILE_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_SECRET_FILE_BYTES {
        return refused(format!(
            "{} is too large to be a key or passphrase file",
            path.display()
        ));
    }
    Ok(text)
}
