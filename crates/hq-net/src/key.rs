//! WireGuard keys: 32 bytes written as standard base64 with padding, which is always 44 characters.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

pub const KEY_BYTES: usize = 32;
pub const KEY_TEXT_LEN: usize = 44;

/// Decodes exactly one key. Only canonical text is accepted: 44 characters, the standard
/// alphabet, `=` padding, no whitespace and no stray bits in the last character.
pub fn decode(text: &str) -> Option<[u8; KEY_BYTES]> {
    if text.len() != KEY_TEXT_LEN {
        return None;
    }
    // Decoded into a buffer on the stack, so no heap copy of a private key is left behind. The
    // buffer has room for one more group than the 32 bytes, which `decode_slice` needs.
    let mut bytes = [0u8; KEY_BYTES + 3];
    let written = STANDARD.decode_slice(text, &mut bytes).ok()?;
    let key = (written == KEY_BYTES).then(|| {
        let mut key = [0u8; KEY_BYTES];
        key.copy_from_slice(&bytes[..KEY_BYTES]);
        key
    });
    bytes.fill(0);
    key
}

pub fn encode(bytes: &[u8; KEY_BYTES]) -> String {
    STANDARD.encode(bytes)
}

/// Whether the key has the shape `wg genkey` gives a PRIVATE key: the clamped low three bits of the
/// first byte clear and the top two bits of the last byte `01`. A public key has that shape by
/// chance about one time in sixteen, so this is a warning sign, never proof.
pub fn looks_like_private_key(text: &str) -> bool {
    decode(text).is_some_and(|b| b[0] & 0b111 == 0 && b[KEY_BYTES - 1] & 0b1100_0000 == 0b0100_0000)
}

/// A key the applier will carry: well formed and not all zero. An all-zero preshared key means
/// "none" to WireGuard, and an all-zero public key is not a device.
pub fn is_usable(text: &str) -> bool {
    decode(text).is_some_and(|bytes| bytes != [0u8; KEY_BYTES])
}

#[cfg(feature = "tools")]
pub use generate::{PrivateKey, new_preshared_key, public_key_of};

#[cfg(feature = "tools")]
mod generate {
    use super::{KEY_BYTES, decode, encode};
    use curve25519_dalek::montgomery::MontgomeryPoint;
    use rand::RngCore;

    /// A private key held as raw bytes. It has no `Debug` or `Display`, so it cannot reach a log by
    /// accident; `reveal` is the one way to read it, for writing it to its file.
    pub struct PrivateKey([u8; KEY_BYTES]);

    impl PrivateKey {
        /// 32 random bytes, clamped as `wg genkey` does.
        pub fn generate() -> Self {
            let mut bytes = [0u8; KEY_BYTES];
            rand::rngs::OsRng.fill_bytes(&mut bytes);
            bytes[0] &= 248;
            bytes[31] = (bytes[31] & 127) | 64;
            Self(bytes)
        }

        pub fn from_text(text: &str) -> Option<Self> {
            decode(text).map(Self)
        }

        pub fn public(&self) -> String {
            encode(&MontgomeryPoint::mul_base_clamped(self.0).to_bytes())
        }

        pub fn reveal(&self) -> String {
            encode(&self.0)
        }
    }

    /// The public half of a private key given as text, or `None` if it is not a key.
    pub fn public_key_of(private: &str) -> Option<String> {
        PrivateKey::from_text(private).map(|key| key.public())
    }

    /// 32 random bytes for a peer's optional preshared key.
    pub fn new_preshared_key() -> String {
        let mut bytes = [0u8; KEY_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        encode(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn a_key_is_44_standard_base64_characters_and_nothing_else() {
        let good = encode(&[7u8; KEY_BYTES]);
        assert_eq!(good.len(), KEY_TEXT_LEN);
        assert!(decode(&good).is_some());
        let flipped_tail = format!("{}B=", &good[..42]);
        let url_safe = good.replace('+', "-").replace('/', "_");
        for bad in [
            "".to_string(),
            good[..43].to_string(),
            format!("{good}A"),
            format!(" {}", &good[1..]),
            format!("{}\n", &good[..43]),
            good.replace('=', "A"),
            flipped_tail,
            format!("{}!", &good[..43]),
            // 33 bytes encode to 44 characters with no padding at all.
            "A".repeat(44),
        ] {
            let ok = decode(&bad).is_some();
            // The only text above that may decode is one that is exactly 32 bytes; none is.
            assert!(!ok, "{bad:?} must be refused");
        }
        if url_safe != good {
            assert!(
                decode(&url_safe).is_none(),
                "base64url is not WireGuard text"
            );
        }
    }

    #[cfg(feature = "tools")]
    #[test]
    fn a_generated_private_key_has_the_private_shape_and_a_known_public_key_does_not() {
        assert!(looks_like_private_key(&PrivateKey::generate().reveal()));
        // RFC 7748 section 6.1, Alice's public key: first byte 0x85, so not clamped.
        let alice_public = "hSDwCYkwp1R0i33ctD73Wg2/Og0mOBr066SpjqqbTmo=";
        assert!(!looks_like_private_key(alice_public));
        assert!(!looks_like_private_key("short"));
    }

    #[test]
    fn the_zero_key_decodes_but_is_not_usable() {
        assert!(decode(ZERO_KEY).is_some());
        assert!(!is_usable(ZERO_KEY));
        assert!(is_usable(&encode(&[1u8; KEY_BYTES])));
    }

    #[cfg(feature = "tools")]
    #[test]
    fn public_keys_match_the_rfc_7748_vector() {
        // RFC 7748 section 6.1: Alice's private key and public key, both as 32 bytes.
        let alice_private = hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let alice_public = hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");
        // `from_text` takes the clamped form WireGuard stores; the vector's key already is clamped
        // by the scalar multiplication, so derive from the raw bytes through the same path.
        let key = PrivateKey::from_text(&encode(&alice_private)).unwrap();
        assert_eq!(key.public(), encode(&alice_public));
        assert_eq!(public_key_of(&key.reveal()).unwrap(), key.public());
    }

    #[cfg(feature = "tools")]
    #[test]
    fn generated_keys_are_clamped_distinct_and_round_trip() {
        let a = PrivateKey::generate();
        let b = PrivateKey::generate();
        assert_ne!(a.reveal(), b.reveal());
        let raw = decode(&a.reveal()).unwrap();
        assert_eq!(raw[0] & 7, 0);
        assert_eq!(raw[31] & 0b1100_0000, 0b0100_0000);
        assert!(is_usable(&a.public()));
        assert!(is_usable(&new_preshared_key()));
        assert!(PrivateKey::from_text("nope").is_none());
    }

    #[cfg(feature = "tools")]
    fn hex(text: &str) -> [u8; KEY_BYTES] {
        let mut out = [0u8; KEY_BYTES];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }
}
