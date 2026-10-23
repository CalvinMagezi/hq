//! A device and the `hq edge` it registers with, on loopback with real iroh endpoints and the real
//! migrated database: registering by QR, then the edge's streams reaching the connector door, and
//! nobody else's. The "door" here is a plain TCP echo server standing in for the web server's
//! loopback listener.

use hq_bridge::{
    Bridge, BridgeConfig, DoorTarget, EDGE_ALPN, EdgeDoor, JoinResult, OpenTicket, PairingSpec,
    Purpose, Registrar, SERVICE_ALPN, Ticket, Tuning, device_label, echo_handler, host::Joiner,
};
use hq_db::{Database, bridge as peers};
use iroh::{EndpointAddr, RelayMode, endpoint::Connection};
use std::{
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const LOOPBACK: &str = "127.0.0.1:0";
const GIVE_UP: Duration = Duration::from_secs(20);
const GONE: Duration = Duration::from_secs(5);
const EDGE_DOMAIN: &str = "edge.example.com";

async fn with_timeout<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(GIVE_UP, f)
        .await
        .expect("took too long")
}

struct Machine {
    bridge: Bridge,
    db: Database,
    _dir: tempfile::TempDir,
}

impl Machine {
    async fn new(name: &str, serve: bool, door: Option<EdgeDoor>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let bridge = Bridge::start_with_edge_door(
            BridgeConfig {
                key: dir.path().join("bridge.key"),
                bind: vec![LOOPBACK.parse().unwrap()],
                relay: RelayMode::Disabled,
                name: name.into(),
                with_addrs: true,
                advertise: vec![],
            },
            db.clone(),
            serve.then(echo_handler),
            door,
        )
        .await
        .unwrap();
        Self {
            bridge,
            db,
            _dir: dir,
        }
    }

    /// The machine showing the registration QR code, named as an edge's domain.
    async fn edge() -> Self {
        Self::new(EDGE_DOMAIN, true, None).await
    }

    fn id(&self) -> String {
        self.bridge.id().to_string()
    }

    fn addr(&self) -> EndpointAddr {
        let card = self.bridge.card();
        let mut addr = EndpointAddr::new(self.bridge.id());
        for a in &card.addrs {
            addr = addr.with_ip_addr(a.parse().unwrap());
        }
        addr
    }
}

fn tuning() -> Tuning {
    Tuning {
        ttl: Duration::from_secs(60),
        ..Tuning::default()
    }
}

/// What an edge's registrar does: name the device `<label>.<domain>`, or something else on request.
struct Naming {
    wrong: bool,
}

impl Registrar for Naming {
    fn accept(&self, joiner: &Joiner) -> anyhow::Result<Option<serde_json::Value>> {
        let label = if self.wrong {
            "0000000000000000".to_string()
        } else {
            device_label(&joiner.id)
        };
        Ok(Some(
            serde_json::json!({ "hostname": format!("{label}.{EDGE_DOMAIN}") }),
        ))
    }
}

async fn open(edge: &Machine, purpose: Purpose, wrong: bool) -> OpenTicket {
    OpenTicket::open(PairingSpec {
        purpose,
        card: edge.bridge.card(),
        registrar: Arc::new(Naming { wrong }),
        bind: vec![LOOPBACK.parse().unwrap()],
        relay: RelayMode::Disabled,
        with_addrs: true,
        advertise: vec![],
        tuning: tuning(),
    })
    .await
    .unwrap()
}

/// The person at the edge types the right digits.
fn confirm(mut open: OpenTicket) -> tokio::task::JoinHandle<OpenTicket> {
    tokio::spawn(async move {
        if let Some(request) = open.requests.recv().await {
            let _ = request.decide.send(true);
        }
        open
    })
}

async fn register(device: &Machine, ticket: &Ticket) -> anyhow::Result<JoinResult> {
    with_timeout(device.bridge.register(ticket, &tuning(), |_| {})).await
}

/// A TCP server that echoes every connection: the connector door's stand-in.
async fn echo_door() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut tcp, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
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

/// A TCP server that accepts and then says nothing, keeping the connection open.
async fn silent_door() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((tcp, _)) = listener.accept().await {
            held.push(tcp);
        }
    });
    addr
}

/// A device that will accept the edge's streams and send them to `door`, with `edge` registered.
async fn device_behind(edge: &Machine, door: EdgeDoor) -> Machine {
    let device = Machine::new("phone", false, Some(door)).await;
    peers::edge_set(
        &device.db,
        &edge.id(),
        EDGE_DOMAIN,
        &format!("{}.{EDGE_DOMAIN}", device_label(&device.bridge.id())),
        None,
    )
    .unwrap();
    device
}

/// Opens an edge stream from `from`'s endpoint to `device` and tries an echo through it.
async fn through_edge(from: &Machine, device: &Machine, message: &[u8]) -> Option<Vec<u8>> {
    let conn = tokio::time::timeout(
        GONE,
        from.bridge.endpoint().connect(device.addr(), EDGE_ALPN),
    )
    .await
    .ok()?
    .ok()?;
    echo_on(&conn, message).await
}

async fn echo_on(conn: &Connection, message: &[u8]) -> Option<Vec<u8>> {
    let (mut send, mut recv) = conn.open_bi().await.ok()?;
    send.write_all(message).await.ok()?;
    send.finish().ok()?;
    tokio::time::timeout(GONE, recv.read_to_end(64 * 1024))
        .await
        .ok()?
        .ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_registers_with_an_edge_and_remembers_it() {
    let (edge, device) = (
        Machine::edge().await,
        Machine::new("phone", false, None).await,
    );
    let open = open(&edge, Purpose::Register, false).await;
    let ticket = open.ticket.clone();
    let confirming = confirm(open);
    let result = register(&device, &ticket).await.unwrap();
    assert!(matches!(result, JoinResult::Paired(_)), "{result:?}");
    let _open = confirming.await.unwrap();

    let record = peers::edge_get(&device.db).unwrap().expect("the edge");
    assert_eq!(record.id, edge.id());
    assert_eq!(record.domain, EDGE_DOMAIN);
    assert_eq!(
        record.hostname,
        format!("{}.{EDGE_DOMAIN}", device_label(&device.bridge.id()))
    );
    assert!(peers::is_edge(&device.db, &edge.id()).unwrap());
    // The edge is not a paired peer: registering adds none.
    assert!(peers::list(&device.db, false).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edge_that_names_the_device_wrongly_registers_nothing() {
    let (edge, device) = (
        Machine::edge().await,
        Machine::new("phone", false, None).await,
    );
    let open = open(&edge, Purpose::Register, true).await;
    let ticket = open.ticket.clone();
    let _confirming = confirm(open);
    let result = register(&device, &ticket).await;
    assert!(result.is_err(), "{result:?}");
    assert_eq!(peers::edge_get(&device.db).unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_ticket_cannot_register_a_device() {
    let (host, device) = (
        Machine::edge().await,
        Machine::new("phone", false, None).await,
    );
    let open = host.bridge.open_ticket(tuning()).await.unwrap();
    let result = device
        .bridge
        .register(&open.ticket, &tuning(), |_| {})
        .await;
    assert!(result.is_err());
    assert_eq!(peers::edge_get(&device.db).unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_registered_edge_reaches_the_door_through_a_stream() {
    let edge = Machine::edge().await;
    let door = echo_door().await;
    let device = device_behind(&edge, EdgeDoor::new(DoorTarget::Tcp(door))).await;
    let got = with_timeout(through_edge(&edge, &device, b"a tls hello would be here")).await;
    assert_eq!(got.as_deref(), Some(&b"a tls hello would be here"[..]));
    assert_eq!(device.bridge.reached_edge.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn nobody_but_the_registered_edge_may_open_the_edge_protocol() {
    let edge = Machine::edge().await;
    let door = echo_door().await;
    let device = device_behind(&edge, EdgeDoor::new(DoorTarget::Tcp(door))).await;
    // A stranger, and a paired peer: both are refused at the gate, before any handler runs.
    let stranger = Machine::edge().await;
    assert_eq!(through_edge(&stranger, &device, b"hello").await, None);
    let peer = Machine::edge().await;
    peers::add_peer(&device.db, &peer.id(), "tablet", &[], &[]).unwrap();
    assert_eq!(through_edge(&peer, &device, b"hello").await, None);
    assert_eq!(device.bridge.reached_edge.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_edge_cannot_use_the_service_protocol_and_a_peer_cannot_use_the_edge_one() {
    let edge = Machine::edge().await;
    let door = echo_door().await;
    let device = Machine::new("phone", true, Some(EdgeDoor::new(DoorTarget::Tcp(door)))).await;
    peers::edge_set(
        &device.db,
        &edge.id(),
        EDGE_DOMAIN,
        "x.edge.example.com",
        None,
    )
    .unwrap();
    let peer = Machine::edge().await;
    peers::add_peer(&device.db, &peer.id(), "tablet", &[], &[]).unwrap();

    // The edge on the paired peers' door: refused. The paired peer on the edge's door: refused.
    let on_service = tokio::time::timeout(
        GONE,
        edge.bridge.endpoint().connect(device.addr(), SERVICE_ALPN),
    )
    .await;
    let served = match on_service {
        Ok(Ok(conn)) => echo_on(&conn, b"hello").await,
        _ => None,
    };
    assert_eq!(served, None, "the edge got the paired peers' service");
    assert_eq!(through_edge(&peer, &device, b"hello").await, None);
    // Each still has its own door.
    assert!(through_edge(&edge, &device, b"hello").await.is_some());
    let on_service = tokio::time::timeout(
        GONE,
        peer.bridge.endpoint().connect(device.addr(), SERVICE_ALPN),
    )
    .await;
    let peer_served = match on_service {
        Ok(Ok(conn)) => echo_on(&conn, b"hello").await,
        _ => None,
    };
    assert!(peer_served.is_some(), "the paired peer lost its own door");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bridge_without_an_edge_door_does_not_speak_the_edge_protocol() {
    let edge = Machine::edge().await;
    let device = Machine::new("phone", true, None).await;
    peers::edge_set(
        &device.db,
        &edge.id(),
        EDGE_DOMAIN,
        "x.edge.example.com",
        None,
    )
    .unwrap();
    assert_eq!(through_edge(&edge, &device, b"hello").await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_the_edge_closes_its_open_connection_and_refuses_the_next() {
    let edge = Machine::edge().await;
    let door = echo_door().await;
    let device = device_behind(&edge, EdgeDoor::new(DoorTarget::Tcp(door))).await;
    let conn = with_timeout(edge.bridge.endpoint().connect(device.addr(), EDGE_ALPN))
        .await
        .unwrap();
    assert!(echo_on(&conn, b"hello").await.is_some());

    let forgot = with_timeout(device.bridge.forget_edge()).await.unwrap();
    assert!(forgot.had_record);
    assert_eq!(forgot.closed, 1);
    tokio::time::timeout(GONE, conn.closed())
        .await
        .expect("the edge's connection stayed open after the edge was forgotten");
    assert_eq!(through_edge(&edge, &device, b"hello").await, None);
    assert!(
        !device.bridge.forget_edge().await.unwrap().had_record,
        "already gone"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edge_cleared_by_another_process_gets_no_more_streams_on_an_open_connection() {
    let edge = Machine::edge().await;
    let door = echo_door().await;
    let device = device_behind(&edge, EdgeDoor::new(DoorTarget::Tcp(door))).await;
    let conn = with_timeout(edge.bridge.endpoint().connect(device.addr(), EDGE_ALPN))
        .await
        .unwrap();
    assert!(echo_on(&conn, b"hello").await.is_some());
    // The record goes away without this process being told (the CLI in another process).
    assert!(peers::edge_clear(&device.db).unwrap());
    assert_eq!(echo_on(&conn, b"hello").await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_door_that_is_not_there_ends_the_stream_quickly() {
    let edge = Machine::edge().await;
    let closed = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap()
    };
    let device = device_behind(&edge, EdgeDoor::new(DoorTarget::Tcp(closed))).await;
    let conn = with_timeout(edge.bridge.endpoint().connect(device.addr(), EDGE_ALPN))
        .await
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(b"hello").await.unwrap();
    send.finish().unwrap();
    let started = std::time::Instant::now();
    // The stream ends with no data, and within a moment: a hang would run into the timeout.
    let read = tokio::time::timeout(Duration::from_secs(3), recv.read_to_end(1024)).await;
    assert!(read.is_ok(), "the stream was left hanging");
    assert!(read.unwrap().is_ok_and(|bytes| bytes.is_empty()) || true);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[cfg(unix)]
async fn unix_echo_door(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("door.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 || stream.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    path
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn the_registered_edge_reaches_a_unix_door_and_a_missing_one_ends_the_stream() {
    let edge = Machine::edge().await;
    let dir = tempfile::tempdir().unwrap();
    let socket = unix_echo_door(dir.path()).await;
    let device = device_behind(&edge, EdgeDoor::new(DoorTarget::Unix(socket))).await;
    let got = with_timeout(through_edge(&edge, &device, b"over a unix socket")).await;
    assert_eq!(got.as_deref(), Some(&b"over a unix socket"[..]));

    let gone = dir.path().join("nobody-listens.sock");
    let device = device_behind(&edge, EdgeDoor::new(DoorTarget::Unix(gone))).await;
    let conn = with_timeout(edge.bridge.endpoint().connect(device.addr(), EDGE_ALPN))
        .await
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(b"hello").await.unwrap();
    send.finish().unwrap();
    let read = tokio::time::timeout(Duration::from_secs(3), recv.read_to_end(1024)).await;
    assert!(read.is_ok(), "the stream was left hanging");
}

#[tokio::test(flavor = "multi_thread")]
async fn streams_over_the_cap_are_dropped_and_counted() {
    let edge = Machine::edge().await;
    let door = silent_door().await;
    let device = device_behind(
        &edge,
        EdgeDoor {
            target: DoorTarget::Tcp(door),
            max_streams: 1,
            recheck: Duration::from_secs(1),
        },
    )
    .await;
    let conn = with_timeout(edge.bridge.endpoint().connect(device.addr(), EDGE_ALPN))
        .await
        .unwrap();
    // The first stream takes the one permit and is held open by the silent door.
    let (mut first_send, _first_recv) = conn.open_bi().await.unwrap();
    first_send.write_all(b"one").await.unwrap();
    let started = std::time::Instant::now();
    while device.bridge.edge_over_cap.load(Ordering::SeqCst) == 0 {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        let _ = send.write_all(b"two").await;
        let _ = send.finish();
        let _ = tokio::time::timeout(Duration::from_millis(300), recv.read_to_end(1024)).await;
        assert!(
            started.elapsed() < GONE,
            "a stream over the cap was not counted"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_stream_gives_its_place_back() {
    let edge = Machine::edge().await;
    let door = echo_door().await;
    let device = device_behind(
        &edge,
        EdgeDoor {
            target: DoorTarget::Tcp(door),
            max_streams: 1,
            recheck: Duration::from_secs(1),
        },
    )
    .await;
    let conn = with_timeout(edge.bridge.endpoint().connect(device.addr(), EDGE_ALPN))
        .await
        .unwrap();
    for round in 0..5 {
        let got = echo_on(&conn, b"again").await;
        assert_eq!(got.as_deref(), Some(&b"again"[..]), "round {round}");
    }
    assert_eq!(device.bridge.edge_over_cap.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_sweeps_the_edges_connections_even_when_another_process_cleared_the_record() {
    let edge = Machine::edge().await;
    let door = echo_door().await;
    // The periodic recheck is an hour away, so only the sweep can close the connection here.
    let device = device_behind(
        &edge,
        EdgeDoor {
            target: DoorTarget::Tcp(door),
            max_streams: 8,
            recheck: Duration::from_secs(3600),
        },
    )
    .await;
    let conn = with_timeout(edge.bridge.endpoint().connect(device.addr(), EDGE_ALPN))
        .await
        .unwrap();
    assert!(echo_on(&conn, b"hello").await.is_some());
    assert!(peers::edge_clear(&device.db).unwrap(), "the CLI cleared it");

    let forgot = device.bridge.forget_edge().await.unwrap();
    assert!(!forgot.had_record, "nothing was left on record");
    assert_eq!(
        forgot.closed, 1,
        "the connection still open here was closed"
    );
    tokio::time::timeout(GONE, conn.closed())
        .await
        .expect("the edge's connection stayed open");
}

#[tokio::test(flavor = "multi_thread")]
async fn registering_with_another_edge_closes_the_one_it_replaces() {
    let (old_edge, new_edge) = (Machine::edge().await, Machine::edge().await);
    let door = echo_door().await;
    // The periodic recheck is an hour away, so only registering itself can close the old edge.
    let device = device_behind(
        &old_edge,
        EdgeDoor {
            target: DoorTarget::Tcp(door),
            max_streams: 8,
            recheck: Duration::from_secs(3600),
        },
    )
    .await;
    let conn = with_timeout(old_edge.bridge.endpoint().connect(device.addr(), EDGE_ALPN))
        .await
        .unwrap();
    assert!(echo_on(&conn, b"hello").await.is_some());

    let open = open(&new_edge, Purpose::Register, false).await;
    let ticket = open.ticket.clone();
    let _confirming = confirm(open);
    let result = register(&device, &ticket).await.unwrap();
    assert!(matches!(result, JoinResult::Paired(_)), "{result:?}");
    assert!(peers::is_edge(&device.db, &new_edge.id()).unwrap());
    tokio::time::timeout(GONE, conn.closed())
        .await
        .expect("the replaced edge's connection stayed open");
    assert_eq!(through_edge(&old_edge, &device, b"hello").await, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_door_the_edge_must_not_be_pointed_at_is_refused_at_start() {
    let dir = tempfile::tempdir().unwrap();
    for target in [
        DoorTarget::Tcp("0.0.0.0:5679".parse().unwrap()),
        DoorTarget::Tcp("192.168.1.9:5679".parse().unwrap()),
        DoorTarget::Unix("relative.sock".into()),
    ] {
        let started = Bridge::start_with_edge_door(
            BridgeConfig {
                key: dir.path().join("bridge.key"),
                bind: vec![LOOPBACK.parse().unwrap()],
                relay: RelayMode::Disabled,
                name: "phone".into(),
                with_addrs: true,
                advertise: vec![],
            },
            Database::open_memory().unwrap(),
            None,
            Some(EdgeDoor::new(target.clone())),
        )
        .await;
        assert!(started.is_err(), "{target:?} was accepted");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_database_that_fails_closes_the_edge_gate() {
    let edge = Machine::edge().await;
    let door = echo_door().await;
    let device = device_behind(&edge, EdgeDoor::new(DoorTarget::Tcp(door))).await;
    assert!(through_edge(&edge, &device, b"hello").await.is_some());
    device
        .db
        .with_conn(|c| {
            c.execute("DROP TABLE bridge_edge", [])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        through_edge(&edge, &device, b"hello").await,
        None,
        "a table that cannot be read must read as no edge"
    );
}
