//! The long-term side of the bridge: this machine's key, the paired-only gate, the service that
//! paired peers reach by key alone, and revocation that reaches connections already open.
//!
//! The long-term endpoint never speaks the pairing protocol. Pairing runs on a throwaway endpoint
//! per ticket (`host.rs`); this endpoint serves one protocol, to peers on the paired list.

use crate::host::{self, OpenTicket, PairingSpec, Registrar, Tuning};
use crate::join::{self, JoinOptions, JoinResult, target_from};
use crate::key::load_or_create_key;
use crate::ticket::{Purpose, Ticket};
use crate::wire::Card;
use anyhow::{Context, Result, anyhow, bail};
use hq_db::{Database, bridge as peers};
use iroh::{
    Endpoint, EndpointId, RelayMode,
    endpoint::{
        AfterHandshakeOutcome, Connection, EndpointHooks, RecvStream, SendStream,
        WeakConnectionHandle, presets,
    },
    protocol::{AcceptError, ProtocolHandler, Router},
};
use std::{
    collections::HashMap,
    future::Future,
    net::SocketAddr,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

/// What paired peers speak to each other after pairing.
pub const SERVICE_ALPN: &[u8] = b"hq/bridge/1";
const REJECT_UNPAIRED: u32 = 403;
const REJECT_REVOKED: u32 = 403;
const MAX_ECHO_BYTES: usize = 64 * 1024;
const ONLINE_WAIT: Duration = Duration::from_secs(5);

/// One request from a paired peer: a bidirectional stream, already checked against the list.
pub struct PeerStream {
    pub peer: EndpointId,
    pub send: SendStream,
    pub recv: RecvStream,
}

pub type StreamHandler =
    Arc<dyn Fn(PeerStream) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Echoes what it is sent: the smallest service, for tests and `hq bridge ping`.
pub fn echo_handler() -> StreamHandler {
    Arc::new(|mut stream: PeerStream| {
        Box::pin(async move {
            if let Ok(data) = stream.recv.read_to_end(MAX_ECHO_BYTES).await {
                let _ = stream.send.write_all(&data).await;
                let _ = stream.send.finish();
            }
        })
    })
}

pub struct BridgeConfig {
    /// The long-term key file (created with mode 0600 if missing).
    pub key: PathBuf,
    pub bind: Vec<SocketAddr>,
    pub relay: RelayMode,
    /// What this machine calls itself to the people it pairs with.
    pub name: String,
    /// Whether tickets and cards list this machine's IP addresses. Off, they hold only ids and
    /// relays, which is smaller and shows no address.
    pub with_addrs: bool,
    pub advertise: Vec<SocketAddr>,
}

/// Connections that are open right now, by the peer behind them, so a revoke can reach them.
#[derive(Default)]
struct Live {
    next: AtomicU64,
    conns: Mutex<HashMap<EndpointId, Vec<(u64, WeakConnectionHandle)>>>,
}

impl Live {
    fn register(&self, id: EndpointId, conn: WeakConnectionHandle) -> u64 {
        let token = self.next.fetch_add(1, Ordering::SeqCst);
        self.conns
            .lock()
            .unwrap()
            .entry(id)
            .or_default()
            .push((token, conn));
        token
    }

    fn unregister(&self, id: &EndpointId, token: u64) {
        let mut conns = self.conns.lock().unwrap();
        if let Some(list) = conns.get_mut(id) {
            list.retain(|(t, _)| *t != token);
            if list.is_empty() {
                conns.remove(id);
            }
        }
    }

    /// Closes every open connection of `id`. Returns how many were still alive.
    fn close_all(&self, id: &EndpointId) -> usize {
        let handles: Vec<WeakConnectionHandle> = self
            .conns
            .lock()
            .unwrap()
            .get(id)
            .map(|l| l.iter().map(|(_, c)| c.clone()).collect())
            .unwrap_or_default();
        handles
            .into_iter()
            .filter_map(|h| h.upgrade())
            .map(|c| c.close(REJECT_REVOKED.into(), b"revoked"))
            .count()
    }
}

struct LiveGuard {
    live: Arc<Live>,
    id: EndpointId,
    token: u64,
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.live.unregister(&self.id, self.token);
    }
}

/// Whether `id` is on the list. Any failure is "no": the gate fails closed.
async fn still_paired(db: &Database, id: EndpointId) -> bool {
    let db = db.clone();
    tokio::task::spawn_blocking(move || peers::is_paired(&db, &id.to_string()).unwrap_or(false))
        .await
        .unwrap_or(false)
}

#[derive(Clone)]
struct ServiceGate {
    db: Database,
}

impl std::fmt::Debug for ServiceGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServiceGate")
    }
}

impl EndpointHooks for ServiceGate {
    async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        if conn.side().is_client() {
            return AfterHandshakeOutcome::accept();
        }
        if conn.alpn() == SERVICE_ALPN && still_paired(&self.db, conn.remote_id()).await {
            return AfterHandshakeOutcome::accept();
        }
        AfterHandshakeOutcome::Reject {
            error_code: REJECT_UNPAIRED.into(),
            reason: b"not paired".to_vec(),
        }
    }
}

#[derive(Clone)]
struct Service {
    db: Database,
    live: Arc<Live>,
    handler: StreamHandler,
    reached: Arc<AtomicU64>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Service")
    }
}

impl ProtocolHandler for Service {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.reached.fetch_add(1, Ordering::SeqCst);
        let id = conn.remote_id();
        // Register first, then read the list: a revoke that lands between the gate and here
        // either finds this connection in the registry or is seen by the read below.
        let token = self.live.register(id, conn.weak_handle());
        let _guard = LiveGuard {
            live: self.live.clone(),
            id,
            token,
        };
        if !still_paired(&self.db, id).await {
            conn.close(REJECT_REVOKED.into(), b"revoked");
            return Ok(());
        }
        self.touch(id);
        while let Ok((send, recv)) = conn.accept_bi().await {
            if !still_paired(&self.db, id).await {
                conn.close(REJECT_REVOKED.into(), b"revoked");
                break;
            }
            tokio::spawn((self.handler)(PeerStream {
                peer: id,
                send,
                recv,
            }));
        }
        Ok(())
    }
}

impl Service {
    fn touch(&self, id: EndpointId) {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let _ = peers::touch(&db, &id.to_string());
        });
    }
}

/// Records a device the person accepted as a peer in the database.
pub struct PeerRegistrar {
    pub db: Database,
}

impl Registrar for PeerRegistrar {
    fn accept(&self, joiner: &host::Joiner) -> Result<Option<serde_json::Value>> {
        peers::add_peer(
            &self.db,
            &joiner.id.to_string(),
            &joiner.name,
            &joiner.relays,
            &joiner.addrs,
        )?;
        Ok(None)
    }
}

pub struct Bridge {
    endpoint: Endpoint,
    db: Database,
    live: Arc<Live>,
    name: String,
    relay: RelayMode,
    bind: Vec<SocketAddr>,
    with_addrs: bool,
    advertise: Vec<SocketAddr>,
    /// Connections that reached the service handler; the rest were closed at the gate.
    pub reached_service: Arc<AtomicU64>,
    _router: Option<Router>,
}

fn any_port(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    addrs.iter().map(|a| SocketAddr::new(a.ip(), 0)).collect()
}

impl Bridge {
    /// Binds this machine's long-term endpoint. With a `handler` it also serves paired peers; with
    /// none it only dials out, and the endpoint accepts no protocol at all.
    pub async fn start(
        cfg: BridgeConfig,
        db: Database,
        handler: Option<StreamHandler>,
    ) -> Result<Self> {
        let key = load_or_create_key(&cfg.key)?;
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .relay_mode(cfg.relay.clone())
            .clear_ip_transports()
            .hooks(ServiceGate { db: db.clone() });
        for addr in &cfg.bind {
            builder = builder.bind_addr(*addr)?;
        }
        let endpoint = builder.bind().await?;
        if !matches!(cfg.relay, RelayMode::Disabled) {
            let _ = tokio::time::timeout(ONLINE_WAIT, endpoint.online()).await;
        }
        let live = Arc::new(Live::default());
        let reached_service = Arc::new(AtomicU64::new(0));
        let router = handler.map(|handler| {
            Router::builder(endpoint.clone())
                .accept(
                    SERVICE_ALPN,
                    Service {
                        db: db.clone(),
                        live: live.clone(),
                        handler,
                        reached: reached_service.clone(),
                    },
                )
                .spawn()
        });
        Ok(Self {
            endpoint,
            db,
            live,
            name: cfg.name,
            relay: cfg.relay,
            bind: cfg.bind,
            with_addrs: cfg.with_addrs,
            advertise: cfg.advertise,
            reached_service,
            _router: router,
        })
    }

    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Who this machine is and how to dial it, as it tells a machine that pairs with it.
    pub fn card(&self) -> Card {
        Card {
            id: self.id().to_string(),
            name: self.name.clone(),
            relays: self
                .endpoint
                .addr()
                .relay_urls()
                .map(|r| r.to_string())
                .collect(),
            addrs: host::ticket_addrs(&self.endpoint, self.with_addrs, &self.advertise),
        }
    }

    /// Opens a pairing ticket on a throwaway endpoint made for it. The person at this machine
    /// answers the requests that arrive on `requests`. The ticket's link is `ticket.link()`.
    pub async fn open_ticket(&self, tuning: Tuning) -> Result<OpenTicket> {
        OpenTicket::open(PairingSpec {
            purpose: Purpose::Pair,
            card: self.card(),
            registrar: Arc::new(PeerRegistrar {
                db: self.db.clone(),
            }),
            bind: any_port(&self.bind),
            relay: self.relay.clone(),
            with_addrs: self.with_addrs,
            advertise: Vec::new(),
            tuning,
        })
        .await
    }

    /// Pairs with the machine that shows `ticket`, and on acceptance records it as a peer.
    pub async fn join(
        &self,
        ticket: &Ticket,
        tuning: &Tuning,
        show_sas: impl FnOnce(&str),
    ) -> Result<JoinResult> {
        if ticket.k != Purpose::Pair {
            bail!("this ticket registers a device with an edge; it does not pair two machines");
        }
        let card = self.card();
        let mut opts = JoinOptions::new(&self.name, card.relays, card.addrs);
        opts.hello_timeout = tuning.hello_timeout;
        opts.decision_timeout = tuning.decision_timeout;
        let result = join::join(&self.endpoint, ticket, &opts, show_sas).await?;
        if let JoinResult::Paired(joined) = &result {
            let db = self.db.clone();
            let card = joined.card.clone();
            tokio::task::spawn_blocking(move || {
                peers::add_peer(&db, &card.id, &card.name, &card.relays, &card.addrs)
            })
            .await
            .context("recording the host")??;
        }
        Ok(result)
    }

    /// Connects to a paired peer by key alone: no code, no ticket.
    pub async fn connect(&self, peer: &str) -> Result<Connection> {
        let db = self.db.clone();
        let id = peer.to_string();
        let record = tokio::task::spawn_blocking(move || peers::get(&db, &id))
            .await??
            .filter(|p| p.is_active())
            .ok_or_else(|| anyhow!("{peer} is not a paired peer"))?;
        let target = target_from(record.id.parse()?, &record.relays, &record.addrs)?;
        Ok(self.endpoint.connect(target, SERVICE_ALPN).await?)
    }

    /// Takes a peer off the list and closes every connection it has open, at once.
    ///
    /// Always sweeps, even when the list says the peer was already revoked: a revoke made from
    /// another process changes the list but cannot reach connections held here, and this is how
    /// they are closed. Returns whether the peer was active.
    pub async fn revoke(&self, peer: &EndpointId) -> Result<bool> {
        let db = self.db.clone();
        let id = peer.to_string();
        let changed = tokio::task::spawn_blocking(move || peers::revoke(&db, &id)).await??;
        self.live.close_all(peer);
        Ok(changed)
    }

    pub async fn shutdown(self) {
        if let Some(router) = &self._router {
            let _ = router.shutdown().await;
        }
        self.endpoint.close().await;
    }
}

/// Sends `message` on a new stream and checks it comes back unchanged.
pub async fn echo(conn: &Connection, message: &[u8]) -> Result<()> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(message).await?;
    send.finish()?;
    let got = recv.read_to_end(MAX_ECHO_BYTES).await?;
    if got != message {
        bail!("the echo came back different");
    }
    Ok(())
}
