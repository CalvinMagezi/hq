//! A revoke cleans both sides of a pairing, and only an explicit revoke does:
//! (a) the hub revokes the machine while connected, and the machine learns and cleans up;
//! (b) the machine revokes the hub while connected, and the hub learns and cleans up;
//! (c) an offline revoke is learned at the first dial;
//! (d) a 403 for another reason (not the edge, not paired) revokes nothing;
//! (e) a hub that cannot read its list says "unavailable", and nothing is revoked;
//! (f) a stranger cannot make us revoke a third peer;
//! (g) the watcher on a dialed connection does not keep it open.
#![cfg(unix)]

use hq_bridge::{
    Bridge, BridgeConfig, DoorTarget, EDGE_ALPN, EdgeDoor, JoinResult, Role, SVC_ALPN, Services,
    SetupConfig, Ticket, Tuning, error_close_reason,
    setup::{SVC_GATE, SVC_WEB, gate_label},
};
use hq_db::{Database, bridge as peers, bridge_setup as rows};
use iroh::{
    Endpoint, RelayMode, SecretKey,
    endpoint::{Connection, ConnectionError, presets},
};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const LOOPBACK: &str = "127.0.0.1:0";
const GIVE_UP: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

async fn with_timeout<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(GIVE_UP, f)
        .await
        .expect("took too long")
}

struct Node {
    bridge: Bridge,
    db: Database,
    dir: Arc<tempfile::TempDir>,
    role: Role,
    name: &'static str,
    service: SocketAddr,
}

impl Node {
    async fn hub() -> Self {
        Self::start(Role::Hub, "phone", true, None).await
    }

    async fn machine() -> Self {
        Self::start(Role::Machine, "laptop", true, None).await
    }

    async fn start(role: Role, name: &'static str, offers: bool, from: Option<Kept>) -> Self {
        let (db, dir, service) = match from {
            Some(kept) => kept,
            None => (
                Database::open_memory().unwrap(),
                Arc::new(tempfile::tempdir().unwrap()),
                echo_server().await,
            ),
        };
        let svc = if role == Role::Hub { SVC_WEB } else { SVC_GATE };
        let services = Services::new(BTreeMap::from([(
            svc.to_string(),
            DoorTarget::Tcp(service),
        )]));
        let mut setup = SetupConfig::new(role, authorized(dir.path()), dir.path().join("gate"));
        setup.web = role == Role::Hub && offers;
        setup.gate = role == Role::Machine && offers;
        let bridge = Bridge::start_with_setup(
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
            // A hub that serves the edge protocol too, so its gate has an edge to refuse.
            (role == Role::Hub).then(|| EdgeDoor::new(DoorTarget::Tcp(service))),
            Some(services),
            Some(setup),
        )
        .await
        .unwrap();
        bridge.keep_setup().unwrap();
        Self {
            bridge,
            db,
            dir,
            role,
            name,
            service,
        }
    }

    async fn stop(self) -> (iroh::EndpointId, Kept, Role, &'static str) {
        let Node {
            bridge,
            db,
            dir,
            role,
            name,
            service,
        } = self;
        let id = bridge.id();
        bridge.shutdown().await;
        (id, (db, dir, service), role, name)
    }

    fn id(&self) -> String {
        self.bridge.id().to_string()
    }

    fn row(&self, peer: &Node) -> anyhow::Result<Option<rows::Setup>> {
        rows::get(&self.db, &peer.id())
    }

    fn gate_lines(&self) -> String {
        std::fs::read_to_string(authorized(self.dir.path())).unwrap_or_default()
    }
}

type Kept = (Database, Arc<tempfile::TempDir>, SocketAddr);

fn authorized(dir: &Path) -> PathBuf {
    dir.join("gate_authorized_keys")
}

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

async fn round_trip(port: u16, msg: &[u8]) -> Option<Vec<u8>> {
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
    tcp.write_all(msg).await.ok()?;
    let mut got = vec![0u8; msg.len()];
    let read = tokio::time::timeout(Duration::from_secs(5), tcp.read_exact(&mut got)).await;
    matches!(read, Ok(Ok(_))).then_some(got)
}

async fn settles(port: u16, want: bool) -> bool {
    let end = tokio::time::Instant::now() + SETTLE;
    loop {
        if round_trip(port, b"ping").await.is_some() == want {
            return true;
        }
        if tokio::time::Instant::now() >= end {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn refuses(port: u16) -> bool {
    let end = tokio::time::Instant::now() + SETTLE;
    while tokio::time::Instant::now() < end {
        if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

async fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
    let end = tokio::time::Instant::now() + WAIT_TIMEOUT;
    while tokio::time::Instant::now() < end {
        if condition() {
            return true;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    condition()
}

async fn pair(hub: &Node, machine: &Node) {
    let mut open = hub
        .bridge
        .open_ticket(Tuning {
            ttl: Duration::from_secs(60),
            ..Tuning::default()
        })
        .await
        .unwrap();
    let ticket = Ticket::parse(&open.ticket.link()).unwrap();
    let answering = tokio::spawn(async move {
        if let Some(request) = open.requests.recv().await {
            let _ = request.decide.send(true);
        }
        open.closed().await;
    });
    let result = with_timeout(machine.bridge.join(&ticket, &Tuning::default(), |_| {}))
        .await
        .unwrap();
    assert!(matches!(result, JoinResult::Paired(_)), "{result:?}");
    with_timeout(answering).await.unwrap();
    assert!(peers::is_paired(&hub.db, &machine.id()).unwrap());
}

async fn paired_and_set_up() -> (Node, Node, rows::Setup) {
    let (hub, machine) = (Node::hub().await, Node::machine().await);
    pair(&hub, &machine).await;
    let outcome = with_timeout(machine.bridge.set_up(&hub.id()))
        .await
        .unwrap();
    assert!(outcome.sent);
    (hub, machine, outcome.setup)
}

/// Records the peers the revoke callback ran for.
fn count_callbacks(node: &Node) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    node.bridge.set_on_revoked(Arc::new(move |peer: &str| {
        record.lock().unwrap().push(peer.to_string());
    }));
    seen
}

/// After the wait that saw the callback run: a late duplicate in the settle period still fails.
async fn ran_once(seen: &Mutex<Vec<String>>, peer: &str) {
    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [peer],
        "the callback ran once"
    );
}

/// The application close a refused dial carries, so a test knows the refusal came from the peer.
/// The gate refuses after the handshake, so the dial may succeed and the connection then close.
async fn refusal_of(dialed: anyhow::Result<Connection>) -> (u64, Vec<u8>) {
    let reason = match dialed {
        Ok(conn) => with_timeout(conn.closed()).await,
        Err(e) => error_close_reason(&e)
            .cloned()
            .unwrap_or_else(|| panic!("the dial failed before the peer answered: {e:#}")),
    };
    match reason {
        ConnectionError::ApplicationClosed(close) => {
            (close.error_code.into_inner(), close.reason.to_vec())
        }
        other => panic!("the peer did not refuse with an application close: {other:?}"),
    }
}

fn delete_row(db: &Database, peer: &str) {
    db.with_conn(|c| {
        c.execute("DELETE FROM bridge_peers WHERE id = ?1", [peer])?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn hub_revoking_the_machine_while_connected_cleans_the_machine_and_runs_callback() {
    let (hub, machine, mine) = paired_and_set_up().await;
    let hub_port = hub.row(&machine).unwrap().unwrap().local_port;
    assert!(settles(mine.local_port, true).await);
    assert!(settles(hub_port, true).await);
    let seen = count_callbacks(&machine);

    assert!(hub.bridge.revoke(&machine.bridge.id()).await.unwrap());

    // The machine learns of it from the closed connection.
    let cleaned = wait_for(|| {
        machine.row(&hub).as_ref().is_ok_and(|r| r.is_none())
            && !machine.gate_lines().contains(&gate_label(&hub.id()))
            && !seen.lock().unwrap().is_empty()
    })
    .await;
    assert!(cleaned, "machine did not learn of the revoke in time");
    assert!(!peers::is_paired(&machine.db, &hub.id()).unwrap());
    assert!(refuses(mine.local_port).await, "machine forward closed");
    ran_once(&seen, &hub.id()).await;
}

#[tokio::test]
async fn machine_revoking_the_hub_while_connected_cleans_the_hub_and_runs_callback() {
    let (hub, machine, mine) = paired_and_set_up().await;
    let hub_port = hub.row(&machine).unwrap().unwrap().local_port;
    let mcp_key = mine.mcp_key.clone().unwrap();
    assert!(settles(mine.local_port, true).await);
    assert!(settles(hub_port, true).await);
    let gate_key_file = hub
        .dir
        .path()
        .join("gate")
        .join(format!("{}.key", machine.name));
    assert!(gate_key_file.exists(), "setup made the gate key");
    // Only counts: the bridge's own revoke has to delete the gate key.
    let seen = count_callbacks(&hub);

    assert!(machine.bridge.revoke(&hub.bridge.id()).await.unwrap());

    let cleaned = wait_for(|| {
        hub.row(&machine).as_ref().is_ok_and(|r| r.is_none())
            && !rows::device_key_live(&hub.db, &mcp_key).unwrap()
            && !gate_key_file.exists()
            && !seen.lock().unwrap().is_empty()
    })
    .await;
    assert!(cleaned, "hub did not clean up in time");
    assert!(refuses(hub_port).await, "hub forward closed");
    ran_once(&seen, &machine.id()).await;
}

#[tokio::test]
async fn offline_revoke_cleans_up_on_first_dial() {
    let (hub, machine, mine) = paired_and_set_up().await;
    let hub_port = hub.row(&machine).unwrap().unwrap().local_port;
    assert!(settles(mine.local_port, true).await);
    assert!(settles(hub_port, true).await);

    let (machine_id, kept, role, name) = machine.stop().await;
    assert!(hub.bridge.revoke(&machine_id).await.unwrap());

    // The machine starts again with its row, and learns at its first dial.
    let machine = Node::start(role, name, true, Some(kept)).await;
    assert_eq!(machine_id, machine.bridge.id());
    assert!(machine.row(&hub).unwrap().is_some(), "row before the dial");
    let seen = count_callbacks(&machine);
    let refused = machine.bridge.connect_service(&hub.id()).await;
    assert_eq!(refusal_of(refused).await, (403, b"revoked".to_vec()));

    let cleaned = wait_for(|| {
        machine.row(&hub).as_ref().is_ok_and(|r| r.is_none())
            && !machine.gate_lines().contains(&gate_label(&hub.id()))
            && !seen.lock().unwrap().is_empty()
    })
    .await;
    assert!(cleaned, "machine did not clean up on its first dial");
    assert!(refuses(mine.local_port).await, "machine forward closed");
    ran_once(&seen, &hub.id()).await;
}

#[tokio::test]
async fn a_refusal_other_than_revoked_never_revokes() {
    let (hub, machine, mine) = paired_and_set_up().await;
    assert!(settles(mine.local_port, true).await);
    let seen = count_callbacks(&machine);

    // Edge-style: the hub's gate turns a paired peer away from the edge protocol.
    let refused = machine
        .bridge
        .endpoint()
        .connect(hub.bridge.endpoint().addr(), EDGE_ALPN)
        .await
        .map_err(anyhow::Error::from);
    assert_eq!(refusal_of(refused).await, (403, b"not the edge".to_vec()));

    // The hub lost its row (restored from an older backup): it says "not paired", end to end.
    delete_row(&hub.db, &machine.id());
    let refused = machine.bridge.connect_service(&hub.id()).await;
    assert_eq!(refusal_of(refused).await, (403, b"not paired".to_vec()));

    tokio::time::sleep(SETTLE).await;
    assert!(seen.lock().unwrap().is_empty(), "no learned revoke");
    assert!(peers::is_paired(&machine.db, &hub.id()).unwrap());
    assert!(machine.row(&hub).unwrap().is_some(), "setup row intact");
    assert!(machine.gate_lines().contains(&gate_label(&hub.id())));
}

#[tokio::test]
async fn a_hub_that_cannot_read_its_list_says_unavailable_and_nothing_is_revoked() {
    let (hub, machine, _mine) = paired_and_set_up().await;
    let seen = count_callbacks(&machine);
    hub.db
        .with_conn(|c| {
            c.execute_batch("ALTER TABLE bridge_peers RENAME TO bridge_peers_away")?;
            Ok(())
        })
        .unwrap();

    let refused = machine.bridge.connect_service(&hub.id()).await;
    assert_eq!(refusal_of(refused).await, (503, b"unavailable".to_vec()));

    tokio::time::sleep(SETTLE).await;
    assert!(seen.lock().unwrap().is_empty(), "no learned revoke");
    assert!(peers::is_paired(&machine.db, &hub.id()).unwrap());
    assert!(machine.row(&hub).unwrap().is_some(), "setup row intact");
}

#[tokio::test]
async fn stranger_cannot_make_us_revoke_a_third_peer() {
    let (hub, machine, _mine) = paired_and_set_up().await;
    let stranger = Node::hub().await;
    let before = peers::list(&machine.db, true).unwrap();
    // The stranger knows how to reach the machine, so its dial really leaves it.
    let known = peers::get(&hub.db, &machine.id()).unwrap().unwrap();
    peers::add_peer(
        &stranger.db,
        &machine.id(),
        "laptop",
        &known.relays,
        &known.addrs,
    )
    .unwrap();

    let refused = stranger.bridge.connect_service(&machine.id()).await;
    assert_eq!(refusal_of(refused).await, (403, b"not paired".to_vec()));

    assert_eq!(
        peers::list(&machine.db, true).unwrap(),
        before,
        "list unchanged"
    );
    assert!(machine.row(&hub).unwrap().is_some(), "hub setup row intact");
    assert!(
        peers::is_paired(&stranger.db, &machine.id()).unwrap(),
        "\"not paired\" made the stranger revoke nothing either"
    );
}

#[tokio::test]
async fn a_dialed_connection_closes_when_its_last_handle_drops() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open_memory().unwrap();
    let dialer = Bridge::start(
        BridgeConfig {
            key: dir.path().join("bridge.key"),
            bind: vec![LOOPBACK.parse().unwrap()],
            relay: RelayMode::Disabled,
            name: "laptop".into(),
            with_addrs: true,
            advertise: vec![],
        },
        db.clone(),
        None,
    )
    .await
    .unwrap();
    let bare = Endpoint::builder(presets::Minimal)
        .secret_key(SecretKey::generate())
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .alpns(vec![SVC_ALPN.to_vec()])
        .bind_addr(LOOPBACK.parse::<SocketAddr>().unwrap())
        .unwrap()
        .bind()
        .await
        .unwrap();
    let addrs: Vec<String> = bare.bound_sockets().iter().map(|a| a.to_string()).collect();
    peers::add_peer(&db, &bare.id().to_string(), "bare", &[], &addrs).unwrap();

    let accepted = tokio::spawn({
        let bare = bare.clone();
        async move { bare.accept().await.unwrap().await.unwrap() }
    });
    let conn = with_timeout(dialer.connect_service(&bare.id().to_string()))
        .await
        .unwrap();
    let theirs = with_timeout(accepted).await.unwrap();
    drop(conn);

    // The revoke watcher holds the connection weakly, so the peer hears it close now.
    let reason = with_timeout(theirs.closed()).await;
    assert!(
        matches!(reason, ConnectionError::ApplicationClosed(_)),
        "{reason:?}"
    );
}
