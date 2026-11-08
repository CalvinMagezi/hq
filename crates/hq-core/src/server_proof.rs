//! Lets a client that holds the web token check that the server on a loopback port is the HQ
//! that holds it too, without sending the token. Any program on the machine can listen on a free
//! loopback port, so a client must not send its Bearer token to whatever answers there.
//!
//! The client sends a fresh random nonce (`GET /api/hello?proof=<nonce>`, base64url without
//! padding, 16 to 64 bytes) and HQ answers `"proof"` in the hello body:
//! `base64url(HMAC-SHA256(key = the web token's UTF-8 bytes, message = "hq-server-proof/1" || nonce))`.
//! Only something that knows the token can answer, and the nonce is new each time, so an answer
//! cannot be replayed. See `docs/security/WEB_AUTH.md`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Domain separation: no other HQ message is signed with the web token.
pub const LABEL: &[u8] = b"hq-server-proof/1";
/// The query parameter on `/api/hello`, and the field of its answer.
pub const PARAM: &str = "proof";
pub const NONCE_MIN_BYTES: usize = 16;
pub const NONCE_MAX_BYTES: usize = 64;
/// What a client sends: 32 random bytes.
pub const NONCE_BYTES: usize = 32;
/// The longest text [`NONCE_MAX_BYTES`] encodes to, checked before any decoding.
const NONCE_MAX_CHARS: usize = NONCE_MAX_BYTES.div_ceil(3) * 4;

/// Why a nonce was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonceError {
    NotBase64,
    Length,
}

impl std::fmt::Display for NonceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotBase64 => "proof must be base64url without padding",
            Self::Length => "proof must be 16 to 64 bytes",
        })
    }
}

impl std::error::Error for NonceError {}

/// The nonce bytes a client sent, or why they are refused.
pub fn decode_nonce(text: &str) -> Result<Vec<u8>, NonceError> {
    if text.len() > NONCE_MAX_CHARS {
        return Err(NonceError::Length);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| NonceError::NotBase64)?;
    if !(NONCE_MIN_BYTES..=NONCE_MAX_BYTES).contains(&bytes.len()) {
        return Err(NonceError::Length);
    }
    Ok(bytes)
}

/// HQ's answer to `nonce`, base64url without padding.
pub fn answer(token: &str, nonce: &[u8]) -> String {
    let mut message = Vec::with_capacity(LABEL.len() + nonce.len());
    message.extend_from_slice(LABEL);
    message.extend_from_slice(nonce);
    URL_SAFE_NO_PAD.encode(crate::approval_key::hmac_sha256(token.as_bytes(), &message))
}

/// A fresh nonce for a client to send, already encoded.
pub fn new_nonce() -> String {
    let bytes: [u8; NONCE_BYTES] = rand::random();
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Whether `proof` is the answer to `nonce` from a server holding `token`.
pub fn verify(token: &str, nonce: &str, proof: &str) -> bool {
    let Ok(bytes) = decode_nonce(nonce) else {
        return false;
    };
    crate::pairing::constant_time_eq(answer(token, &bytes).as_bytes(), proof.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "example-web-token";
    /// Bytes 0..32, the vector in docs/security/WEB_AUTH.md.
    const NONCE: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    const PROOF: &str = "s887Mn5Ba7kWekr_rkJlZl5zEYzF3_5f3jGwuiy_PXk";

    #[test]
    fn answer_matches_the_documented_vector() {
        let nonce = decode_nonce(NONCE).unwrap();
        assert_eq!(nonce, (0u8..32).collect::<Vec<_>>());
        assert_eq!(answer(TOKEN, &nonce), PROOF);
        assert!(verify(TOKEN, NONCE, PROOF));
    }

    #[test]
    fn another_token_or_nonce_does_not_verify() {
        assert!(!verify("another-token", NONCE, PROOF));
        let other = new_nonce();
        assert!(!verify(TOKEN, &other, PROOF));
        assert!(!verify(TOKEN, NONCE, ""));
        assert!(!verify(TOKEN, "!", PROOF));
    }

    #[test]
    fn fresh_nonces_decode_and_differ() {
        let (a, b) = (new_nonce(), new_nonce());
        assert_ne!(a, b);
        assert_eq!(decode_nonce(&a).unwrap().len(), NONCE_BYTES);
        let proof = answer(TOKEN, &decode_nonce(&a).unwrap());
        assert!(verify(TOKEN, &a, &proof));
    }

    #[test]
    fn bad_nonces_are_refused() {
        assert_eq!(decode_nonce(""), Err(NonceError::Length));
        assert_eq!(
            decode_nonce(&URL_SAFE_NO_PAD.encode([0u8; 15])),
            Err(NonceError::Length)
        );
        assert_eq!(
            decode_nonce(&URL_SAFE_NO_PAD.encode([0u8; 65])),
            Err(NonceError::Length)
        );
        assert_eq!(decode_nonce(&"A".repeat(10_000)), Err(NonceError::Length));
        assert!(decode_nonce(&URL_SAFE_NO_PAD.encode([0u8; 16])).is_ok());
        assert!(decode_nonce(&URL_SAFE_NO_PAD.encode([0u8; 64])).is_ok());
        // Standard base64 and padding are not base64url without padding.
        assert_eq!(
            decode_nonce("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="),
            Err(NonceError::NotBase64)
        );
        assert_eq!(
            decode_nonce("++++++++++++++++++++++++"),
            Err(NonceError::NotBase64)
        );
    }
}
