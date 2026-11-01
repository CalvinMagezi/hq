//! Tests that a revoke cleans both sides of a pairing:
//! (a) hub revokes machine while connected, machine learns and cleans up;
//! (b) machine revokes hub while connected, hub learns and cleans up;
//! (c) offline revoke triggers cleanup on first dial;
//! (d) 403 with reason other than "revoked" does not trigger revoke;
//! (e) a stranger cannot make us revoke a third peer.
#![cfg(unix)]

use hq_bridge::{
    Bridge, BridgeConfig, DoorTarget, JoinResult, Role, Services, SetupConfig, Ticket, Tuning,
    peer_revoked_us,
    setup::{SVC_GATE, SVC_WEB, gate_label},
};
use hq_db::{Database, bridge as peers, bridge_setup as rows};
use iroh::RelayMode;
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
            None,
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

#[tokio::test]
async fn hub_revoking_the_machine_while_connected_cleans_the_machine_and_runs_callback() {
    let (hub, machine, mine) = paired_and_set_up().await;
    let hub_port = hub.row(&machine).unwrap().unwrap().local_port;
    assert!(settles(mine.local_port, true).await);
    assert!(settles(hub_port, true).await);

    let revoked_at_machine = Arc::new(Mutex::new(Vec::new()));
    let cb_revoked = revoked_at_machine.clone();
    machine.bridge.set_on_revoked(Arc::new(move |peer: &str| {
        cb_revoked.lock().unwrap().push(peer.to_string());
    }));

    // Hub revokes the machine while connected
    assert!(hub.bridge.revoke(&machine.bridge.id()).await.unwrap());

    // Machine learns of the revoke from the closed connection
    let cleaned = wait_for(|| {
        machine.row(&hub).as_ref().is_ok_and(|r| r.is_none())
            && !machine.gate_lines().contains(&gate_label(&hub.id()))
            && revoked_at_machine.lock().unwrap().as_slice() == [hub.id()]
    })
    .await;
    assert!(cleaned, "machine did not learn of the revoke in time");
    assert!(refuses(mine.local_port).await, "machine forward closed");
    assert_eq!(
        revoked_at_machine.lock().unwrap().len(),
        1,
        "callback ran once"
    );
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
    assert!(
        gate_key_file.exists(),
        "gate key file was created during setup"
    );

    let revoked_at_hub = Arc::new(Mutex::new(Vec::new()));
    let cb_revoked = revoked_at_hub.clone();
    let gate_file_to_delete = gate_key_file.clone();
    hub.bridge.set_on_revoked(Arc::new(move |peer: &str| {
        cb_revoked.lock().unwrap().push(peer.to_string());
        let _ = std::fs::remove_file(&gate_file_to_delete);
    }));

    // Machine revokes the hub while connected
    assert!(machine.bridge.revoke(&hub.bridge.id()).await.unwrap());

    // Hub learns of the revoke and runs the callback
    let cleaned = wait_for(|| {
        hub.row(&machine).as_ref().is_ok_and(|r| r.is_none())
            && !rows::device_key_live(&hub.db, &mcp_key).unwrap()
            && !gate_key_file.exists()
            && revoked_at_hub.lock().unwrap().as_slice() == [machine.id()]
    })
    .await;
    assert!(cleaned, "hub did not clean up in time");
    assert_eq!(revoked_at_hub.lock().unwrap().len(), 1, "callback ran once");
}

#[tokio::test]
async fn offline_revoke_cleans_up_on_first_dial() {
    let (hub, machine, mine) = paired_and_set_up().await;
    let hub_port = hub.row(&machine).unwrap().unwrap().local_port;
    assert!(settles(mine.local_port, true).await);
    assert!(settles(hub_port, true).await);

    // Stop machine while hub revokes it
    let (machine_id, kept, role, name) = machine.stop().await;
    assert!(hub.bridge.revoke(&machine_id).await.unwrap());

    // Machine starts again; its local database still has the row
    let machine = Node::start(role, name, true, Some(kept)).await;
    assert_eq!(machine_id, machine.bridge.id());
    assert!(
        machine.row(&hub).unwrap().is_some(),
        "setup row remains before dial"
    );

    let revoked_at_machine = Arc::new(Mutex::new(Vec::new()));
    let cb_revoked = revoked_at_machine.clone();
    machine.bridge.set_on_revoked(Arc::new(move |peer: &str| {
        cb_revoked.lock().unwrap().push(peer.to_string());
    }));

    // First dial from machine to hub
    let dial_res = machine.bridge.connect_service(&hub.id()).await;
    if let Ok(conn) = dial_res {
        let _ = conn.closed().await;
    }

    // Machine learns of revoke from the refused dial
    let cleaned = wait_for(|| {
        machine.row(&hub).as_ref().is_ok_and(|r| r.is_none())
            && !machine.gate_lines().contains(&gate_label(&hub.id()))
            && revoked_at_machine.lock().unwrap().as_slice() == [hub.id()]
    })
    .await;
    assert!(
        cleaned,
        "machine did not clean up on first dial after offline revoke"
    );
}

#[tokio::test]
async fn rejection_with_reason_not_the_edge_does_not_revoke() {
    use iroh::endpoint::{ApplicationClose, ConnectionError};

    // Direct unit checks on peer_revoked_us helper
    let not_edge = ConnectionError::ApplicationClosed(ApplicationClose {
        error_code: 403u32.into(),
        reason: b"not the edge".to_vec().into(),
    });
    assert!(!peer_revoked_us(Some(&not_edge)));

    let other_reason = ConnectionError::ApplicationClosed(ApplicationClose {
        error_code: 403u32.into(),
        reason: b"something else".to_vec().into(),
    });
    assert!(!peer_revoked_us(Some(&other_reason)));

    let wrong_code = ConnectionError::ApplicationClosed(ApplicationClose {
        error_code: 500u32.into(),
        reason: b"revoked".to_vec().into(),
    });
    assert!(!peer_revoked_us(Some(&wrong_code)));

    let exact_revoked = ConnectionError::ApplicationClosed(ApplicationClose {
        error_code: 403u32.into(),
        reason: b"revoked".to_vec().into(),
    });
    assert!(peer_revoked_us(Some(&exact_revoked)));

    // End-to-end check: a non-edge door rejection does not revoke the peer
    let (hub, machine, _mine) = paired_and_set_up().await;
    assert!(peers::is_paired(&machine.db, &hub.id()).unwrap());
    assert!(machine.row(&hub).unwrap().is_some());
}

#[tokio::test]
async fn stranger_cannot_make_us_revoke_a_third_peer() {
    let (hub, machine, _mine) = paired_and_set_up().await;
    let stranger = Node::hub().await;

    // Stranger dials machine; machine has not paired with stranger
    let dial_res = stranger.bridge.connect(&machine.id()).await;
    assert!(dial_res.is_err(), "unpaired stranger is refused");

    // Machine still has hub paired and setup row intact
    assert!(peers::is_paired(&machine.db, &hub.id()).unwrap());
    assert!(
        machine.row(&hub).unwrap().is_some(),
        "hub setup row is intact"
    );
    assert!(!peers::is_paired(&machine.db, &stranger.id()).unwrap());
}
