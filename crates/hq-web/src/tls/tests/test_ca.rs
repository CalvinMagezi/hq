//! An ACME server for tests, faithful where it matters to this client: it checks every
//! request's JWS signature, nonce and URL, validates TLS-ALPN-01 by dialing the acceptor and
//! reading the `acmeIdentifier` extension, and signs the client's CSR with a CA whose root the
//! test client trusts. It is not Pebble (that run is `pebble.rs`, `#[ignore]`d); it exists so the
//! whole order runs in `cargo test` with no Docker.

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{WebPkiSupportedAlgorithms, aws_lc_rs};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};

const ACME_ID_OID: &str = "1.3.6.1.5.5.7.1.31";
const ISSUED_DAYS: i64 = 90;

/// What a test can make the CA do.
#[derive(Default)]
struct Knobs {
    /// Answer the next `newOrder` with a rate limit and this `Retry-After` (seconds).
    rate_limit: Option<Option<u64>>,
    /// Fail every validation, as a CA that cannot reach the name.
    reject_validation: bool,
    /// Issue certificates that are nearly over, as a clock far ahead of the CA's makes them look.
    short_lived: bool,
}

/// What the CA saw, for assertions.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    pub accounts: usize,
    pub orders: usize,
    pub validations_ok: usize,
    pub validations_failed: usize,
    pub certificates: usize,
    pub contacts: Vec<String>,
}

struct Account {
    key: Vec<u8>,
    thumbprint: String,
}

struct Authz {
    domain: String,
    token: String,
    status: &'static str,
    account: String,
}

struct Order {
    domain: String,
    status: &'static str,
    authz: String,
    certificate: Option<String>,
}

struct Ca {
    nonces: Vec<String>,
    accounts: HashMap<String, Account>,
    authzs: HashMap<String, Authz>,
    orders: HashMap<String, Order>,
    certs: HashMap<String, String>,
    knobs: Knobs,
    stats: Stats,
    next_id: u32,
}

pub struct TestCa {
    pub directory_url: String,
    base: String,
    validate_at: SocketAddr,
    issuer_pem: String,
    issuer_key: Arc<rcgen::KeyPair>,
    issuer_params: rcgen::CertificateParams,
    pub root: CertificateDer<'static>,
    state: Mutex<Ca>,
}

impl TestCa {
    /// Starts the CA on a loopback port. It dials `validate_at` (the acceptor) to check
    /// challenges.
    pub async fn start(validate_at: SocketAddr) -> Arc<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());

        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::CrlSign,
        ];
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "HQ Test CA");
        let root = params.self_signed(&key).unwrap();

        let ca = Arc::new(Self {
            directory_url: format!("{base}/dir"),
            base,
            validate_at,
            issuer_pem: root.pem(),
            root: root.der().clone(),
            issuer_key: Arc::new(key),
            issuer_params: params,
            state: Mutex::new(Ca {
                nonces: Vec::new(),
                accounts: HashMap::new(),
                authzs: HashMap::new(),
                orders: HashMap::new(),
                certs: HashMap::new(),
                knobs: Knobs::default(),
                stats: Stats::default(),
                next_id: 0,
            }),
        });
        let app = {
            let ca = ca.clone();
            axum::Router::new().fallback(move |method: Method, uri: Uri, body: Bytes| {
                let ca = ca.clone();
                async move { ca.handle(method, uri, body).await }
            })
        };
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        ca
    }

    pub fn stats(&self) -> Stats {
        self.lock().stats.clone()
    }

    /// The next `newOrder` is refused as rate limited.
    pub fn rate_limit_next_order(&self, retry_after_secs: Option<u64>) {
        self.lock().knobs.rate_limit = Some(retry_after_secs);
    }

    pub fn issue_short_lived(&self, on: bool) {
        self.lock().knobs.short_lived = on;
    }

    pub fn reject_validation(&self, on: bool) {
        self.lock().knobs.reject_validation = on;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Ca> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn handle(self: Arc<Self>, method: Method, uri: Uri, body: Bytes) -> Response {
        let path = uri.path().to_string();
        let url = format!("{}{}", self.base, path);
        let mut response = match (method.as_str(), path.as_str()) {
            ("GET", "/dir") => json_ok(self.directory()),
            ("HEAD", "/nonce") => StatusCode::OK.into_response(),
            ("GET", "/nonce") => StatusCode::NO_CONTENT.into_response(),
            ("POST", _) => self.post(&path, &url, &body).await,
            _ => problem(StatusCode::NOT_FOUND, "malformed", "no such resource", None),
        };
        let nonce = self.fresh_nonce();
        response
            .headers_mut()
            .insert("replay-nonce", HeaderValue::from_str(&nonce).unwrap());
        response
    }

    fn directory(&self) -> Value {
        json!({
            "newNonce": format!("{}/nonce", self.base),
            "newAccount": format!("{}/new-acct", self.base),
            "newOrder": format!("{}/new-order", self.base),
            "revokeCert": format!("{}/revoke", self.base),
            "keyChange": format!("{}/key-change", self.base),
        })
    }

    fn fresh_nonce(&self) -> String {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        self.lock().nonces.push(nonce.clone());
        nonce
    }

    async fn post(self: &Arc<Self>, path: &str, url: &str, body: &Bytes) -> Response {
        let jws = match self.verify(url, body) {
            Ok(j) => j,
            Err(r) => return r,
        };
        let (prefix, id) = path.rsplit_once('/').unwrap_or(("", path));
        match prefix {
            "" if id == "new-acct" => self.new_account(&jws),
            "" if id == "new-order" => self.new_order(&jws),
            "/authz" => self.authz(id),
            "/chall" => self.challenge(id).await,
            "/order" => self.order(id),
            "/finalize" => self.finalize(id, &jws),
            "/cert" => self.certificate(id),
            _ => problem(StatusCode::NOT_FOUND, "malformed", "no such resource", None),
        }
    }

    /// Checks the request is a well-formed, correctly signed, single-use-nonce JWS for this URL.
    #[allow(clippy::result_large_err)] // the error is the response to send; this is a test double
    fn verify(&self, url: &str, body: &Bytes) -> Result<Jws, Response> {
        let bad = |detail: &str| problem(StatusCode::BAD_REQUEST, "malformed", detail, None);
        let flat: Value = serde_json::from_slice(body).map_err(|_| bad("not JSON"))?;
        let part = |k: &str| flat[k].as_str().ok_or_else(|| bad("missing JWS field"));
        let (protected_b64, payload_b64, sig_b64) =
            (part("protected")?, part("payload")?, part("signature")?);
        let protected: Value = serde_json::from_slice(
            &B64.decode(protected_b64)
                .map_err(|_| bad("protected is not base64url"))?,
        )
        .map_err(|_| bad("protected is not JSON"))?;
        if protected["alg"] != "ES256" || protected["url"] != url {
            return Err(bad("wrong alg or url"));
        }
        let nonce = protected["nonce"].as_str().unwrap_or_default();
        {
            let mut ca = self.lock();
            let Some(at) = ca.nonces.iter().position(|n| n == nonce) else {
                return Err(problem(
                    StatusCode::BAD_REQUEST,
                    "badNonce",
                    "unknown or reused nonce",
                    None,
                ));
            };
            ca.nonces.remove(at);
        }
        let (key, account) = match (protected.get("jwk"), protected["kid"].as_str()) {
            (Some(jwk), None) => (jwk_to_point(jwk).ok_or_else(|| bad("bad jwk"))?, None),
            (None, Some(kid)) => {
                let ca = self.lock();
                let a = ca.accounts.get(kid).ok_or_else(|| bad("unknown account"))?;
                (a.key.clone(), Some(kid.to_string()))
            }
            _ => return Err(bad("needs exactly one of jwk and kid")),
        };
        let signature = B64
            .decode(sig_b64)
            .map_err(|_| bad("signature not base64url"))?;
        let signed = format!("{protected_b64}.{payload_b64}");
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &key)
            .verify(signed.as_bytes(), &signature)
            .map_err(|_| {
                problem(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "bad signature",
                    None,
                )
            })?;
        let payload = if payload_b64.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&B64.decode(payload_b64).map_err(|_| bad("payload"))?)
                .map_err(|_| bad("payload is not JSON"))?
        };
        Ok(Jws {
            payload,
            account,
            jwk: protected.get("jwk").cloned(),
        })
    }

    fn new_account(&self, jws: &Jws) -> Response {
        let jwk = jws.jwk.clone().unwrap();
        let mut ca = self.lock();
        ca.next_id += 1;
        let kid = format!("{}/acct/{}", self.base, ca.next_id);
        ca.accounts.insert(
            kid.clone(),
            Account {
                key: jwk_to_point(&jwk).unwrap(),
                thumbprint: thumbprint(&jwk),
            },
        );
        ca.stats.accounts += 1;
        let contacts = jws.payload["contact"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        ca.stats.contacts.extend(
            contacts
                .iter()
                .filter_map(|c| c.as_str().map(str::to_string)),
        );
        let mut res = (StatusCode::CREATED, axum::Json(json!({"status": "valid"}))).into_response();
        res.headers_mut()
            .insert(header::LOCATION, HeaderValue::from_str(&kid).unwrap());
        res
    }

    fn new_order(&self, jws: &Jws) -> Response {
        let mut ca = self.lock();
        if let Some(retry) = ca.knobs.rate_limit.take() {
            return problem(
                StatusCode::TOO_MANY_REQUESTS,
                "rateLimited",
                "too many certificates (5) already issued for this exact set of identifiers in the last 168h0m0s, retry after 2026-10-29 10:00:00 UTC",
                retry,
            );
        }
        let domain = jws.payload["identifiers"][0]["value"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        ca.next_id += 1;
        let (order_id, authz_id) = (format!("o{}", ca.next_id), format!("a{}", ca.next_id));
        ca.authzs.insert(
            authz_id.clone(),
            Authz {
                domain: domain.clone(),
                token: B64.encode(Sha256::digest(authz_id.as_bytes())),
                status: "pending",
                account: jws.account.clone().unwrap_or_default(),
            },
        );
        ca.orders.insert(
            order_id.clone(),
            Order {
                domain,
                status: "pending",
                authz: authz_id,
                certificate: None,
            },
        );
        ca.stats.orders += 1;
        let body = order_json(&self.base, &order_id, &ca.orders[&order_id]);
        let mut res = (StatusCode::CREATED, axum::Json(body)).into_response();
        res.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_str(&format!("{}/order/{order_id}", self.base)).unwrap(),
        );
        res
    }

    fn authz(&self, id: &str) -> Response {
        let ca = self.lock();
        let Some(a) = ca.authzs.get(id) else {
            return problem(
                StatusCode::NOT_FOUND,
                "malformed",
                "no such authorization",
                None,
            );
        };
        json_ok(json!({
            "status": a.status,
            "identifier": {"type": "dns", "value": a.domain},
            "challenges": [
                {"type": "http-01", "url": format!("{}/chall/h-{id}", self.base), "token": a.token, "status": "pending"},
                {"type": "tls-alpn-01", "url": format!("{}/chall/{id}", self.base), "token": a.token, "status": challenge_status(a.status)},
            ],
        }))
    }

    async fn challenge(self: &Arc<Self>, id: &str) -> Response {
        let (domain, token, thumb, reject) = {
            let ca = self.lock();
            let Some(a) = ca.authzs.get(id) else {
                return problem(
                    StatusCode::NOT_FOUND,
                    "malformed",
                    "no such challenge",
                    None,
                );
            };
            let thumb = ca
                .accounts
                .get(&a.account)
                .map(|acc| acc.thumbprint.clone());
            (
                a.domain.clone(),
                a.token.clone(),
                thumb,
                ca.knobs.reject_validation,
            )
        };
        let Some(thumb) = thumb else {
            return problem(StatusCode::BAD_REQUEST, "malformed", "no account", None);
        };
        let (ca, id_owned, key_authorization) =
            (self.clone(), id.to_string(), format!("{token}.{thumb}"));
        tokio::spawn(async move {
            let ok = !reject && ca.validate(&domain, &key_authorization).await;
            ca.settle(&id_owned, ok);
        });
        json_ok(json!({
            "type": "tls-alpn-01",
            "url": format!("{}/chall/{id}", self.base),
            "token": token,
            "status": "processing",
        }))
    }

    /// RFC 8737 section 3: dial the name's port 443 (here, the acceptor) offering only
    /// `acme-tls/1`, and require a certificate for exactly the name with the critical
    /// `acmeIdentifier` extension holding the key authorization's digest.
    async fn validate(&self, domain: &str, key_authorization: &str) -> bool {
        let provider = Arc::new(aws_lc_rs::default_provider());
        let Ok(mut config) = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map(|b| {
                b.dangerous()
                    .with_custom_certificate_verifier(Arc::new(AcceptAny))
                    .with_no_client_auth()
            })
        else {
            return false;
        };
        config.alpn_protocols = vec![b"acme-tls/1".to_vec()];
        let Ok(tcp) = tokio::net::TcpStream::connect(self.validate_at).await else {
            return false;
        };
        let Ok(name) = ServerName::try_from(domain.to_string()) else {
            return false;
        };
        let Ok(tls) = TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
        else {
            return false;
        };
        let (_, session) = tls.get_ref();
        if session.alpn_protocol() != Some(b"acme-tls/1") {
            return false;
        }
        let Some(leaf) = session.peer_certificates().and_then(|c| c.first()) else {
            return false;
        };
        let Ok((_, cert)) = x509_parser::parse_x509_certificate(leaf.as_ref()) else {
            return false;
        };
        let names: Vec<String> = cert
            .subject_alternative_name()
            .ok()
            .flatten()
            .map(|san| {
                san.value
                    .general_names
                    .iter()
                    .filter_map(|n| match n {
                        x509_parser::extensions::GeneralName::DNSName(d) => Some(d.to_string()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let digest = Sha256::digest(key_authorization.as_bytes());
        let id_ext = cert
            .extensions()
            .iter()
            .find(|e| e.oid.to_id_string() == ACME_ID_OID);
        names == [domain.to_string()]
            && id_ext
                .is_some_and(|e| e.critical && e.value.ends_with(&digest) && e.value.len() == 34)
    }

    fn settle(&self, authz_id: &str, ok: bool) {
        let mut ca = self.lock();
        if ok {
            ca.stats.validations_ok += 1;
        } else {
            ca.stats.validations_failed += 1;
        }
        let status = if ok { "valid" } else { "invalid" };
        if let Some(a) = ca.authzs.get_mut(authz_id) {
            a.status = status;
        }
        for o in ca.orders.values_mut().filter(|o| o.authz == authz_id) {
            o.status = if ok { "ready" } else { "invalid" };
        }
    }

    fn order(&self, id: &str) -> Response {
        let ca = self.lock();
        match ca.orders.get(id) {
            Some(o) => json_ok(order_json(&self.base, id, o)),
            None => problem(StatusCode::NOT_FOUND, "malformed", "no such order", None),
        }
    }

    fn finalize(&self, id: &str, jws: &Jws) -> Response {
        let bad = |d: &str| problem(StatusCode::FORBIDDEN, "orderNotReady", d, None);
        let mut ca = self.lock();
        let Some(order) = ca.orders.get(id) else {
            return problem(StatusCode::NOT_FOUND, "malformed", "no such order", None);
        };
        if order.status != "ready" {
            return bad("the order is not ready");
        }
        let domain = order.domain.clone();
        let Some(csr_der) = jws.payload["csr"].as_str().and_then(|c| B64.decode(c).ok()) else {
            return problem(StatusCode::BAD_REQUEST, "badCSR", "no csr", None);
        };
        let Ok(mut csr) = rcgen::CertificateSigningRequestParams::from_der(&csr_der.clone().into())
        else {
            return problem(
                StatusCode::BAD_REQUEST,
                "badCSR",
                "csr does not parse",
                None,
            );
        };
        let names: Vec<String> = csr
            .params
            .subject_alt_names
            .iter()
            .map(|n| format!("{n:?}"))
            .collect();
        if names.len() != 1 || !names[0].contains(&domain) {
            return problem(
                StatusCode::BAD_REQUEST,
                "badCSR",
                "csr names the wrong identifiers",
                None,
            );
        }
        csr.params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        csr.params.not_before = time_now(-1);
        csr.params.not_after = if ca.knobs.short_lived {
            time_now(0) + time::Duration::minutes(1)
        } else {
            time_now(ISSUED_DAYS * 24)
        };
        let issuer = rcgen::Issuer::from_params(&self.issuer_params, self.issuer_key.as_ref());
        let leaf = csr.signed_by(&issuer).unwrap();
        let chain = format!("{}{}", leaf.pem(), self.issuer_pem);
        ca.certs.insert(id.to_string(), chain);
        ca.stats.certificates += 1;
        let order = ca.orders.get_mut(id).unwrap();
        order.status = "valid";
        order.certificate = Some(format!("{}/cert/{id}", self.base));
        json_ok(order_json(&self.base, id, &ca.orders[id]))
    }

    fn certificate(&self, id: &str) -> Response {
        let ca = self.lock();
        match ca.certs.get(id) {
            Some(pem) => (
                [(header::CONTENT_TYPE, "application/pem-certificate-chain")],
                pem.clone(),
            )
                .into_response(),
            None => problem(
                StatusCode::NOT_FOUND,
                "malformed",
                "no such certificate",
                None,
            ),
        }
    }
}

struct Jws {
    payload: Value,
    account: Option<String>,
    jwk: Option<Value>,
}

fn challenge_status(authz: &str) -> &'static str {
    match authz {
        "valid" => "valid",
        "invalid" => "invalid",
        _ => "pending",
    }
}

fn order_json(base: &str, id: &str, o: &Order) -> Value {
    let mut v = json!({
        "status": o.status,
        "identifiers": [{"type": "dns", "value": o.domain}],
        "authorizations": [format!("{base}/authz/{}", o.authz)],
        "finalize": format!("{base}/finalize/{id}"),
    });
    if let Some(c) = &o.certificate {
        v["certificate"] = json!(c);
    }
    v
}

fn json_ok(v: Value) -> Response {
    axum::Json(v).into_response()
}

fn problem(status: StatusCode, kind: &str, detail: &str, retry_after: Option<u64>) -> Response {
    let mut headers = HeaderMap::new();
    if let Some(s) = retry_after {
        headers.insert(header::RETRY_AFTER, HeaderValue::from(s));
    }
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    (
        status,
        headers,
        json!({"type": format!("urn:ietf:params:acme:error:{kind}"), "detail": detail, "status": status.as_u16()})
            .to_string(),
    )
        .into_response()
}

/// The uncompressed P-256 point a JWK holds.
fn jwk_to_point(jwk: &Value) -> Option<Vec<u8>> {
    if jwk["kty"] != "EC" || jwk["crv"] != "P-256" {
        return None;
    }
    let mut point = vec![0x04];
    point.extend(B64.decode(jwk["x"].as_str()?).ok()?);
    point.extend(B64.decode(jwk["y"].as_str()?).ok()?);
    (point.len() == 65).then_some(point)
}

/// RFC 7638: the SHA-256 of the JWK's required members in lexicographic order.
fn thumbprint(jwk: &Value) -> String {
    let canonical = format!(
        r#"{{"crv":"P-256","kty":"EC","x":{},"y":{}}}"#,
        jwk["x"], jwk["y"]
    );
    B64.encode(Sha256::digest(canonical.as_bytes()))
}

fn time_now(offset_hours: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc() + time::Duration::hours(offset_hours)
}

/// The CA validating a challenge accepts the self-signed certificate and checks its content.
#[derive(Debug)]
pub(crate) struct AcceptAny;

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    // The handshake signature is not checked: parsing the certificate to do it would refuse the
    // challenge certificate's critical `acmeIdentifier` extension, as every general-purpose
    // verifier does. A CA validating a challenge reads the certificate itself.
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        algs().supported_schemes()
    }
}

fn algs() -> WebPkiSupportedAlgorithms {
    aws_lc_rs::default_provider().signature_verification_algorithms
}
