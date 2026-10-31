//! Named services between two paired machines on loopback, with real iroh endpoints and the real
//! migrated database: a forward on one reaches a service on the other, and nobody else does. The
//! "service" is a plain TCP echo server standing in for the web server's or the gate's port.

use hq_bridge::{Bridge, BridgeConfig, DoorTarget, Services, open_service, run_forward};
use hq_db::{Database, bridge as peers};
use iroh::RelayMode;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const LOOPBACK: &str = "127.0.0.1:0";
const GIVE_UP: Duration = Duration::from_secs(20);

async fn with_timeout<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(GIVE_UP, f)
        .await
        .expect("took too long")
}

struct Machine {
    bridge: Arc<Bridge>,
    db: Database,
    _dir: tempfile::TempDir,
}

impl Machine {
    async fn new(name: &str, services: Option<Services>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let bridge = Bridge::start_with_services(
            BridgeConfig {
                key: dir.path().join("bridge.key"),
                bind: vec![LOOPBACK.parse().unwrap()],
                relay: RelayMode::Disabled,
                name: name.into(),
                with_addrs: true,
                advertise: vec![],
            },
            db.clone(),
            None,
            None,
            services,
        )
        .await
        .unwrap();
        Self {
            bridge: Arc::new(bridge),
            db,
            _dir: dir,
        }
    }

    fn id(&self) -> String {
        self.bridge.id().to_string()
    }

    /// Puts `other` on this machine's list, as a finished pairing would.
    fn knows(&self, other: &Machine) {
        let card = other.bridge.card();
        peers::add_peer(&self.db, &card.id, &card.name, &card.relays, &card.addrs).unwrap();
    }
}

/// A TCP echo server on loopback, standing in for a service's port.
async fn echo_server() -> SocketAddr {
    let listener = TcpListener::bind(LOOPBACK).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = tcp.read(&mut buf).await {
                    if n == 0 || tcp.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

fn offering(name: &str, addr: SocketAddr) -> Services {
    Services::new(BTreeMap::from([(name.to_string(), DoorTarget::Tcp(addr))]))
}

/// A paired pair: `phone` offers `web`, `mac` knows it and is known by it.
async fn pair() -> (Machine, Machine) {
    let web = echo_server().await;
    let phone = Machine::new("phone", Some(offering("web", web))).await;
    let mac = Machine::new("mac", None).await;
    phone.knows(&mac);
    mac.knows(&phone);
    (phone, mac)
}

async fn forward(mac: &Machine, peer: &str, service: &str) -> SocketAddr {
    let listener = TcpListener::bind(LOOPBACK).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(run_forward(
        mac.bridge.dialer(),
        peer.to_string(),
        service.to_string(),
        listener,
    ));
    addr
}

/// Writes `msg` and reads it back, or `None` if the line closed or stayed silent.
async fn round_trip(addr: SocketAddr, msg: &[u8]) -> Option<Vec<u8>> {
    let mut tcp = TcpStream::connect(addr).await.ok()?;
    tcp.write_all(msg).await.ok()?;
    let mut got = vec![0u8; msg.len()];
    let read = tokio::time::timeout(Duration::from_secs(5), tcp.read_exact(&mut got)).await;
    matches!(read, Ok(Ok(_))).then_some(got)
}

#[tokio::test]
async fn a_forward_reaches_the_named_service_on_a_paired_peer() {
    let (phone, mac) = pair().await;
    let port = forward(&mac, &phone.id(), "web").await;
    for msg in [&b"first"[..], b"second, on a new stream"] {
        let got = with_timeout(round_trip(port, msg)).await;
        assert_eq!(got.as_deref(), Some(msg));
    }
}

#[tokio::test]
async fn an_unknown_service_is_refused_by_name() {
    let (phone, mac) = pair().await;
    let conn = with_timeout(mac.bridge.connect_service(&phone.id()))
        .await
        .unwrap();
    let err = with_timeout(open_service(&conn, "ssh")).await.unwrap_err();
    assert!(err.to_string().contains("no service called ssh"), "{err}");
    assert!(with_timeout(open_service(&conn, "web")).await.is_ok());
}

#[tokio::test]
async fn a_machine_that_is_not_paired_gets_nothing() {
    let (phone, _mac) = pair().await;
    let stranger = Machine::new("stranger", None).await;
    // The stranger knows how to dial the phone, but the phone does not know the stranger.
    stranger.knows(&phone);
    let reached = with_timeout(async {
        let conn = stranger.bridge.connect_service(&phone.id()).await?;
        open_service(&conn, "web").await
    })
    .await;
    assert!(reached.is_err(), "an unpaired machine reached a service");
}

#[tokio::test]
async fn a_machine_that_offers_no_services_does_not_speak_the_protocol() {
    let (phone, mac) = pair().await;
    // An endpoint that accepts nothing never finishes the handshake, so "refused" may also look
    // like silence; either way nothing is reached.
    let reached = tokio::time::timeout(Duration::from_secs(5), async {
        let conn = phone.bridge.connect_service(&mac.id()).await?;
        open_service(&conn, "web").await
    })
    .await;
    assert!(!matches!(reached, Ok(Ok(_))));
}

#[tokio::test]
async fn revoking_closes_open_streams_and_refuses_new_ones() {
    let (phone, mac) = pair().await;
    let port = forward(&mac, &phone.id(), "web").await;
    let mut open = TcpStream::connect(port).await.unwrap();
    open.write_all(b"hello").await.unwrap();
    let mut got = [0u8; 5];
    with_timeout(open.read_exact(&mut got)).await.unwrap();

    phone.bridge.revoke(&mac.bridge.id()).await.unwrap();

    // The open stream ends: a write may still be buffered, but nothing comes back.
    let _ = open.write_all(b"again").await;
    let mut more = [0u8; 5];
    let after = tokio::time::timeout(Duration::from_secs(5), open.read_exact(&mut more)).await;
    assert!(
        !matches!(after, Ok(Ok(_))),
        "an echo came back after revoke"
    );
    assert_eq!(with_timeout(round_trip(port, b"new")).await, None);
}

#[tokio::test]
async fn a_service_whose_port_is_down_says_so() {
    let closed = {
        let l = TcpListener::bind(LOOPBACK).await.unwrap();
        l.local_addr().unwrap()
    };
    let phone = Machine::new("phone", Some(offering("web", closed))).await;
    let mac = Machine::new("mac", None).await;
    phone.knows(&mac);
    mac.knows(&phone);
    let conn = with_timeout(mac.bridge.connect_service(&phone.id()))
        .await
        .unwrap();
    let err = with_timeout(open_service(&conn, "web")).await.unwrap_err();
    assert!(err.to_string().contains("not answering"), "{err}");
}

/// A revoke made by another process (`hq bridge revoke`) changes only the list. The forward must
/// still stop: it reads the list before every new local connection and drops its kept connection.
#[tokio::test]
async fn a_forward_stops_when_its_peer_is_revoked_from_another_process() {
    let (phone, mac) = pair().await;
    let port = forward(&mac, &phone.id(), "web").await;
    assert!(with_timeout(round_trip(port, b"before")).await.is_some());
    peers::revoke(&mac.db, &phone.id()).unwrap();
    assert_eq!(with_timeout(round_trip(port, b"after")).await, None);
}
