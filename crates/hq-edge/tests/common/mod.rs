//! A running edge, a device behind a simulated NAT and a TLS client, all on loopback.
//!
//! The device has no IP transport at all, so the only way to reach it is through the relay: that
//! is a NAT that never lets a direct path through, the hardest case the edge has to serve.

#![allow(dead_code)]

use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use hq_edge::{
    AllowlistFile, EDGE_ALPN, Edge, EdgeConfig, Registered, RelayHost, RelayParams, RelayTls,
    allow_device, device_label, load_or_create_key,
};
use iroh::{Endpoint, EndpointId, RelayMode, RelayUrl, SecretKey, endpoint::presets};
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::{TlsAcceptor, TlsConnector, rustls};

pub const DOMAIN: &str = "edge.test";
pub const RELAY_NAME: &str = "relay.edge.test";
pub const BODY: &str = "hello from the device behind the NAT";
const STEP: Duration = Duration::from_millis(50);
const SETUP_LIMIT: Duration = Duration::from_secs(20);

pub fn ring() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A TLS server config for `name` and a client config that trusts it, both offering `alpn`.
pub fn tls_pair(name: &str, alpn: &[&[u8]]) -> (TlsAcceptor, TlsConnector) {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca_params, ca_key);
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec![name.to_string()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();

    let protocols: Vec<Vec<u8>> = alpn.iter().map(|a| a.to_vec()).collect();
    let mut server = rustls::ServerConfig::builder_with_provider(ring())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .unwrap();
    server.alpn_protocols = protocols.clone();

    let mut roots = rustls::RootCertStore::empty();
    roots.add(ca_cert.der().clone()).unwrap();
    let mut client = rustls::ClientConfig::builder_with_provider(ring())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client.alpn_protocols = protocols;
    (
        TlsAcceptor::from(Arc::new(server)),
        TlsConnector::from(Arc::new(client)),
    )
}

/// The first flight of a TLS client for `name`: a real ClientHello, nothing more.
pub fn client_hello(name: &str) -> Vec<u8> {
    let config = rustls::ClientConfig::builder_with_provider(ring())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(name.to_owned()).unwrap();
    let mut conn = rustls::ClientConnection::new(Arc::new(config), name).unwrap();
    let mut out = Vec::new();
    conn.write_tls(&mut out).unwrap();
    out
}

pub fn name_of(id: &EndpointId) -> String {
    format!("{}.{DOMAIN}", device_label(id))
}

/// What the device does with a stream the edge opens.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    /// Finish TLS with the client and answer one HTTP request.
    Http,
    /// Accept the stream and hold it without answering.
    Hold,
    /// Read everything, counting bytes.
    Sink,
}

pub struct Device {
    pub key: SecretKey,
    pub endpoint: Endpoint,
    /// Streams the device has accepted from the edge.
    pub streams: Arc<AtomicU64>,
    /// Bytes read from streams in `Sink` mode.
    pub sunk: Arc<AtomicU64>,
    /// The ALPN the client negotiated, as the device saw it.
    pub alpn_seen: Arc<Mutex<Option<Vec<u8>>>>,
    /// Connections from anyone but the edge that the device turned away.
    pub strangers: Arc<AtomicU64>,
}

impl Device {
    pub fn id(&self) -> EndpointId {
        self.key.public()
    }

    pub fn name(&self) -> String {
        name_of(&self.id())
    }

    pub fn streams(&self) -> u64 {
        self.streams.load(Ordering::SeqCst)
    }

    pub fn sunk(&self) -> u64 {
        self.sunk.load(Ordering::SeqCst)
    }
}

/// Starts the device: relay-only (no IP transport), serving the edge protocol to the edge alone.
pub async fn start_device(
    key: SecretKey,
    relay: &RelayUrl,
    edge_id: EndpointId,
    behavior: Behavior,
    acceptor: TlsAcceptor,
) -> Device {
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(key.clone())
        .relay_mode(RelayMode::custom([relay.clone()]))
        .clear_ip_transports()
        .alpns(vec![EDGE_ALPN.to_vec()])
        .bind()
        .await
        .expect("an endpoint with no IP transport must bind");
    tokio::time::timeout(SETUP_LIMIT, endpoint.online())
        .await
        .expect("the device never reached the relay");
    let device = Device {
        key: key.clone(),
        endpoint: endpoint.clone(),
        streams: Arc::default(),
        sunk: Arc::default(),
        alpn_seen: Arc::default(),
        strangers: Arc::default(),
    };
    let (streams, sunk, seen, strangers) = (
        device.streams.clone(),
        device.sunk.clone(),
        device.alpn_seen.clone(),
        device.strangers.clone(),
    );
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let Ok(conn) = incoming.await else { continue };
            if conn.remote_id() != edge_id {
                strangers.fetch_add(1, Ordering::SeqCst);
                conn.close(403u32.into(), b"not the edge");
                continue;
            }
            let (acceptor, streams, sunk, seen) = (
                acceptor.clone(),
                streams.clone(),
                sunk.clone(),
                seen.clone(),
            );
            tokio::spawn(async move {
                let (mut held_send, mut held_recv) = (Vec::new(), Vec::new());
                while let Ok((send, recv)) = conn.accept_bi().await {
                    streams.fetch_add(1, Ordering::SeqCst);
                    match behavior {
                        Behavior::Hold => {
                            held_send.push(send);
                            held_recv.push(recv);
                        }
                        Behavior::Sink => {
                            tokio::spawn(sink(recv, sunk.clone()));
                            held_send.push(send);
                        }
                        Behavior::Http => {
                            tokio::spawn(answer(acceptor.clone(), send, recv, seen.clone()));
                        }
                    }
                }
            });
        }
    });
    device
}

async fn sink(mut recv: iroh::endpoint::RecvStream, sunk: Arc<AtomicU64>) {
    let mut buf = vec![0u8; 16 * 1024];
    while let Ok(Some(n)) = recv.read(&mut buf).await {
        sunk.fetch_add(n as u64, Ordering::SeqCst);
    }
}

async fn answer(
    acceptor: TlsAcceptor,
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    seen: Arc<Mutex<Option<Vec<u8>>>>,
) {
    let Ok(mut tls) = acceptor.accept(tokio::io::join(recv, send)).await else {
        return;
    };
    *seen.lock().unwrap() = tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec);
    let mut request = Vec::new();
    let mut buf = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match tls.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
        BODY.len()
    );
    let _ = tls.write_all(response.as_bytes()).await;
    let _ = tls.shutdown().await;
}

pub struct Rig {
    pub edge: Edge,
    pub dir: tempfile::TempDir,
    pub list: PathBuf,
    pub relay_url: RelayUrl,
    pub edge_id: EndpointId,
}

impl Rig {
    pub fn addr(&self) -> SocketAddr {
        self.edge.local_addr()
    }

    pub async fn connect(&self) -> TcpStream {
        TcpStream::connect(self.addr()).await.unwrap()
    }
}

/// An edge on loopback with the given devices registered and a relay without TLS. `tune` adjusts
/// the config before the edge starts.
pub async fn rig(devices: &[EndpointId], tune: impl FnOnce(&mut EdgeConfig)) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let list = dir.path().join("devices.txt");
    for id in devices {
        allow_device(
            &list,
            Registered {
                id: *id,
                relay: None,
            },
        )
        .unwrap();
    }
    let key = load_or_create_key(&dir.path().join("edge.key")).unwrap();
    let edge_id = key.public();
    let registry = Arc::new(AllowlistFile::open(&list).unwrap());
    let relay = RelayHost::spawn(
        RelayParams {
            hostname: RELAY_NAME.into(),
            tls: RelayTls::PlainHttp,
            quic_bind: None,
        },
        registry.clone(),
        edge_id,
    )
    .await
    .unwrap();
    let relay_url = relay.url().clone();
    let mut config = EdgeConfig {
        enabled: true,
        domain: DOMAIN.into(),
        listen: "127.0.0.1:0".parse().unwrap(),
        dir: dir.path().to_path_buf(),
        iroh_bind: "127.0.0.1:0".parse().unwrap(),
        ..EdgeConfig::default()
    };
    tune(&mut config);
    let edge = Edge::start_with(config, registry, key, Some(relay))
        .await
        .unwrap();
    Rig {
        edge,
        dir,
        list,
        relay_url,
        edge_id,
    }
}

/// Reads until the peer closes, returning the bytes read and how long it took. A reset counts as
/// a close. Gives up (and panics) after `limit`.
pub async fn read_until_closed(stream: &mut TcpStream, limit: Duration) -> (Vec<u8>, Duration) {
    let started = Instant::now();
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let left = limit.saturating_sub(started.elapsed());
        match tokio::time::timeout(left, stream.read(&mut buf)).await {
            Err(_) => panic!("still open after {limit:?} (read {} bytes)", got.len()),
            Ok(Ok(0)) | Ok(Err(_)) => return (got, started.elapsed()),
            Ok(Ok(n)) => got.extend_from_slice(&buf[..n]),
        }
    }
}

/// Whether the connection is still open after `wait` (nothing arrived and it did not close).
pub async fn stays_open(stream: &mut TcpStream, wait: Duration) -> bool {
    let mut buf = [0u8; 16];
    tokio::time::timeout(wait, stream.read(&mut buf))
        .await
        .is_err()
}

/// Polls `check` every few milliseconds until it holds or `limit` passes.
pub async fn eventually(limit: Duration, what: &str, mut check: impl FnMut() -> bool) {
    let started = Instant::now();
    while !check() {
        assert!(started.elapsed() < limit, "timed out waiting for {what}");
        tokio::time::sleep(STEP).await;
    }
}
