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
    forward_failing(mac, peer, service, None).await
}

async fn forward_failing(
    mac: &Machine,
    peer: &str,
    service: &str,
    fail_fast: Option<Duration>,
) -> SocketAddr {
    let listener = TcpListener::bind(LOOPBACK).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(run_forward(
        mac.bridge.dialer(),
        peer.to_string(),
        service.to_string(),
        listener,
        fail_fast,
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
async fn a_service_with_a_peers_list_admits_only_those_peers() {
    let (web, llm) = (echo_server().await, echo_server().await);
    let phone = Machine::new("phone", None).await;
    let tablet = Machine::new("tablet", None).await;
    let mut offered = offering("web", web);
    offered.targets.insert("llm".into(), DoorTarget::Tcp(llm));
    offered
        .peers
        .insert("llm".into(), [phone.bridge.id()].into());
    let laptop = Machine::new("laptop", Some(offered)).await;
    for device in [&phone, &tablet] {
        laptop.knows(device);
        device.knows(&laptop);
    }

    let from_phone = with_timeout(phone.bridge.connect_service(&laptop.id()))
        .await
        .unwrap();
    assert!(with_timeout(open_service(&from_phone, "llm")).await.is_ok());
    let port = forward(&phone, &laptop.id(), "llm").await;
    assert_eq!(
        with_timeout(round_trip(port, b"hello")).await.as_deref(),
        Some(&b"hello"[..])
    );

    // Another paired device is told the service does not exist, and keeps the open ones.
    let from_tablet = with_timeout(tablet.bridge.connect_service(&laptop.id()))
        .await
        .unwrap();
    let err = with_timeout(open_service(&from_tablet, "llm"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no service called llm"), "{err}");
    assert!(
        with_timeout(open_service(&from_tablet, "web"))
            .await
            .is_ok()
    );
    let port = forward(&tablet, &laptop.id(), "llm").await;
    assert_eq!(with_timeout(round_trip(port, b"hello")).await, None);
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

/// A forward from the config that listens on a socket path: it reaches the service, its socket
/// and the directory made for it are this user's alone, and a second bridge leaves it alone.
#[cfg(unix)]
#[tokio::test]
async fn a_forward_on_a_private_socket_reaches_the_service_and_is_not_taken_over() {
    use hq_core::config::{BridgeForward, BridgeServicesConfig};
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

    let (phone, mac) = pair().await;
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("bridge").join("web.sock");
    let config = BridgeServicesConfig {
        forwards: vec![BridgeForward {
            peer: phone.id(),
            service: "web".into(),
            listen: socket.display().to_string(),
            fail_fast_secs: None,
        }],
        ..BridgeServicesConfig::default()
    };
    let forwards = hq_bridge::start_forwards(&mac.bridge, &config)
        .await
        .unwrap();
    assert!(!forwards.status[0].running_elsewhere);
    assert_eq!(mode(socket.parent().unwrap()), 0o700);
    assert_eq!(mode(&socket), 0o600);

    let mut unix = tokio::net::UnixStream::connect(&socket).await.unwrap();
    unix.write_all(b"through the socket").await.unwrap();
    let mut got = vec![0u8; b"through the socket".len()];
    with_timeout(unix.read_exact(&mut got)).await.unwrap();
    assert_eq!(got, b"through the socket");

    let again = hq_bridge::start_forwards(&mac.bridge, &config)
        .await
        .unwrap();
    assert!(
        again.status[0].running_elsewhere,
        "a live socket is not taken over"
    );
    forwards.abort();
}

/// A paired machine that accepts service streams and never answers them, like a laptop whose lid
/// closed after the connection was made: packets go out and nothing comes back.
struct Silent {
    endpoint: iroh::Endpoint,
    _router: iroh::protocol::Router,
}

#[derive(Debug, Clone)]
struct NeverAnswer;

impl iroh::protocol::ProtocolHandler for NeverAnswer {
    async fn accept(
        &self,
        conn: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        while let Ok(stream) = conn.accept_bi().await {
            tokio::spawn(async move {
                let _held = stream;
                tokio::time::sleep(Duration::from_secs(3600)).await;
            });
        }
        Ok(())
    }
}

impl Silent {
    async fn new() -> Self {
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr(LOOPBACK.parse::<SocketAddr>().unwrap())
            .unwrap()
            .bind()
            .await
            .unwrap();
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(hq_bridge::SVC_ALPN, NeverAnswer)
            .spawn();
        Self {
            endpoint,
            _router: router,
        }
    }

    fn id(&self) -> String {
        self.endpoint.id().to_string()
    }

    fn known_by(&self, mac: &Machine) {
        let addrs: Vec<String> = self
            .endpoint
            .bound_sockets()
            .iter()
            .map(|a| a.to_string())
            .collect();
        peers::add_peer(&mac.db, &self.id(), "silent", &[], &addrs).unwrap();
    }
}

/// How long a connection to the forward at `addr` stays open before the forward closes it.
async fn time_to_close(addr: SocketAddr) -> Duration {
    let started = std::time::Instant::now();
    let mut tcp = TcpStream::connect(addr).await.unwrap();
    let _ = tcp.write_all(b"hello").await;
    let mut buf = [0u8; 64];
    let _ = tokio::time::timeout(Duration::from_secs(60), tcp.read(&mut buf)).await;
    started.elapsed()
}

const FAIL_FAST: Duration = Duration::from_secs(2);
/// Slack for scheduling on a loaded test machine.
const MARGIN: Duration = Duration::from_millis(1500);
/// A connection to a peer already known to be unreachable ends at about once.
const AT_ONCE: Duration = Duration::from_millis(500);

/// Without `fail_fast`, a forward to a peer that is gone waits out the dial: this is the old
/// behaviour, kept for forwards with no fallback (the web app, the host gate).
#[tokio::test]
async fn without_fail_fast_a_forward_still_waits_for_a_peer_that_is_gone() {
    let (phone, mac) = pair().await;
    let port = forward(&mac, &phone.id(), "web").await;
    assert!(with_timeout(round_trip(port, b"up")).await.is_some());
    let Machine { bridge, .. } = phone;
    Arc::try_unwrap(bridge).ok().unwrap().shutdown().await;
    assert!(time_to_close(port).await > FAIL_FAST + MARGIN);
}

/// A peer that is gone costs one `fail_fast` wait, then connections end at once until it is back,
/// and the first connection after a background try reached it gets through.
#[tokio::test]
async fn a_fail_fast_forward_gives_up_on_a_peer_that_is_gone_and_finds_it_again() {
    let web = echo_server().await;
    let key_dir = tempfile::tempdir().unwrap();
    let start_phone = |port: u16| {
        let key = key_dir.path().join("bridge.key");
        async move {
            let db = Database::open_memory().unwrap();
            let bridge = Bridge::start_with_services(
                BridgeConfig {
                    key,
                    bind: vec![SocketAddr::from(([127, 0, 0, 1], port))],
                    relay: RelayMode::Disabled,
                    name: "phone".into(),
                    with_addrs: true,
                    advertise: vec![],
                },
                db.clone(),
                None,
                None,
                Some(offering("web", web)),
            )
            .await
            .unwrap();
            (bridge, db)
        }
    };
    let mac = Machine::new("mac", None).await;
    let (phone, phone_db) = start_phone(0).await;
    let card = phone.card();
    peers::add_peer(&mac.db, &card.id, &card.name, &card.relays, &card.addrs).unwrap();
    let mac_card = mac.bridge.card();
    peers::add_peer(&phone_db, &mac_card.id, "mac", &[], &mac_card.addrs).unwrap();
    let port = forward_failing(&mac, &card.id, "web", Some(FAIL_FAST)).await;
    assert!(with_timeout(round_trip(port, b"up")).await.is_some());

    let phone_port: u16 = card.addrs[0].rsplit(':').next().unwrap().parse().unwrap();
    phone.shutdown().await;
    let first = time_to_close(port).await;
    assert!(first <= FAIL_FAST + MARGIN, "first: {first:?}");
    let next = time_to_close(port).await;
    assert!(next <= AT_ONCE, "next: {next:?}");

    let (_back, back_db) = start_phone(phone_port).await;
    peers::add_peer(&back_db, &mac_card.id, "mac", &[], &mac_card.addrs).unwrap();
    // Each connection while down asks for one background try; one of them finds the phone.
    let mut through = false;
    for _ in 0..20 {
        if round_trip(port, b"back").await.is_some() {
            through = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        through,
        "the phone came back but the forward never used it again"
    );
}

/// A peer that holds the connection open and never answers (a closed lid) costs one `fail_fast`
/// wait, not the ten seconds a service header may take, and later connections end at once.
#[tokio::test]
async fn a_fail_fast_forward_gives_up_on_a_peer_that_never_answers() {
    let mac = Machine::new("mac", None).await;
    let silent = Silent::new().await;
    silent.known_by(&mac);
    let port = forward_failing(&mac, &silent.id(), "llm", Some(FAIL_FAST)).await;
    let first = time_to_close(port).await;
    assert!(first <= FAIL_FAST + MARGIN, "first: {first:?}");
    for _ in 0..3 {
        let next = time_to_close(port).await;
        assert!(next <= AT_ONCE, "next: {next:?}");
    }
}
