//! Shared pieces for the door tests: a real server state with the connector on, a TLS client
//! that trusts a test CA, and a plain HTTP exchange written by hand so a test controls every
//! header (a forged `Host` is the point of several).

use super::test_ca::TestCa;
use crate::WsState;
use crate::tls::acme::{AcmeHttp, AcmeSettings};
use crate::tls::gate::Gate;
use crate::tls::store::PrivateDir;
use crate::tls::{ConnectorTls, Parts};
use axum::Router;
use hq_db::mcp_connector as grants;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::crypto::aws_lc_rs;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

/// The connector's public name in tests.
pub const HOST: &str = "device.edge.example.test";
/// A private consent origin, as a tailnet name would be.
pub const CONSENT_HOST: &str = "hq.example.ts.net";
pub const WEB_TOKEN: &str = "owner-web-token";

/// A server state with the connector on and an owner web token, and the router built from it.
pub struct World {
    pub _vault: tempfile::TempDir,
    pub state: Arc<WsState>,
    pub router: Router,
}

pub fn world() -> World {
    world_with(hq_core::config::Profile::Full)
}

pub fn world_with(profile: hq_core::config::Profile) -> World {
    build_world(profile, HOST, Some(CONSENT_HOST))
}

/// A server whose connector is reached at `host`, as when the name comes from a device's id.
pub fn world_at(host: &str) -> World {
    build_world(hq_core::config::Profile::Full, host, Some(CONSENT_HOST))
}

/// A server whose consent page is on the public name, as with no `authorize_url`.
pub fn world_single_origin() -> World {
    build_world(hq_core::config::Profile::Full, HOST, Some(HOST))
}

/// A server whose connector is not configured: no OAuth routes, no door possible.
pub fn world_without_connector() -> World {
    build_world(hq_core::config::Profile::Full, HOST, None)
}

fn build_world(profile: hq_core::config::Profile, host: &str, consent: Option<&str>) -> World {
    let vault = tempfile::TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None).with_profile(profile);
    state.web_auth_token = Some(WEB_TOKEN.into());
    if let Some(consent) = consent {
        state.connector = Some(hq_core::config::ConnectorSettings {
            public: format!("https://{host}"),
            consent: format!("https://{consent}"),
            default_scope: "read_only".into(),
            gemini: false,
        });
    }
    let state = Arc::new(state);
    let router = crate::create_router(state.clone());
    World {
        _vault: vault,
        state,
        router,
    }
}

/// The check the gate uses for "is this a live grant's token", built the way `for_state` builds it.
pub fn live_grant_check(state: &Arc<WsState>) -> crate::tls::gate::GrantCheck {
    let state = state.clone();
    Arc::new(move |token: &str| {
        state
            .db
            .with_conn(|c| grants::grant_for_access_token(c, token, crate::oauth::now()))
            .ok()
            .flatten()
            .is_some()
    })
}

/// Runs the grant flow the owner's approval would, and returns the tokens.
pub fn mint_grant(state: &WsState) -> grants::TokenPair {
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    const CLIENT: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";
    const REDIRECT: &str = "https://claude.ai/api/mcp/auth_callback";
    let resource = format!("https://{HOST}/mcp");
    let now = crate::oauth::now();
    state
        .db
        .with_conn(|c| {
            grants::arm(c, now + 600)?;
            let new = grants::NewRequest {
                client_id: CLIENT,
                client_name: "Claude",
                redirect_uri: REDIRECT,
                state: None,
                code_challenge: CHALLENGE,
                resource: &resource,
                scope: "hq",
                profile: grants::Profile::Claude,
            };
            let grants::Pending::Created(id) = grants::create_request(c, &new, "binder", now)?
            else {
                anyhow::bail!("the request was not stored");
            };
            let grants::Approval::Issued { code, .. } = grants::approve_request(
                c,
                &id,
                "binder",
                grants::GrantScope::ReadOnly,
                "test grant",
                now,
            )?
            else {
                anyhow::bail!("the approval was not issued");
            };
            let exchange = grants::CodeExchange {
                code: &code,
                client_id: CLIENT,
                redirect_uri: REDIRECT,
                challenge: CHALLENGE,
                resource: Some(&resource),
            };
            grants::redeem_code(c, &exchange, now)?
                .ok_or_else(|| anyhow::anyhow!("the code was not redeemed"))
        })
        .expect("a grant for the tests")
}

/// A door on a TCP port, answering ACME-style validation as well as HTTPS, with a test CA behind it.
pub struct Door {
    pub tls: Arc<ConnectorTls>,
    pub ca: Arc<TestCa>,
    pub addr: SocketAddr,
    pub store_dir: tempfile::TempDir,
    /// The task handing accepted streams to the door; aborting it closes the port.
    pub accept: tokio::task::JoinHandle<()>,
}

/// Binds the port first, because the CA has to know where to dial before the door exists.
pub async fn door(world: &World) -> Door {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let ca = TestCa::start(addr).await;
    let store_dir = tempfile::TempDir::new().unwrap();
    let tls = Arc::new(build_tls(world, &ca, &store_dir, |_| {}));
    let accept = serve_forever(listener, tls.clone());
    Door {
        tls,
        ca,
        addr,
        store_dir,
        accept,
    }
}

pub fn build_tls(
    world: &World,
    ca: &TestCa,
    store_dir: &tempfile::TempDir,
    tweak: impl FnOnce(&mut ConnectorTls),
) -> ConnectorTls {
    build_tls_for(HOST, world, ca, store_dir, tweak)
}

pub fn build_tls_for(
    host: &str,
    world: &World,
    ca: &TestCa,
    store_dir: &tempfile::TempDir,
    tweak: impl FnOnce(&mut ConnectorTls),
) -> ConnectorTls {
    let mut tls = ConnectorTls::from_parts(Parts {
        acme: AcmeSettings {
            host: host.into(),
            directory_url: ca.directory_url.clone(),
            contact_email: Some("owner@example.com".into()),
            agreed_to_terms: true,
        },
        http: AcmeHttp::new(reqwest::Client::new()),
        store: PrivateDir::at(store_dir.path().join("connector_tls")),
        gate: Gate::new(host, live_grant_check(&world.state)),
        router: world.router.clone(),
    })
    .unwrap();
    tweak(&mut tls);
    tls
}

/// Accepts connections on `listener` and hands each to the door, as the edge bridge will.
pub fn serve_forever(listener: TcpListener, tls: Arc<ConnectorTls>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let tls = tls.clone();
            tokio::spawn(async move {
                let _ = tls.serve(stream).await;
            });
        }
    })
}

/// A TLS client config that trusts `root`, offering `alpn`.
pub fn client_config(root: &CertificateDer<'static>, alpn: &[&[u8]]) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(root.clone()).unwrap();
    let mut config = ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    Arc::new(config)
}

/// A client that accepts any certificate, as a CA validating a challenge does: what it checks is
/// inside the certificate, not who signed it.
pub fn client_config_unverified(alpn: &[&[u8]]) -> Arc<ClientConfig> {
    let mut config = ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(super::test_ca::AcceptAny))
        .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    Arc::new(config)
}

pub async fn connect(
    addr: SocketAddr,
    sni: &str,
    config: Arc<ClientConfig>,
) -> std::io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let tcp = TcpStream::connect(addr).await?;
    let name = ServerName::try_from(sni.to_string()).unwrap();
    TlsConnector::from(config).connect(name, tcp).await
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub head: String,
    pub body: String,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim())
        })
    }
}

/// Where an exchange goes and what it trusts.
pub struct Target<'a> {
    pub addr: SocketAddr,
    pub root: &'a CertificateDer<'static>,
    pub sni: &'a str,
}

/// One HTTP/1.1 exchange over TLS, written byte for byte. `host` is the `Host` header sent, which
/// is not the SNI when a test forges it.
pub async fn exchange(
    target: Target<'_>,
    method: &str,
    host: Option<&str>,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Reply {
    let Target { addr, root, sni } = target;
    let mut tls = connect(addr, sni, client_config(root, &[b"http/1.1"]))
        .await
        .expect("the TLS handshake");
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    if let Some(h) = host {
        request.push_str(&format!("Host: {h}\r\n"));
    }
    for (k, v) in headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    // A server that answers early (a refused body) may close before the whole request is sent.
    let _ = tls.write_all(request.as_bytes()).await;
    let mut raw = Vec::new();
    let _ = tls.read_to_end(&mut raw).await;
    parse(&String::from_utf8_lossy(&raw))
}

fn parse(raw: &str) -> Reply {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Reply {
        status,
        head: head.to_string(),
        body: body.to_string(),
    }
}

/// A GET to the door as Claude's servers would send it.
pub async fn get(door: &Door, path: &str, headers: &[(&str, &str)]) -> Reply {
    exchange(
        Target {
            addr: door.addr,
            root: &door.ca.root,
            sni: HOST,
        },
        "GET",
        Some(HOST),
        path,
        headers,
        "",
    )
    .await
}
