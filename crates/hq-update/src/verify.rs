//! Minisign (Ed25519) verification via `minisign-verify`. Nothing here shells out.

use crate::error::{Result, UpdateError};
use minisign_verify::{PublicKey, Signature};

/// Accepts either a bare base64 key line (`RWQ...`) or the full contents of a
/// minisign `.pub` file (comment line plus key line).
pub fn parse_public_key(text: &str) -> Result<PublicKey> {
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .ok_or_else(|| UpdateError::NoPublicKey("key text is empty".into()))?;
    PublicKey::from_base64(line)
        .map_err(|e| UpdateError::NoPublicKey(format!("not a minisign public key: {e}")))
}

/// Verifies `signature` (the text of a `.minisig` file) over `data`. Only
/// prehashed signatures, the `minisign -S` default, are accepted.
pub fn verify(key: &PublicKey, what: &str, data: &[u8], signature: &[u8]) -> Result<()> {
    let fail = |reason: String| UpdateError::Signature {
        what: what.to_string(),
        reason,
    };
    let text = std::str::from_utf8(signature).map_err(|_| fail("signature is not UTF-8".into()))?;
    let sig = Signature::decode(text).map_err(|e| fail(format!("malformed signature: {e}")))?;
    key.verify(data, &sig, false)
        .map_err(|e| fail(e.to_string()))
}
