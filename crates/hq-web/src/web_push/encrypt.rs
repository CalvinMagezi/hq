//! Message encryption for Web Push (RFC 8291 over the `aes128gcm` coding of RFC 8188).
//!
//! The push service relays the body without being able to read it: only the browser that
//! holds the subscription's private key and auth secret can decrypt it.

use anyhow::{Result, anyhow, bail};
use ring::{aead, agreement, hkdf, rand};

/// Length of an uncompressed P-256 public key, as browsers hand out in `keys.p256dh`.
pub(crate) const P256_PUBLIC_LEN: usize = 65;
/// Length of the subscription's `keys.auth` secret.
pub(crate) const AUTH_SECRET_LEN: usize = 16;
const SALT_LEN: usize = 16;
const TAG_LEN: usize = 16;
/// Record size written in the header. The message is one record, so this only has to be
/// larger than it.
const RECORD_SIZE: u32 = 4096;
/// Largest plaintext that still fits in one record: the record holds the plaintext, the
/// one-byte delimiter and the tag.
pub(crate) const MAX_PLAINTEXT: usize = RECORD_SIZE as usize - TAG_LEN - 1;

/// A subscription's receiving keys, checked for length.
#[derive(Debug, Clone)]
pub(crate) struct ReceiverKeys {
    pub(crate) p256dh: [u8; P256_PUBLIC_LEN],
    pub(crate) auth: [u8; AUTH_SECRET_LEN],
}

impl ReceiverKeys {
    pub(crate) fn new(p256dh: &[u8], auth: &[u8]) -> Result<Self> {
        let p256dh: [u8; P256_PUBLIC_LEN] = p256dh
            .try_into()
            .map_err(|_| anyhow!("p256dh must be a {P256_PUBLIC_LEN}-byte P-256 point"))?;
        if p256dh[0] != 0x04 {
            bail!("p256dh must be an uncompressed P-256 point");
        }
        let auth: [u8; AUTH_SECRET_LEN] = auth
            .try_into()
            .map_err(|_| anyhow!("auth must be {AUTH_SECRET_LEN} bytes"))?;
        Ok(Self { p256dh, auth })
    }
}

/// Encrypts `plaintext` for one subscription with a fresh sender key and salt. The result is
/// the request body, header included.
pub(crate) fn encrypt(receiver: &ReceiverKeys, plaintext: &[u8]) -> Result<Vec<u8>> {
    if plaintext.len() > MAX_PLAINTEXT {
        bail!(
            "push payload is {} bytes; the limit is {MAX_PLAINTEXT}",
            plaintext.len()
        );
    }
    let rng = rand::SystemRandom::new();
    let sender = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
        .map_err(|_| anyhow!("could not generate a sender key"))?;
    let sender_public = sender
        .compute_public_key()
        .map_err(|_| anyhow!("could not compute the sender key"))?;
    let sender_public: [u8; P256_PUBLIC_LEN] = sender_public
        .as_ref()
        .try_into()
        .map_err(|_| anyhow!("unexpected sender key length"))?;
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, receiver.p256dh);
    let ecdh_secret = agreement::agree_ephemeral(sender, &peer, |s| s.to_vec())
        .map_err(|_| anyhow!("the subscription's p256dh is not a valid P-256 key"))?;
    let mut salt = [0u8; SALT_LEN];
    rand::SecureRandom::fill(&rng, &mut salt).map_err(|_| anyhow!("no randomness for salt"))?;
    encrypt_with(&ecdh_secret, &sender_public, receiver, &salt, plaintext)
}

/// The deterministic part of [`encrypt`]: key derivation and sealing once the shared secret,
/// sender key and salt are fixed. Split out so it can be checked against fixed vectors.
pub(crate) fn encrypt_with(
    ecdh_secret: &[u8],
    sender_public: &[u8; P256_PUBLIC_LEN],
    receiver: &ReceiverKeys,
    salt: &[u8; SALT_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    // RFC 8291 section 3.4: mix the auth secret into the shared secret, bound to both keys.
    let mut key_info = Vec::with_capacity(14 + 2 * P256_PUBLIC_LEN);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(&receiver.p256dh);
    key_info.extend_from_slice(sender_public);
    let ikm: [u8; 32] = expand(
        &hkdf::Salt::new(hkdf::HKDF_SHA256, &receiver.auth).extract(ecdh_secret),
        &key_info,
    )?;

    // RFC 8188 section 2.2 and 2.3: the content key and nonce from the salt.
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
    let cek: [u8; 16] = expand(&prk, b"Content-Encoding: aes128gcm\0")?;
    let nonce: [u8; 12] = expand(&prk, b"Content-Encoding: nonce\0")?;

    // One record, so it is the last one: the plaintext ends with the 0x02 delimiter.
    let mut record = Vec::with_capacity(plaintext.len() + 1 + TAG_LEN);
    record.extend_from_slice(plaintext);
    record.push(0x02);
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| anyhow!("bad content key"))?,
    );
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::empty(),
        &mut record,
    )
    .map_err(|_| anyhow!("encryption failed"))?;

    let mut body = Vec::with_capacity(SALT_LEN + 5 + P256_PUBLIC_LEN + record.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(P256_PUBLIC_LEN as u8);
    body.extend_from_slice(sender_public);
    body.extend_from_slice(&record);
    Ok(body)
}

/// An HKDF output length, which ring takes as a key type.
struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand<const N: usize>(prk: &hkdf::Prk, info: &[u8]) -> Result<[u8; N]> {
    let mut out = [0u8; N];
    prk.expand(&[info], Len(N))
        .and_then(|okm| okm.fill(&mut out))
        .map_err(|_| anyhow!("HKDF expand failed"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;

    fn b64(s: &str) -> Vec<u8> {
        B64.decode(s).unwrap()
    }

    /// Fixed keys and salt. The expected body was produced by the `http_ece` reference
    /// implementation (web-push-libs/encrypted-content-encoding, Python) encrypting the same
    /// plaintext with `version="aes128gcm"`, `rs=4096`, these keys and this salt, and that
    /// implementation decrypts it back.
    #[test]
    fn matches_the_reference_implementation() {
        let receiver = ReceiverKeys::new(
            &b64(
                "BL18c7iLLptM7aYgIrLai-Exk6W1btwm5994QuJM0LXrBgWtp72oOsaiuA1-MUBA-kf_Frg7rIXO2wFEUbt85xo",
            ),
            &b64("AAECAwQFBgcICQoLDA0ODw"),
        )
        .unwrap();
        let sender_public: [u8; P256_PUBLIC_LEN] = b64(
            "BBYLBhUVnrvff9SxGU1CsphsayzMrXmVIbizJikrZBmzk6IOEkrSpW7Op0-NroTY2R0PWsSjPyISJtFoHNElUso",
        )
        .try_into()
        .unwrap();
        let ecdh_secret = b64("vmeLcVHcL34_HpW8m5bqrU3oY6sQVyvRTvf73wDcfkk");
        let salt: [u8; SALT_LEN] = b64("ZGVmZ2hpamtsbW5vcHFycw").try_into().unwrap();
        let body = encrypt_with(
            &ecdh_secret,
            &sender_public,
            &receiver,
            &salt,
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();
        assert_eq!(
            B64.encode(&body),
            "ZGVmZ2hpamtsbW5vcHFycwAAEABBBBYLBhUVnrvff9SxGU1CsphsayzMrXmVIbizJikrZBmzk6IOEkrSpW7Op0-NroTY2R0PWsSjPyISJtFoHNElUspXgvXrNqz1EPQwR5AXECzXDPQn2z1fd09Q9m5Xms3piJWWFZyqnC4SFOhZQdPLyVVO2OkUT5-x2oru"
        );
    }

    /// The receiving side of the scheme, written out from RFC 8291, decrypts a fresh message.
    #[test]
    fn a_fresh_message_decrypts_with_the_receiver_keys() {
        let rng = rand::SystemRandom::new();
        let ua = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua_public = ua.compute_public_key().unwrap();
        let receiver = ReceiverKeys::new(ua_public.as_ref(), &[7u8; AUTH_SECRET_LEN]).unwrap();
        let body = encrypt(&receiver, b"{\"title\":\"hi\"}").unwrap();

        let salt = &body[..SALT_LEN];
        assert_eq!(&body[16..20], &RECORD_SIZE.to_be_bytes());
        assert_eq!(body[20] as usize, P256_PUBLIC_LEN);
        let sender_public: [u8; P256_PUBLIC_LEN] =
            body[21..21 + P256_PUBLIC_LEN].try_into().unwrap();
        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, sender_public);
        let secret = agreement::agree_ephemeral(ua, &peer, |s| s.to_vec()).unwrap();

        let mut key_info = b"WebPush: info\0".to_vec();
        key_info.extend_from_slice(&receiver.p256dh);
        key_info.extend_from_slice(&sender_public);
        let ikm: [u8; 32] = expand(
            &hkdf::Salt::new(hkdf::HKDF_SHA256, &receiver.auth).extract(&secret),
            &key_info,
        )
        .unwrap();
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
        let cek: [u8; 16] = expand(&prk, b"Content-Encoding: aes128gcm\0").unwrap();
        let nonce: [u8; 12] = expand(&prk, b"Content-Encoding: nonce\0").unwrap();
        let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
        let mut record = body[21 + P256_PUBLIC_LEN..].to_vec();
        let plain = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::empty(),
                &mut record,
            )
            .unwrap();
        assert_eq!(plain, b"{\"title\":\"hi\"}\x02");
    }

    #[test]
    fn rejects_bad_keys_and_oversized_payloads() {
        assert!(ReceiverKeys::new(&[4u8; 64], &[0u8; 16]).is_err());
        assert!(ReceiverKeys::new(&[3u8; 65], &[0u8; 16]).is_err());
        assert!(ReceiverKeys::new(&[4u8; 65], &[0u8; 15]).is_err());
        let receiver = ReceiverKeys::new(&[4u8; 65], &[0u8; 16]).unwrap();
        assert!(encrypt(&receiver, &vec![b'x'; MAX_PLAINTEXT + 1]).is_err());
    }
}
