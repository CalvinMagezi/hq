//! Setup after pairing, end to end on loopback: real iroh endpoints, no relay, the real migrated
//! database and the real gate key files under a temporary directory. The phone is the hub (it
//! offers `web`), the laptop is the machine (it offers `gate`); each "service" is a TCP echo
//! server standing in for the paired-devices door or the host gate.
#![cfg(unix)]

use hq_bridge::{
    Bridge, BridgeConfig, DoorTarget, JoinResult, Role, SETUP_ALPN, Services, SetupConfig, Ticket,
    Tuning,
    setup::{MAX_SETUP_BYTES, SVC_GATE, SVC_WEB, gate_label},
};
use hq_db::{Database, bridge as peers, bridge_setup as rows};
use iroh::{EndpointAddr, RelayMode};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const LOOPBACK: &str = "127.0.0.1:0";
const GIVE_UP: Duration = Duration::from_secs(20);
/// Longer than the forwards' reconcile interval.
const SETTLE: Duration = Duration::from_secs(8);

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

    /// The same machine started again (same key, same database), offering its gate this time.
    async fn restart_offering(self) -> Self {
        let Node {
            bridge,
            db,
            dir,
            role,
            name,
            service,
            ..
        } = self;
        bridge.shutdown().await;
        Self::start(role, name, true, Some((db, dir, service))).await
    }

    fn id(&self) -> String {
        self.bridge.id().to_string()
    }

    fn row(&self, peer: &Node) -> Option<rows::Setup> {
        rows::get(&self.db, &peer.id()).unwrap()
    }

    fn knows(&self, other: &Node) {
        let card = other.bridge.card();
        peers::add_peer(&self.db, &card.id, &card.name, &card.relays, &card.addrs).unwrap();
    }

    fn gate_lines(&self) -> String {
        std::fs::read_to_string(authorized(self.dir.path())).unwrap_or_default()
    }
}

/// What a restart keeps: the database, the directory with the key, the service's address.
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

/// Waits until `port` echoes (`want`) or stops echoing (`!want`).
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

/// Waits until nothing listens on `port` any more.
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

/// A real pairing: the hub shows the ticket, the machine scans it, the hub types the digits.
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
async fn pairing_then_setup_wires_both_sides_with_no_config() {
    let (hub, machine, mine) = paired_and_set_up().await;

    // The hub: a host entry for the machine and a forward that reaches its gate.
    let theirs = hub.row(&machine).expect("the hub recorded the machine");
    assert_eq!(theirs.peer_role, rows::ROLE_MACHINE);
    assert_eq!(theirs.peer_name, "laptop");
    assert!(theirs.peer_offers, "the machine offered its gate");
    assert_eq!(
        theirs.peer_port,
        Some(mine.local_port),
        "the agents' MCP port"
    );
    assert!(
        settles(theirs.local_port, true).await,
        "the hub reaches the gate"
    );

    // The machine: a forward that reaches the hub's web, and the key minted for it.
    assert_eq!(mine.peer_role, rows::ROLE_HUB);
    assert_eq!(mine.peer_name, "phone");
    assert!(mine.peer_offers);
    assert!(
        settles(mine.local_port, true).await,
        "the machine reaches the web"
    );
    let key = mine.mcp_key.clone().expect("a stored key");
    assert!(rows::device_key_live(&hub.db, &key).unwrap());

    // The hub's gate key for this host is authorized on the machine, from loopback only.
    let gate_key = hq_gate::ClientKey::load(&hub.dir.path().join("gate/laptop.key"))
        .unwrap()
        .public_key();
    let line = format!("{gate_key} from=127.0.0.1 # {}", gate_label(&hub.id()));
    assert_eq!(machine.gate_lines().trim(), line);
}

#[tokio::test]
async fn running_setup_again_keeps_the_ports_and_mints_no_second_key() {
    let (hub, machine, first) = paired_and_set_up().await;
    let hub_first = hub.row(&machine).unwrap();
    let again = with_timeout(machine.bridge.set_up(&hub.id()))
        .await
        .unwrap();
    assert!(!again.sent, "a setup in place sends nothing");
    assert_eq!(again.setup, first);

    // A message on the wire for a peer the hub already set up (here: a machine that offers its
    // gate only now) keeps every port and the one live key.
    let mut quiet = Node::start(Role::Machine, "laptop", false, None).await;
    quiet.knows(&hub);
    hub.knows(&quiet);
    let before = with_timeout(quiet.bridge.set_up(&hub.id())).await.unwrap();
    assert!(!hub.row(&quiet).unwrap().peer_offers);
    quiet = quiet.restart_offering().await;
    let after = with_timeout(quiet.bridge.set_up(&hub.id())).await.unwrap();
    assert!(after.sent, "offering the gate now runs setup again");
    assert_eq!(after.setup.local_port, before.setup.local_port);
    assert_eq!(
        after.setup.mcp_key, before.setup.mcp_key,
        "the machine keeps its key"
    );
    let row = hub.row(&quiet).unwrap();
    assert!(row.peer_offers);
    assert_eq!(row.peer_port, Some(before.setup.local_port));
    assert_eq!(rows::keys_minted(&hub.db, &quiet.id()).unwrap(), 1);
    assert_eq!(rows::keys_minted(&hub.db, &machine.id()).unwrap(), 1);
    assert_eq!(hub.row(&machine).unwrap(), hub_first);
}

#[tokio::test]
async fn revoking_on_the_machine_undoes_its_side() {
    let (hub, machine, mine) = paired_and_set_up().await;
    let hub_port = hub.row(&machine).unwrap().local_port;
    assert!(settles(mine.local_port, true).await);
    assert!(settles(hub_port, true).await);

    assert!(machine.bridge.revoke(&hub.bridge.id()).await.unwrap());
    assert_eq!(machine.row(&hub), None, "the setup row is gone");
    assert_eq!(machine.gate_lines().trim(), "", "the gate line is gone");
    assert!(
        refuses(mine.local_port).await,
        "the machine's forward is closed"
    );
    assert!(
        settles(hub_port, false).await,
        "the hub no longer reaches the gate"
    );
}

#[tokio::test]
async fn revoking_on_the_hub_undoes_its_side() {
    let (hub, machine, mine) = paired_and_set_up().await;
    let hub_port = hub.row(&machine).unwrap().local_port;
    let key = mine.mcp_key.clone().unwrap();
    assert!(settles(hub_port, true).await);
    assert!(settles(mine.local_port, true).await);

    assert!(hub.bridge.revoke(&machine.bridge.id()).await.unwrap());
    assert_eq!(hub.row(&machine), None, "the setup row is gone");
    assert!(
        !rows::device_key_live(&hub.db, &key).unwrap(),
        "the key is revoked"
    );
    assert!(refuses(hub_port).await, "the hub's forward is closed");
    assert!(
        settles(mine.local_port, false).await,
        "the machine no longer reaches the web"
    );
}

/// A revoke made by another process (`hq bridge revoke`) changes only the database; the running
/// bridge's forwards task closes the forward and removes the gate line on its own.
#[tokio::test]
async fn a_revoke_from_another_process_is_swept_by_the_running_bridge() {
    let (hub, machine, mine) = paired_and_set_up().await;
    assert!(settles(mine.local_port, true).await);
    peers::revoke(&machine.db, &hub.id()).unwrap();
    assert!(refuses(mine.local_port).await);
    let end = tokio::time::Instant::now() + SETTLE;
    while !machine.gate_lines().trim().is_empty() && tokio::time::Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(machine.gate_lines().trim(), "");
}

#[tokio::test]
async fn a_stranger_gets_nothing_and_two_machines_set_nothing_up() {
    let hub = Node::hub().await;
    let stranger = Node::machine().await;
    // The stranger can dial the hub, but the hub never paired it.
    stranger.knows(&hub);
    assert!(
        with_timeout(stranger.bridge.set_up(&hub.id()))
            .await
            .is_err()
    );
    assert_eq!(hub.row(&stranger), None);
    assert_eq!(rows::keys_minted(&hub.db, &stranger.id()).unwrap(), 0);
    assert_eq!(stranger.row(&hub), None);
    assert_eq!(stranger.gate_lines(), "");

    let (a, b) = (Node::machine().await, Node::machine().await);
    a.knows(&b);
    b.knows(&a);
    assert!(with_timeout(a.bridge.set_up(&b.id())).await.is_err());
    assert_eq!((a.row(&b), b.row(&a)), (None, None));
    assert_eq!(a.gate_lines(), "");
}

/// A paired peer that sends a message too long, with a field it may not set or of another
/// version gets no answer, and nothing is stored or minted.
#[tokio::test]
async fn a_bad_message_from_a_paired_peer_sets_nothing_up() {
    let hub = Node::hub().await;
    let machine = Node::machine().await;
    hub.knows(&machine);
    machine.knows(&hub);
    let good = serde_json::json!({
        "v": 1, "role": "machine", "name": "laptop", "gate": true, "mcp_port": 47_500,
    });
    let mut extra = good.clone();
    extra["listen"] = "0.0.0.0:22".into();
    let mut newer = good.clone();
    newer["v"] = 2.into();
    let mut hub_role = good.clone();
    hub_role["role"] = "hub".into();
    let oversized = vec![b'x'; MAX_SETUP_BYTES + 1];
    let bodies = [
        serde_json::to_vec(&extra).unwrap(),
        serde_json::to_vec(&newer).unwrap(),
        serde_json::to_vec(&hub_role).unwrap(),
        oversized,
    ];
    for body in bodies {
        assert!(raw_setup(&machine, &hub, &body).await.is_none());
    }
    assert_eq!(hub.row(&machine), None);
    assert_eq!(rows::keys_minted(&hub.db, &machine.id()).unwrap(), 0);
    // The control: the same peer, a good message, gets an answer.
    assert!(
        raw_setup(&machine, &hub, &serde_json::to_vec(&good).unwrap())
            .await
            .is_some()
    );
    assert!(hub.row(&machine).is_some());
}

/// Speaks `hq/setup/1` by hand and returns the answer's bytes, if any.
async fn raw_setup(from: &Node, to: &Node, body: &[u8]) -> Option<Vec<u8>> {
    let mut addr = EndpointAddr::new(to.bridge.id());
    for a in &to.bridge.card().addrs {
        addr = addr.with_ip_addr(a.parse().unwrap());
    }
    let conn = with_timeout(from.bridge.endpoint().connect(addr, SETUP_ALPN))
        .await
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    let len = u16::try_from(body.len()).unwrap();
    send.write_all(&len.to_be_bytes()).await.ok()?;
    send.write_all(body).await.ok()?;
    send.finish().ok()?;
    let mut head = [0u8; 2];
    let read = tokio::time::timeout(Duration::from_secs(5), recv.read_exact(&mut head)).await;
    let answer = match read {
        Ok(Ok(())) => {
            let mut reply = vec![0u8; usize::from(u16::from_be_bytes(head))];
            recv.read_exact(&mut reply).await.ok().map(|_| reply)
        }
        _ => None,
    };
    conn.close(0u32.into(), b"done");
    answer
}
