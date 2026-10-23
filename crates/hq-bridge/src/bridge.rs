//! The long-term side of the bridge: this machine's key, the paired-only gate, the service that
//! paired peers reach by key alone, and revocation that reaches connections already open.
//!
//! The long-term endpoint never speaks the pairing protocol. Pairing runs on a throwaway endpoint
//! per ticket (`host.rs`); this endpoint serves one protocol, to peers on the paired list.

use crate::door::{EDGE_ALPN, EdgeDoor, EdgeService, LABEL_BYTES};
use crate::host::{self, OpenTicket, PairingSpec, Registrar, Tuning};
use crate::join::{self, JoinOptions, JoinResult, target_from};
use crate::key::load_or_create_key;
use crate::ticket::{Purpose, Ticket};
use crate::wire::{Card, MAX_DIAL_ITEMS};
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
pub(crate) const REJECT_REVOKED: u32 = 403;
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
pub(crate) struct Live {
    next: AtomicU64,
    conns: Mutex<HashMap<EndpointId, Vec<(u64, WeakConnectionHandle)>>>,
}

impl Live {
    /// Remembers a connection (weakly: dropping it still closes it). Handles whose connection is
    /// gone are dropped here, so connections nobody unregisters do not pile up.
    pub(crate) fn register(&self, id: EndpointId, conn: WeakConnectionHandle) -> u64 {
        let token = self.next.fetch_add(1, Ordering::SeqCst);
        let mut conns = self.conns.lock().unwrap();
        conns.retain(|_, list| {
            list.retain(|(_, c)| c.upgrade().is_some());
            !list.is_empty()
        });
        conns.entry(id).or_default().push((token, conn));
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

    /// Closes every open connection of every id. Returns how many were still alive.
    pub(crate) fn close_every(&self) -> usize {
        let handles: Vec<WeakConnectionHandle> = self
            .conns
            .lock()
            .unwrap()
            .values()
            .flat_map(|l| l.iter().map(|(_, c)| c.clone()))
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.upgrade())
            .map(|c| c.close(REJECT_REVOKED.into(), b"revoked"))
            .count()
    }

    /// Closes every open connection of `id`. Returns how many were still alive.
    pub(crate) fn close_all(&self, id: &EndpointId) -> usize {
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

pub(crate) struct LiveGuard {
    live: Arc<Live>,
    id: EndpointId,
    token: u64,
}

impl LiveGuard {
    pub(crate) fn new(live: Arc<Live>, id: EndpointId, token: u64) -> Self {
        Self { live, id, token }
    }
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

/// Whether `id` is the edge this device registered with. Any failure is "no": the gate fails closed.
pub(crate) async fn still_edge(db: &Database, id: EndpointId) -> bool {
    let db = db.clone();
    tokio::task::spawn_blocking(move || peers::is_edge(&db, &id.to_string()).unwrap_or(false))
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
        // Two protocols, two lists: a paired peer may not open the edge protocol and the edge may
        // not open the service protocol, so neither can borrow the other's door.
        let allowed = if conn.alpn() == SERVICE_ALPN {
            still_paired(&self.db, conn.remote_id()).await
        } else if conn.alpn() == EDGE_ALPN {
            still_edge(&self.db, conn.remote_id()).await
        } else {
            false
        };
        if allowed {
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
        // The joiner may never send its details, so the address it came from is recorded now.
        let observed: Vec<String> = joiner.observed.iter().map(|a| a.to_string()).collect();
        peers::add_peer(
            &self.db,
            &joiner.id.to_string(),
            &joiner.name,
            &[],
            &observed,
        )?;
        Ok(None)
    }

    fn details(&self, joiner: &EndpointId, relays: &[String], addrs: &[String]) {
        if let Err(e) = peers::set_dial_info(&self.db, &joiner.to_string(), relays, addrs) {
            tracing::warn!("could not record how to dial {}: {e:#}", joiner.fmt_short());
        }
    }
}

/// What forgetting the edge did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForgotEdge {
    /// Whether an edge was on record.
    pub had_record: bool,
    /// How many of the edge's connections were open here and are now closed.
    pub closed: usize,
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
    /// The edge's connections, kept apart from the paired peers' so that forgetting the edge
    /// sweeps exactly those, whatever the record says.
    edge_live: Arc<Live>,
    /// Connections from the registered edge that reached the edge handler.
    pub reached_edge: Arc<AtomicU64>,
    /// Edge streams dropped because the device's stream cap was full.
    pub edge_over_cap: Arc<AtomicU64>,
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
        Self::start_with_edge_door(cfg, db, handler, None).await
    }

    /// Like [`Bridge::start`], and with an `edge_door` it also accepts the edge protocol from the
    /// edge this device registered with (and from no other peer), piping each stream to the
    /// connector door on loopback. Without one, the edge protocol is not accepted at all.
    pub async fn start_with_edge_door(
        cfg: BridgeConfig,
        db: Database,
        handler: Option<StreamHandler>,
        edge_door: Option<EdgeDoor>,
    ) -> Result<Self> {
        if let Some(door) = &edge_door {
            door.validate()?;
        }
        let key = load_or_create_key(&cfg.key)?;
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .dns_resolver(crate::dns::resolver())
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
        let edge_live = Arc::new(Live::default());
        let reached_service = Arc::new(AtomicU64::new(0));
        let reached_edge = Arc::new(AtomicU64::new(0));
        let edge_over_cap = Arc::new(AtomicU64::new(0));
        let router = (handler.is_some() || edge_door.is_some()).then(|| {
            let mut builder = Router::builder(endpoint.clone());
            if let Some(handler) = handler {
                builder = builder.accept(
                    SERVICE_ALPN,
                    Service {
                        db: db.clone(),
                        live: live.clone(),
                        handler,
                        reached: reached_service.clone(),
                    },
                );
            }
            if let Some(door) = edge_door {
                builder = builder.accept(
                    EDGE_ALPN,
                    EdgeService {
                        db: db.clone(),
                        live: edge_live.clone(),
                        permits: Arc::new(tokio::sync::Semaphore::new(door.max_streams)),
                        door,
                        reached: reached_edge.clone(),
                        over_cap: edge_over_cap.clone(),
                    },
                );
            }
            builder.spawn()
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
            edge_live,
            reached_edge,
            edge_over_cap,
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
            addrs: host::ticket_addrs(
                &self.endpoint,
                self.with_addrs,
                &self.advertise,
                MAX_DIAL_ITEMS,
            ),
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

    /// Registers this device with the `hq edge` that shows `ticket` and records it as this
    /// device's edge. The edge tells the device the name it is reached at; that name must be the
    /// one this device's key gives it under the edge's domain, so an edge cannot hand over
    /// another device's name or an unrelated one. Nothing is recorded unless the person at the
    /// edge typed the right digits and the answer checks out.
    pub async fn register(
        &self,
        ticket: &Ticket,
        tuning: &Tuning,
        show_sas: impl FnOnce(&str),
    ) -> Result<JoinResult> {
        if ticket.k != Purpose::Register {
            bail!("this ticket pairs two machines; it does not register a device with an edge");
        }
        // The edge dials this device through a relay it can see in its own list; the addresses
        // are left out, since the edge sees the one this connection came from anyway.
        let mut opts = JoinOptions::new(&self.name, self.card().relays, Vec::new());
        opts.hello_timeout = tuning.hello_timeout;
        opts.decision_timeout = tuning.decision_timeout;
        let result = join::join(&self.endpoint, ticket, &opts, show_sas).await?;
        if let JoinResult::Paired(joined) = &result {
            let record = edge_record(&self.id(), &joined.card, joined.info.as_ref())?;
            let db = self.db.clone();
            tokio::task::spawn_blocking(move || {
                peers::edge_set(
                    &db,
                    &record.id,
                    &record.domain,
                    &record.hostname,
                    record.relay.as_deref(),
                )
            })
            .await
            .context("recording the edge")??;
            // A device has one edge: the connections of the one this replaced are closed.
            self.edge_live.close_every();
        }
        Ok(result)
    }

    /// Forgets the edge and closes every connection the edge protocol has open, at once. Returns
    /// whether there was an edge on record. It sweeps even when none is: another process (the
    /// CLI) may have cleared the record while a connection was open here, and this is how that
    /// connection is closed.
    pub async fn forget_edge(&self) -> Result<ForgotEdge> {
        let db = self.db.clone();
        let had_record = tokio::task::spawn_blocking(move || peers::edge_clear(&db)).await??;
        Ok(ForgotEdge {
            had_record,
            closed: self.edge_live.close_every(),
        })
    }

    /// Connects to a paired peer by key alone: no code, no ticket.
    pub async fn connect(&self, peer: &str) -> Result<Connection> {
        let id: EndpointId = peer
            .parse()
            .map_err(|_| anyhow!("{peer} is not a machine id"))?;
        let db = self.db.clone();
        let key = id.to_string();
        let record = tokio::task::spawn_blocking(move || peers::get(&db, &key))
            .await??
            .filter(|p| p.is_active())
            .ok_or_else(|| anyhow!("{peer} is not a paired peer"))?;
        let target = target_from(id, &record.relays, &record.addrs)?;
        let conn = self.endpoint.connect(target, SERVICE_ALPN).await?;
        // A revoke closes this one too, not only the connections the peer opened to us.
        self.live.register(id, conn.weak_handle());
        Ok(conn)
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

/// What the edge told a device about itself, checked: the edge's id, the name this device is
/// reached at and the relay it offers. The name must be `<label>.<domain>` where the label is the
/// one this device's own key gives it (so an edge cannot hand over another device's name) and the
/// domain is a plain DNS name; the domain is whatever the edge serves devices under. The relay
/// must be a plain https URL on a name, never an address, because it becomes a home relay this
/// device connects to.
fn edge_record(
    me: &EndpointId,
    card: &Card,
    info: Option<&serde_json::Value>,
) -> Result<peers::EdgeRecord> {
    let id: EndpointId = card
        .id
        .parse()
        .map_err(|_| anyhow!("the edge's card names no machine"))?;
    let told = info
        .and_then(|i| i.get("hostname"))
        .and_then(|h| h.as_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| anyhow!("the edge did not say what name this device is reached at"))?;
    let (label, domain) = told
        .split_once('.')
        .ok_or_else(|| anyhow!("the edge's name for this device is not a domain name"))?;
    if label != device_label(me) {
        bail!(
            "the edge named this device {label}.{domain}, which is not the name its key gives it: not registering"
        );
    }
    if !is_plain_dns_name(domain) {
        bail!("the edge's domain is not a plain DNS name");
    }
    let relay = info
        .and_then(|i| i.get("relay"))
        .and_then(|r| r.as_str())
        .map(plain_https_relay)
        .transpose()?;
    Ok(peers::EdgeRecord {
        id: id.to_string(),
        domain: domain.to_string(),
        hostname: told.clone(),
        relay,
        registered_at: 0,
    })
}

/// Lowercase letters, digits and hyphens in dot-separated labels, at least two of them, 253
/// characters at most.
fn is_plain_dns_name(name: &str) -> bool {
    name.len() <= 253
        && name.split('.').count() >= 2
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// A relay URL the device may adopt: https on a DNS name (not an address, not `localhost`, not a
/// name that stays on the local network), no credentials, the default port, at most 253
/// characters. Anything else is refused rather than dialed.
fn plain_https_relay(text: &str) -> Result<String> {
    let bad = || anyhow!("the edge offered a relay this device will not use");
    if text.len() > 253 {
        return Err(bad());
    }
    let url = url::Url::parse(text).map_err(|_| bad())?;
    let host = match url.host() {
        Some(url::Host::Domain(d)) => d.to_ascii_lowercase(),
        _ => return Err(bad()),
    };
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with('.') && host.trim_end_matches('.').ends_with("localhost");
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || local
        || !host.contains('.')
        || !is_plain_dns_name(host.trim_end_matches('.'))
    {
        return Err(bad());
    }
    Ok(url.to_string())
}

/// The DNS label an edge reaches a device at: the first [`LABEL_BYTES`] bytes of its id, as hex.
pub fn device_label(id: &EndpointId) -> String {
    id.as_bytes()[..LABEL_BYTES]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// "direct" when the selected path of a connection is an IP address, "relay" when it goes through
/// a relay, "none" while there is no selected path.
pub fn path_kind(conn: &Connection) -> &'static str {
    let paths = conn.paths();
    let selected = paths.iter().find(|p| p.is_selected());
    match selected.as_ref().map(|p| p.remote_addr()) {
        Some(iroh::TransportAddr::Ip(_)) => "direct",
        Some(iroh::TransportAddr::Relay(_)) => "relay",
        _ => "none",
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

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn me() -> EndpointId {
        SecretKey::generate().public()
    }

    fn card(edge: &EndpointId) -> Card {
        Card {
            id: edge.to_string(),
            name: "whatever the edge calls itself".into(),
            relays: vec![],
            addrs: vec![],
        }
    }

    fn told(me: &EndpointId, rest: &str) -> serde_json::Value {
        serde_json::json!({ "hostname": format!("{}.{rest}", device_label(me)) })
    }

    #[test]
    fn the_domain_comes_from_the_name_and_is_not_cut_short() {
        let (me, edge) = (me(), me());
        let long = format!("{}.example.com", "a".repeat(60));
        let record = edge_record(&me, &card(&edge), Some(&told(&me, &long))).unwrap();
        assert_eq!(
            record.domain, long,
            "a domain longer than a card name allows"
        );
        assert_eq!(record.hostname, format!("{}.{long}", device_label(&me)));
        let upper = told(&me, "EDGE.Example.COM");
        let record = edge_record(&me, &card(&edge), Some(&upper)).unwrap();
        assert_eq!(record.domain, "edge.example.com");
    }

    #[test]
    fn a_name_that_is_not_this_devices_own_label_under_a_plain_domain_is_refused() {
        let (me, other, edge) = (me(), me(), me());
        let wrong_label = told(&other, "edge.example.com");
        assert!(edge_record(&me, &card(&edge), Some(&wrong_label)).is_err());
        for bad in [
            json_host(&me, "edge"),
            json_host(&me, "-edge.example.com"),
            json_host(&me, "edge..example.com"),
            json_host(&me, "edge.example.com."),
            json_host(&me, "ed ge.example.com"),
            json_host(&me, "edge.exämple.com"),
            json_host(&me, &format!("{}.com", "a".repeat(64))),
        ] {
            assert!(edge_record(&me, &card(&edge), Some(&bad)).is_err(), "{bad}");
        }
        assert!(edge_record(&me, &card(&edge), None).is_err());
        assert!(edge_record(&me, &card(&edge), Some(&serde_json::json!({}))).is_err());
        assert!(edge_record(&me, &card(&edge), Some(&serde_json::json!({"hostname": 7}))).is_err());
    }

    fn json_host(me: &EndpointId, rest: &str) -> serde_json::Value {
        told(me, rest)
    }

    #[test]
    fn only_a_plain_https_relay_on_a_name_is_adopted() {
        for ok in [
            "https://relay.edge.example.com",
            "https://relay.edge.example.com/",
            "HTTPS://Relay.Edge.Example.com",
        ] {
            assert!(plain_https_relay(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://relay.edge.example.com",
            "https://127.0.0.1",
            "https://169.254.169.254",
            "https://10.0.0.5",
            "https://[::1]",
            "https://localhost",
            "https://localhost.",
            "https://x.localhost",
            "https://printer.local",
            "https://db.internal",
            "https://relay",
            "https://relay.edge.example.com:6379",
            "https://user:pw@relay.example.com",
            "ftp://relay.edge.example.com",
            "relay.edge.example.com",
            "",
        ] {
            assert!(plain_https_relay(bad).is_err(), "{bad}");
        }
        assert!(plain_https_relay(&format!("https://{}.com", "a".repeat(250))).is_err());
    }
}
