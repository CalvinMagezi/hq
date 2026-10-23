//! The whole path Claude's servers take to a device, on loopback with only the CA and the network
//! stand-ins: the real forwarder (`hq-edge`), the real device side (`hq-bridge`) and the real
//! connector door, joined by a unix socket. The name is the one the device would register under
//! (its key's label under the edge's domain), the certificate is ordered through the forwarder,
//! and the client is a plain TLS client that trusts only the test CA.

use super::support::{
    Reply, Target, World, build_tls_for, client_config, connect, exchange, world_at,
};
use super::test_ca::TestCa;
use crate::tls::{ConnectorTls, TlsState};
use hq_bridge::{Bridge, BridgeConfig, EdgeDoor, RelayMode, device_label};
use hq_core::config::{DoorListen, DoorTarget};
use hq_db::{Database, bridge as peers};
use hq_edge::{
    AllowlistFile, Edge, EdgeConfig, Registered, RelayHost, RelayParams, RelayTls, allow_device,
    load_or_create_key,
};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const DOMAIN: &str = "edge.test";
const RELAY_HOSTNAME: &str = "relay.edge.test";
const GIVE_UP: Duration = Duration::from_secs(45);
const GONE: Duration = Duration::from_secs(20);

struct Chain {
    edge: Edge,
    bridge: Bridge,
    ca: Arc<TestCa>,
    tls: Arc<ConnectorTls>,
    hostname: String,
    serving: tokio::task::JoinHandle<()>,
    /// The web server's vault holds the door's socket, so it lives as long as the chain.
    _world: World,
    _dirs: (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir),
}

impl Chain {
    /// The door orders its own certificate as soon as it serves; waits for it to be valid.
    async fn has_a_certificate(&self) {
        eventually("no certificate after the order", || {
            self.tls.status().state == TlsState::Valid
        })
        .await;
    }

    /// One GET to the edge's public port, as Claude's servers would send it to the device's name.
    async fn get(&self, path: &str) -> Reply {
        self.send("GET", path, "").await
    }

    async fn send(&self, method: &str, path: &str, body: &str) -> Reply {
        exchange(
            Target {
                addr: self.edge.local_addr(),
                root: &self.ca.root,
                sni: &self.hostname,
            },
            method,
            Some(&self.hostname),
            path,
            &[("Content-Type", "application/json")],
            body,
        )
        .await
    }
}

async fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ready() {
        assert!(started.elapsed() < GIVE_UP, "{what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// An edge with one registered device, that device's bridge with the edge door, and the web
/// server's door behind it, with no certificate ordered yet.
async fn chain() -> Chain {
    let edge_dir = tempfile::tempdir().unwrap();
    let device_dir = tempfile::tempdir().unwrap();

    let device_key = hq_bridge::key::load_or_create_key(&device_dir.path().join("bridge.key"))
        .expect("the device key");
    let device_id = device_key.public();
    let hostname = format!("{}.{DOMAIN}", device_label(&device_id));

    let list = edge_dir.path().join("devices.txt");
    allow_device(
        &list,
        Registered {
            id: device_id,
            relay: None,
        },
    )
    .unwrap();
    let edge_key = load_or_create_key(&edge_dir.path().join("edge.key")).unwrap();
    let edge_id = edge_key.public();
    let registry = Arc::new(AllowlistFile::open(&list).unwrap());
    let relay = RelayHost::spawn(
        RelayParams {
            hostname: RELAY_HOSTNAME.into(),
            tls: RelayTls::PlainHttp,
            quic_bind: None,
        },
        registry.clone(),
        edge_id,
    )
    .await
    .unwrap();
    let relay_url = relay.url().clone();
    let edge = Edge::start_with(
        EdgeConfig {
            enabled: true,
            domain: DOMAIN.into(),
            listen: "127.0.0.1:0".parse().unwrap(),
            dir: edge_dir.path().to_path_buf(),
            iroh_bind: "127.0.0.1:0".parse().unwrap(),
            ..EdgeConfig::default()
        },
        registry,
        edge_key,
        Some(relay),
    )
    .await
    .unwrap();

    let world = world_at(&hostname);
    let DoorTarget::Unix(socket) = DoorListen::Default.resolve(world._vault.path()) else {
        panic!("the default door is a unix socket")
    };
    let db = Database::open_memory().unwrap();
    peers::edge_set(&db, &edge_id.to_string(), DOMAIN, &hostname, None).unwrap();
    let bridge = Bridge::start_with_edge_door(
        BridgeConfig {
            key: device_dir.path().join("bridge.key"),
            bind: vec!["127.0.0.1:0".parse().unwrap()],
            relay: RelayMode::custom([relay_url]),
            name: "phone".into(),
            with_addrs: false,
            advertise: vec![],
        },
        db,
        None,
        Some(EdgeDoor::new(hq_bridge::DoorTarget::Unix(socket.clone()))),
    )
    .await
    .unwrap();

    let ca = TestCa::start(edge.local_addr()).await;
    let store_dir = tempfile::tempdir().unwrap();
    let tls = Arc::new(build_tls_for(&hostname, &world, &ca, &store_dir, |_| {}));
    let serving = tokio::spawn(crate::tls::listener::serve_door(
        tls.clone(),
        DoorTarget::Unix(socket.clone()),
    ));
    eventually("the door never bound its socket", || socket.exists()).await;
    Chain {
        edge,
        bridge,
        ca,
        tls,
        hostname,
        serving,
        _world: world,
        _dirs: (edge_dir, device_dir, store_dir),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_certificate_is_ordered_through_the_forwarder_and_the_connector_answers_over_it() {
    let chain = chain().await;

    // The CA dials the edge's public port with the device's name: the validation travels the same
    // path as Claude's calls, through the forwarder and the device's bridge to the door.
    chain.has_a_certificate().await;
    let stats = chain.ca.stats();
    assert_eq!(
        (stats.orders, stats.validations_ok, stats.certificates),
        (1, 1, 1)
    );

    let protected = chain.get("/.well-known/oauth-protected-resource").await;
    assert_eq!(protected.status, 200, "{protected:?}");
    let doc: serde_json::Value = serde_json::from_str(&protected.body).unwrap();
    assert_eq!(doc["resource"], format!("https://{}/mcp", chain.hostname));
    assert_eq!(
        doc["authorization_servers"][0],
        format!("https://{}", chain.hostname)
    );

    let server = chain.get("/.well-known/oauth-authorization-server").await;
    assert_eq!(server.status, 200, "{server:?}");
    let meta: serde_json::Value = serde_json::from_str(&server.body).unwrap();
    assert_eq!(meta["issuer"], format!("https://{}", chain.hostname));
    // The consent page is on the private origin, never on the name the internet reaches.
    assert!(
        !meta["authorization_endpoint"]
            .as_str()
            .unwrap()
            .contains(&chain.hostname),
        "{meta}"
    );

    let challenge = chain.send("POST", "/mcp", "{}").await;
    assert_eq!(challenge.status, 401, "{challenge:?}");
    let www = challenge.header("www-authenticate").unwrap_or_default();
    assert!(
        www.contains(&format!("https://{}/.well-known", chain.hostname)),
        "{challenge:?}"
    );

    // Nothing else of the server is on the name: the consent page, the owner's API, the web UI.
    for path in ["/oauth/authorize", "/api/oauth/connector", "/", "/connect"] {
        let reply = chain.get(path).await;
        assert_eq!(reply.status, 404, "{path}: {reply:?}");
    }

    assert!(chain.bridge.reached_edge.load(Ordering::SeqCst) >= 1);
    assert_eq!(chain.bridge.reached_service.load(Ordering::SeqCst), 0);
    chain.serving.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_forgets_its_edge_stops_answering_through_it() {
    let chain = chain().await;
    chain.has_a_certificate().await;
    assert_eq!(
        chain
            .get("/.well-known/oauth-protected-resource")
            .await
            .status,
        200
    );

    let forgot = chain.bridge.forget_edge().await.unwrap();
    assert!(forgot.had_record);

    // The edge may hold a connection it has not noticed is closed; it redials, and the device
    // refuses an edge it has no record of, so the handshake never completes.
    let served_before = chain.bridge.reached_edge.load(Ordering::SeqCst);
    let config = client_config(&chain.ca.root, &[b"http/1.1"]);
    let handshake = tokio::time::timeout(
        GONE,
        connect(chain.edge.local_addr(), &chain.hostname, config),
    )
    .await;
    assert!(
        !matches!(handshake, Ok(Ok(_))),
        "the edge still reached the door after the device forgot it"
    );
    assert_eq!(
        chain.bridge.reached_edge.load(Ordering::SeqCst),
        served_before,
        "no further edge connection was served"
    );
    chain.serving.abort();
}
