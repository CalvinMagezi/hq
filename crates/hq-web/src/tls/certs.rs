//! The certificates the acceptor answers with: the issued one, which can be swapped while
//! connections are open, and the short-lived one that proves a TLS-ALPN-01 challenge.

use super::TlsError;
use std::sync::{Arc, Mutex, RwLock};
use tokio_rustls::rustls::crypto::aws_lc_rs;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;

/// An issued certificate chain and when it is good for, in unix seconds.
pub(crate) struct Issued {
    pub key: Arc<CertifiedKey>,
    pub not_before: i64,
    pub not_after: i64,
}

/// Builds the rustls key for a PKCS#8 private key and a leaf-first chain, refusing a chain
/// whose leaf is not for that key. Returns the leaf's validity too.
pub(crate) fn issued_from_der(
    chain: Vec<CertificateDer<'static>>,
    pkcs8: &[u8],
) -> Result<Issued, TlsError> {
    let leaf = chain
        .first()
        .ok_or_else(|| TlsError::Certificate("the chain is empty".into()))?;
    let (not_before, not_after) = validity(leaf)?;
    let signing = aws_lc_rs::sign::any_supported_type(&PrivateKeyDer::Pkcs8(
        PrivatePkcs8KeyDer::from(pkcs8.to_vec()),
    ))
    .map_err(|e| TlsError::Certificate(format!("the private key is not usable: {e}")))?;
    let key = CertifiedKey::new(chain, signing);
    key.keys_match()
        .map_err(|_| TlsError::Certificate("the certificate is not for this private key".into()))?;
    Ok(Issued {
        key: Arc::new(key),
        not_before,
        not_after,
    })
}

/// A certificate's `notBefore` and `notAfter`, unix seconds.
pub(crate) fn validity(cert: &CertificateDer<'_>) -> Result<(i64, i64), TlsError> {
    let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
        .map_err(|e| TlsError::Certificate(format!("the certificate does not parse: {e}")))?;
    let v = parsed.validity();
    Ok((v.not_before.timestamp(), v.not_after.timestamp()))
}

/// The certificate served for the connector's name. It starts empty, and a handshake for a
/// name that is not ours, or before a certificate exists, is refused.
pub(crate) struct LiveCert {
    host: String,
    current: RwLock<Option<Arc<CertifiedKey>>>,
}

impl LiveCert {
    pub fn new(host: &str) -> Self {
        Self {
            host: host.to_ascii_lowercase(),
            current: RwLock::new(None),
        }
    }

    pub fn install(&self, key: Arc<CertifiedKey>) {
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Some(key);
    }

    pub fn is_installed(&self) -> bool {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

impl std::fmt::Debug for LiveCert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveCert")
            .field("host", &self.host)
            .field("installed", &self.is_installed())
            .finish()
    }
}

impl ResolvesServerCert for LiveCert {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if !sni_is(hello.server_name(), &self.host) {
            return None;
        }
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// The challenge certificate an ACME order is waiting on, if one is.
pub(crate) struct ChallengeSlot {
    host: String,
    pending: Mutex<Option<Arc<CertifiedKey>>>,
}

impl ChallengeSlot {
    pub fn new(host: &str) -> Self {
        Self {
            host: host.to_ascii_lowercase(),
            pending: Mutex::new(None),
        }
    }

    /// Starts answering `acme-tls/1` with a certificate that carries `digest`, the SHA-256 of
    /// the challenge's key authorization.
    pub fn arm(&self, digest: &[u8]) -> Result<(), TlsError> {
        let key = challenge_cert(&self.host, digest)?;
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(key));
        Ok(())
    }

    pub fn disarm(&self) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn is_armed(&self) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

impl std::fmt::Debug for ChallengeSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChallengeSlot")
            .field("host", &self.host)
            .field("armed", &self.is_armed())
            .finish()
    }
}

impl ResolvesServerCert for ChallengeSlot {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if !sni_is(hello.server_name(), &self.host) {
            return None;
        }
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Whether the TLS server name is exactly `host`, ignoring case.
pub(crate) fn sni_is(sni: Option<&str>, host: &str) -> bool {
    sni.is_some_and(|name| name.eq_ignore_ascii_case(host))
}

/// RFC 8737 section 3: a self-signed certificate for the name, whose critical `acmeIdentifier`
/// extension holds the key authorization's digest.
fn challenge_cert(host: &str, digest: &[u8]) -> Result<CertifiedKey, TlsError> {
    let digest: &[u8; 32] = digest
        .try_into()
        .map_err(|_| TlsError::Certificate("a key authorization digest is 32 bytes".into()))?;
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| TlsError::Certificate(format!("challenge key: {e}")))?;
    let mut params = rcgen::CertificateParams::new(vec![host.to_string()])
        .map_err(|e| TlsError::Certificate(format!("challenge certificate: {e}")))?;
    params
        .custom_extensions
        .push(rcgen::CustomExtension::new_acme_identifier(digest));
    let cert = params
        .self_signed(&pair)
        .map_err(|e| TlsError::Certificate(format!("challenge certificate: {e}")))?;
    let signing = aws_lc_rs::sign::any_supported_type(&PrivateKeyDer::Pkcs8(
        PrivatePkcs8KeyDer::from(pair.serialize_der()),
    ))
    .map_err(|e| TlsError::Certificate(format!("challenge key: {e}")))?;
    Ok(CertifiedKey::new(vec![cert.der().clone()], signing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_challenge_certificate_names_the_host_and_carries_the_digest_as_a_critical_extension() {
        let digest = [7u8; 32];
        let key = challenge_cert("hq.example.com", &digest).unwrap();
        let der = key.cert.first().unwrap();
        let (_, parsed) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
        let san = parsed.subject_alternative_name().unwrap().unwrap();
        assert!(san.value.general_names.iter().any(|n| matches!(
            n,
            x509_parser::extensions::GeneralName::DNSName("hq.example.com")
        )));
        let acme = parsed
            .extensions()
            .iter()
            .find(|e| e.oid.to_id_string() == "1.3.6.1.5.5.7.1.31")
            .expect("the acmeIdentifier extension");
        assert!(acme.critical);
        assert!(
            acme.value.ends_with(&digest),
            "the extension is an OCTET STRING of the digest"
        );
    }

    #[test]
    fn a_digest_of_the_wrong_length_is_refused() {
        assert!(challenge_cert("hq.example.com", &[0u8; 31]).is_err());
    }

    #[test]
    fn a_chain_for_another_key_is_refused() {
        let a = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let b = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let cert = rcgen::CertificateParams::new(vec!["hq.example.com".to_string()])
            .unwrap()
            .self_signed(&a)
            .unwrap();
        let chain = vec![cert.der().clone()];
        assert!(issued_from_der(chain.clone(), &a.serialize_der()).is_ok());
        let err = issued_from_der(chain, &b.serialize_der()).err().unwrap();
        assert!(err.to_string().contains("not for this private key"));
    }

    #[test]
    fn the_validity_window_comes_from_the_certificate() {
        let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["hq.example.com".to_string()]).unwrap();
        params.not_before = rcgen::date_time_ymd(2030, 1, 1);
        params.not_after = rcgen::date_time_ymd(2030, 4, 1);
        let cert = params.self_signed(&pair).unwrap();
        let (from, to) = validity(cert.der()).unwrap();
        assert_eq!(to - from, 90 * 24 * 3600);
    }

    #[test]
    fn the_server_name_must_match_exactly_but_not_in_case() {
        assert!(sni_is(Some("HQ.Example.com"), "hq.example.com"));
        assert!(!sni_is(Some("hq.example.com."), "hq.example.com"));
        assert!(!sni_is(Some("evil.hq.example.com"), "hq.example.com"));
        assert!(!sni_is(None, "hq.example.com"));
    }
}
