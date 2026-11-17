//! Sealing one object at a time, for storage that holds many small things (the HQ Suite's cold
//! originals) instead of one archive.
//!
//! The same ChaCha20-Poly1305 container as a backup, under a key the caller keeps. A key comes
//! from a recovery code the owner writes down (Argon2id, so a code is not guessable offline), or is
//! random. Deriving it costs about half a second, so a caller derives it once and seals many.

use std::io::{Cursor, Read, Write};

use zeroize::Zeroizing;

use crate::crypto::{Header, KdfParams, OpenReader, SealWriter, classify, random};
use crate::error::{Result, refused};
use crate::key::{Argon2Cost, KEY_LEN, Secret};

/// Bytes of salt a recovery code is stretched with. Not secret; stored beside what it protects.
pub const SALT_LEN: usize = 16;

/// Characters in a recovery code, without the dashes. Base 32, so 100 bits.
const CODE_CHARS: usize = 20;
const CODE_GROUP: usize = 4;
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// A key that seals and opens objects. The bytes are wiped when it is dropped and never printed.
pub struct ObjectKey(Zeroizing<[u8; KEY_LEN]>);

impl std::fmt::Debug for ObjectKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ObjectKey")
    }
}

/// A new random salt for [`ObjectKey::from_recovery_code`].
pub fn random_salt() -> Result<[u8; SALT_LEN]> {
    random()
}

/// A new recovery code to write down, such as `K7QM-2XPA-R9WD-H4TE-VN3C`. It cannot be confused
/// letter for letter (no 0, 1, I or O), and a person can read it back over the phone.
pub fn new_recovery_code() -> Result<String> {
    let bytes: [u8; CODE_CHARS] = random()?;
    let mut out = String::with_capacity(CODE_CHARS + CODE_CHARS / CODE_GROUP);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % CODE_GROUP == 0 {
            out.push('-');
        }
        // 256 is a multiple of 32, so every letter is equally likely.
        out.push(CODE_ALPHABET[(*b as usize) % CODE_ALPHABET.len()] as char);
    }
    Ok(out)
}

/// A code as typed: capitals, no dashes or spaces. `None` if it is not a code this build makes.
fn clean_code(typed: &str) -> Option<String> {
    let cleaned: String = typed
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '\t' | '\r' | '\n'))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let ok = cleaned.len() == CODE_CHARS && cleaned.bytes().all(|b| CODE_ALPHABET.contains(&b));
    ok.then_some(cleaned)
}

impl ObjectKey {
    /// A random key, for a store the owner never needs to reopen by hand.
    pub fn random() -> Result<Self> {
        Ok(Self(Zeroizing::new(random()?)))
    }

    /// The key a recovery code and its salt make. The same code and salt always make the same key.
    pub fn from_recovery_code(code: &str, salt: &[u8; SALT_LEN]) -> Result<Self> {
        let Some(cleaned) = clean_code(code) else {
            return refused(
                "that is not a recovery code: it is 20 letters and digits, such as K7QM-2XPA-R9WD-H4TE-VN3C",
            );
        };
        let secret = Secret::passphrase(&cleaned)?;
        let kdf = KdfParams::Argon2id {
            cost: Argon2Cost::default(),
            salt: *salt,
        };
        Ok(Self(secret.derive(&kdf)?))
    }
}

/// `plain` sealed under `key`. Two seals of the same bytes differ (a fresh nonce each time).
pub fn seal(plain: &[u8], key: &ObjectKey) -> Result<Vec<u8>> {
    let header = Header::new(false, Argon2Cost::default())?;
    let mut writer = SealWriter::new(Vec::new(), &key.0, &header)?;
    writer.write_all(plain)?;
    Ok(writer.finish()?)
}

/// What `sealed` holds, or an error saying it is not an object this key opens: the wrong key, a
/// damaged file, or something else.
pub fn open(sealed: &[u8], key: &ObjectKey) -> Result<Vec<u8>> {
    let mut input = Cursor::new(sealed);
    let (header, raw) = Header::read(&mut input)?;
    if header.kdf != KdfParams::None {
        return refused("this object was sealed with a passphrase, not an object key");
    }
    let reader = OpenReader::new(input, &key.0, &header, raw);
    let failure = reader.failure();
    let mut out = Vec::new();
    let mut reader = reader;
    reader
        .read_to_end(&mut out)
        .map_err(|e| classify(e, failure.get()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_opens_with_its_key_and_nothing_else() {
        let key = ObjectKey::random().unwrap();
        let sealed = seal(b"the original file", &key).unwrap();
        assert_ne!(&sealed[..], b"the original file");
        assert_eq!(open(&sealed, &key).unwrap(), b"the original file");
        assert!(open(&sealed, &ObjectKey::random().unwrap()).is_err());
        let mut damaged = sealed.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(open(&damaged, &key).is_err());
        assert!(open(b"short", &key).is_err());
        assert_ne!(seal(b"x", &key).unwrap(), seal(b"x", &key).unwrap());
    }

    #[test]
    fn a_recovery_code_makes_the_same_key_as_typed_in_any_style() {
        let code = new_recovery_code().unwrap();
        assert_eq!(code.len(), 24);
        let salt = random_salt().unwrap();
        let key = ObjectKey::from_recovery_code(&code, &salt).unwrap();
        let sealed = seal(b"bytes", &key).unwrap();
        let typed = code.to_lowercase().replace('-', " ");
        let again = ObjectKey::from_recovery_code(&typed, &salt).unwrap();
        assert_eq!(open(&sealed, &again).unwrap(), b"bytes");
        let other_salt = random_salt().unwrap();
        let elsewhere = ObjectKey::from_recovery_code(&code, &other_salt).unwrap();
        assert!(open(&sealed, &elsewhere).is_err());
    }

    #[test]
    fn something_that_is_not_a_recovery_code_is_refused() {
        let salt = random_salt().unwrap();
        for bad in [
            "",
            "hunter2",
            "AAAA-AAAA-AAAA-AAAA-AAAI",
            "K7QM-2XPA-R9WD-H4TE-VN3C-XXXX",
        ] {
            assert!(ObjectKey::from_recovery_code(bad, &salt).is_err(), "{bad}");
        }
    }
}
