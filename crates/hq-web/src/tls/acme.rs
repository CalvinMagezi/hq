//! Getting a certificate for the connector's name from an ACME CA, answering the TLS-ALPN-01
//! challenge on the acceptor itself. The library is `instant-acme`; its HTTP client is
//! the workspace's `reqwest`, so ACME traffic uses the same TLS stack and root store as the rest
//! of HQ and tests can trust a test CA with one `add_root_certificate`.

use super::TlsError;
use super::certs::{ChallengeSlot, Issued, issued_from_der};
use super::store::{PrivateDir, Secret};
use bytes::Bytes;
use http_body_util::BodyExt;
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, BodyWrapper, BytesResponse, ChallengeType,
    HttpClient, Identifier, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

/// RFC 8555 section 6.7: what a CA says when a limit stops a request.
const RATE_LIMITED: &str = "urn:ietf:params:acme:error:rateLimited";
const ACCOUNT_UNKNOWN: &str = "urn:ietf:params:acme:error:accountDoesNotExist";
/// A CA's answer is a small JSON document or a certificate chain; anything larger is not one.
const MAX_ACME_RESPONSE: usize = 1 << 20;
const ACME_HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// The whole of one order, so a CA that stops answering cannot hold the renewal loop.
const ORDER_DEADLINE: Duration = Duration::from_secs(300);
/// How long to wait for the CA to finish validating or issuing.
const POLL_DEADLINE: Duration = Duration::from_secs(120);

/// What a CA is asked for.
#[derive(Debug, Clone)]
pub(crate) struct AcmeSettings {
    pub host: String,
    pub directory_url: String,
    pub contact_email: Option<String>,
}

/// The `reqwest` client ACME talks through, with the `Retry-After` of the latest refusal kept
/// so a rate limit can say when to try again (the library does not hand the header back).
#[derive(Clone)]
pub(crate) struct AcmeHttp {
    client: reqwest::Client,
    retry_after: Arc<Mutex<Option<Duration>>>,
}

impl AcmeHttp {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            retry_after: Arc::new(Mutex::new(None)),
        }
    }

    pub fn default_client() -> Result<reqwest::Client, TlsError> {
        reqwest::Client::builder()
            .timeout(ACME_HTTP_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| TlsError::Config(format!("the ACME HTTP client: {e}")))
    }

    fn take_retry_after(&self) -> Option<Duration> {
        self.retry_after
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

type AcmeFuture = Pin<Box<dyn Future<Output = Result<BytesResponse, instant_acme::Error>> + Send>>;

impl HttpClient for AcmeHttp {
    fn request(&self, req: http::Request<BodyWrapper<Bytes>>) -> AcmeFuture {
        let (client, retry_after) = (self.client.clone(), self.retry_after.clone());
        Box::pin(async move {
            let other = |e: Box<dyn std::error::Error + Send + Sync>| instant_acme::Error::Other(e);
            let (parts, body) = req.into_parts();
            let body = body
                .collect()
                .await
                .map_err(|e| other(Box::new(e)))?
                .to_bytes();
            let mut response = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .map_err(|e| other(Box::new(e)))?;

            let status = response.status();
            let headers = response.headers().clone();
            if !status.is_success() {
                *retry_after.lock().unwrap_or_else(|e| e.into_inner()) = headers
                    .get(http::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(Duration::from_secs);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| other(Box::new(e)))? {
                if body.len() + chunk.len() > MAX_ACME_RESPONSE {
                    return Err(other("the CA's answer is too large".into()));
                }
                body.extend_from_slice(&chunk);
            }
            let (mut parts, ()) = http::Response::new(()).into_parts();
            parts.status = status;
            parts.headers = headers;
            Ok(BytesResponse {
                parts,
                body: Box::new(Bytes::from(body)),
            })
        })
    }
}

/// An issued certificate and the file content that restores it after a restart.
pub(crate) struct Fresh {
    pub issued: Issued,
    pub bundle: Secret,
}

pub(crate) struct AcmeIssuer {
    settings: AcmeSettings,
    http: AcmeHttp,
    store: PrivateDir,
    challenge: Arc<ChallengeSlot>,
    /// The account once loaded or made, so later orders in this process use it even when it
    /// could not be saved.
    account: tokio::sync::Mutex<Option<Account>>,
}

/// Disarms the challenge when an order ends, however it ends.
struct Armed<'a>(&'a ChallengeSlot);

impl Drop for Armed<'_> {
    fn drop(&mut self) {
        self.0.disarm();
    }
}

impl AcmeIssuer {
    pub fn new(
        settings: AcmeSettings,
        http: AcmeHttp,
        store: PrivateDir,
        challenge: Arc<ChallengeSlot>,
    ) -> Self {
        Self {
            settings,
            http,
            store,
            challenge,
            account: tokio::sync::Mutex::new(None),
        }
    }

    /// One order, start to finish. The acceptor must be taking connections: the CA's
    /// validation arrives on it.
    pub async fn issue(&self) -> Result<Fresh, TlsError> {
        match tokio::time::timeout(ORDER_DEADLINE, self.order()).await {
            Ok(result) => result,
            Err(_) => Err(TlsError::Timeout("the ACME order")),
        }
    }

    async fn order(&self) -> Result<Fresh, TlsError> {
        let account = self.account().await?;
        let identifiers = [Identifier::Dns(self.settings.host.clone())];
        let mut order = account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .map_err(|e| self.failure(e))?;

        let _armed = Armed(&self.challenge);
        let mut authorizations = order.authorizations();
        while let Some(next) = authorizations.next().await {
            let mut authz = next.map_err(|e| self.failure(e))?;
            match authz.status {
                AuthorizationStatus::Pending => {}
                AuthorizationStatus::Valid => continue,
                other => {
                    return Err(TlsError::Acme(format!(
                        "the CA will not validate the name: authorization is {other:?}"
                    )));
                }
            }
            let mut challenge = authz
                .challenge(ChallengeType::TlsAlpn01)
                .ok_or_else(|| TlsError::Acme("the CA offers no tls-alpn-01 challenge".into()))?;
            self.challenge
                .arm(challenge.key_authorization().digest().as_ref())?;
            challenge.set_ready().await.map_err(|e| self.failure(e))?;
        }

        let policy = RetryPolicy::new().timeout(POLL_DEADLINE);
        let status = order
            .poll_ready(&policy)
            .await
            .map_err(|e| self.failure(e))?;
        if status != OrderStatus::Ready {
            return Err(TlsError::Acme(format!(
                "the CA did not validate the name: the order is {status:?}"
            )));
        }
        self.challenge.disarm();

        let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .map_err(|e| TlsError::Certificate(format!("certificate key: {e}")))?;
        let request = rcgen::CertificateParams::new(vec![self.settings.host.clone()])
            .and_then(|p| p.serialize_request(&pair))
            .map_err(|e| TlsError::Certificate(format!("certificate request: {e}")))?;
        order
            .finalize_csr(request.der())
            .await
            .map_err(|e| self.failure(e))?;
        let chain_pem = order
            .poll_certificate(&policy)
            .await
            .map_err(|e| self.failure(e))?;

        let chain = chain_from_pem(chain_pem.as_bytes())?;
        let issued = issued_from_der(chain, &pair.serialize_der(), &self.settings.host)?;
        let mut bundle = pair.serialize_pem().into_bytes();
        bundle.extend_from_slice(chain_pem.as_bytes());
        Ok(Fresh {
            issued,
            bundle: Secret::new(bundle),
        })
    }

    /// The account for this CA: the one in use, the stored one, or a new one made once and
    /// stored. A stored account that cannot be read is an error, never a reason to register
    /// another.
    async fn account(&self) -> Result<Account, TlsError> {
        let mut cached = self.account.lock().await;
        if let Some(account) = cached.as_ref() {
            return Ok(account.clone());
        }
        let account = self.load_or_create_account().await?;
        *cached = Some(account.clone());
        Ok(account)
    }

    async fn load_or_create_account(&self) -> Result<Account, TlsError> {
        let path = self.store.account_path(&self.settings.directory_url);
        let builder = || Account::builder_with_http(Box::new(self.http.clone()));
        if let Some(saved) = self.store.read(&path)? {
            let credentials: AccountCredentials = serde_json::from_slice(saved.expose())
                .map_err(|_| TlsError::Store("the stored ACME account does not parse".into()))?;
            return builder()
                .from_credentials(credentials)
                .await
                .map_err(|e| self.failure(e));
        }
        let contact: Vec<String> = self
            .settings
            .contact_email
            .iter()
            .map(|e| format!("mailto:{e}"))
            .collect();
        let contact: Vec<&str> = contact.iter().map(String::as_str).collect();
        let (account, credentials) = builder()
            .create(
                &NewAccount {
                    contact: &contact,
                    // `mcp_connector.tls.agree_to_ca_terms` is required to get this far.
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                self.settings.directory_url.clone(),
                None,
            )
            .await
            .map_err(|e| self.failure(e))?;
        // The account exists at the CA now; a failed save must not make the next try register
        // another, so the account stays in use for this process either way.
        let saved = serde_json::to_vec(&credentials)
            .map_err(|e| TlsError::Store(format!("the ACME account: {e}")))
            .and_then(|json| self.store.write(&path, &json));
        if let Err(e) = saved {
            tracing::warn!("connector tls: the ACME account was not saved: {e}");
        }
        Ok(account)
    }

    /// A library error as ours: a CA's rate limit keeps its `Retry-After`.
    fn failure(&self, error: instant_acme::Error) -> TlsError {
        match &error {
            instant_acme::Error::Api(problem)
                if problem.r#type.as_deref() == Some(RATE_LIMITED)
                    || problem.status == Some(429) =>
            {
                TlsError::RateLimited {
                    retry_after: self.http.take_retry_after(),
                    detail: problem.detail.clone().unwrap_or_default(),
                }
            }
            instant_acme::Error::Api(problem)
                if problem.r#type.as_deref() == Some(ACCOUNT_UNKNOWN) =>
            {
                TlsError::Acme(
                    "the CA does not know the saved account (staging CAs are reset now and then); \
                     delete account-*.json in the connector's TLS folder to register again"
                        .into(),
                )
            }
            instant_acme::Error::Api(problem) => TlsError::Acme(format!(
                "the CA refused: {} {}",
                problem.r#type.as_deref().unwrap_or("error"),
                problem.detail.as_deref().unwrap_or_default()
            )),
            other => TlsError::Acme(other.to_string()),
        }
    }
}

/// The certificates in a PEM chain, leaf first.
pub(crate) fn chain_from_pem(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::Certificate(format!("the chain does not parse: {e}")))
}

/// The issued certificate restored from the file `issue` produced.
pub(crate) fn issued_from_bundle(bundle: &Secret, host: &str) -> Result<Issued, TlsError> {
    let key = PrivatePkcs8KeyDer::from_pem_slice(bundle.expose())
        .map_err(|e| TlsError::Certificate(format!("the stored key does not parse: {e}")))?;
    let chain = chain_from_pem(bundle.expose())?;
    issued_from_der(chain, key.secret_pkcs8_der(), host)
}
