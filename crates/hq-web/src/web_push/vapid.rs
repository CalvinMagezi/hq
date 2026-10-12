//! The server's VAPID identity (RFC 8292): one P-256 key, created on first use, that signs a
//! short-lived JWT on every push so the push service knows the sender. Browsers bind each
//! subscription to the public half, so replacing the key orphans every subscription.

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use std::path::{Path, PathBuf};

/// Where the key lives: next to the database, outside anything the vault syncs as notes.
pub(crate) fn key_path(vault_path: &Path) -> PathBuf {
    vault_path.join("_data").join("web_push_vapid.p8")
}

/// How long a signed token is good for. RFC 8292 caps it at 24 hours.
const TOKEN_LIFETIME_SECS: i64 = 12 * 60 * 60;

pub(crate) struct VapidKey {
    pair: EcdsaKeyPair,
    rng: SystemRandom,
}

impl VapidKey {
    /// Loads the key at `path`, or creates it (owner-only permissions) when there is none.
    pub(crate) fn load_or_create(path: &Path) -> Result<Self> {
        let rng = SystemRandom::new();
        let pkcs8 = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
                    .map_err(|_| anyhow!("could not generate a VAPID key"))?;
                write_private(path, doc.as_ref())?;
                tracing::info!(path = %path.display(), "web push: created a VAPID key");
                doc.as_ref().to_vec()
            }
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &rng)
            .map_err(|_| anyhow!("{} is not a P-256 PKCS#8 key", path.display()))?;
        Ok(Self { pair, rng })
    }

    /// The public key as the browser's `applicationServerKey` wants it: the uncompressed
    /// point, base64url without padding.
    pub(crate) fn public_key_b64(&self) -> String {
        B64.encode(self.pair.public_key().as_ref())
    }

    /// The `Authorization` header value for a push to `endpoint`.
    pub(crate) fn authorization(&self, endpoint: &reqwest::Url, subject: &str) -> Result<String> {
        let exp = chrono::Utc::now().timestamp() + TOKEN_LIFETIME_SECS;
        let token = self.token(&audience(endpoint)?, subject, exp)?;
        Ok(format!("vapid t={token}, k={}", self.public_key_b64()))
    }

    fn token(&self, aud: &str, subject: &str, exp: i64) -> Result<String> {
        let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = B64.encode(serde_json::to_vec(&serde_json::json!({
            "aud": aud,
            "exp": exp,
            "sub": subject,
        }))?);
        let signing_input = format!("{header}.{claims}");
        let sig = self
            .pair
            .sign(&self.rng, signing_input.as_bytes())
            .map_err(|_| anyhow!("could not sign the VAPID token"))?;
        Ok(format!("{signing_input}.{}", B64.encode(sig.as_ref())))
    }
}

/// The JWT audience: the push service's origin.
fn audience(endpoint: &reqwest::Url) -> Result<String> {
    let origin = endpoint.origin();
    if !origin.is_tuple() {
        return Err(anyhow!("push endpoint {endpoint} has no origin"));
    }
    Ok(origin.ascii_serialization())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    std::io::Write::write_all(&mut f, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

    #[test]
    fn the_key_is_created_once_and_reloaded() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = key_path(dir.path());
        let first = VapidKey::load_or_create(&path).unwrap();
        let again = VapidKey::load_or_create(&path).unwrap();
        assert_eq!(first.public_key_b64(), again.public_key_b64());
        assert_eq!(B64.decode(first.public_key_b64()).unwrap().len(), 65);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the private key is owner-only");
        }
    }

    #[test]
    fn the_token_names_the_push_origin_and_verifies_with_the_public_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let key = VapidKey::load_or_create(&key_path(dir.path())).unwrap();
        let endpoint = reqwest::Url::parse("https://fcm.googleapis.com/fcm/send/abc").unwrap();
        let header = key
            .authorization(&endpoint, "mailto:admin@example.com")
            .unwrap();
        let (token, k) = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split_once(", k=")
            .unwrap();
        assert_eq!(k, key.public_key_b64());

        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: serde_json::Value =
            serde_json::from_slice(&B64.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["sub"], "mailto:admin@example.com");
        let exp = claims["exp"].as_i64().unwrap();
        let now = chrono::Utc::now().timestamp();
        assert!(exp > now && exp <= now + 24 * 60 * 60);

        let public = B64.decode(k).unwrap();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public)
            .verify(
                format!("{}.{}", parts[0], parts[1]).as_bytes(),
                &B64.decode(parts[2]).unwrap(),
            )
            .expect("ES256 signature verifies");
    }

    #[test]
    fn an_unreadable_key_file_is_an_error_not_a_new_key() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = key_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not a key").unwrap();
        assert!(VapidKey::load_or_create(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not a key", "left alone");
    }
}
