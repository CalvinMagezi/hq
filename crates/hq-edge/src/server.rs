//! The forwarder: one listener, one decision per connection, one iroh stream per forwarded
//! connection. See `docs/security/EDGE.md` for what each guard is for.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, QuicTransportConfig, RecvStream, SendStream, presets},
};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::{Semaphore, watch},
    task::JoinHandle,
};

use crate::{
    EDGE_ALPN, LABEL_BYTES,
    config::{EdgeConfig, LimitsConfig},
    hello::{MAX_HELLO_BYTES, read_client_hello},
    limits::{PendingGuard, PendingLimiter, TokenBucket},
    pipe::{PipeEnd, PipeLimits, pipe},
    registry::{Registered, Registry, device_label},
    relay::{RelayHost, RelayParams, RelayTls},
};

/// How often the sweep checks that every device with a connection is still registered.
const REVOKE_CHECK: Duration = Duration::from_secs(1);
/// How long to wait for the relay's loopback listener.
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The QUIC close code for a connection to a device that is no longer registered.
const REVOKED_CODE: u32 = 403;
/// Pause after an accept error (out of file descriptors, for one) so the loop does not spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);
/// Characters of a device label that may appear in a log line.
const LOG_PREFIX: usize = 6;

/// Counters for tests and for the one summary line at shutdown. Nothing here is exposed on the
/// network: the edge has no admin surface.
#[derive(Default)]
pub struct Stats {
    accepted: AtomicU64,
    refused_busy: AtomicU64,
    hello_timeout: AtomicU64,
    hello_bad: AtomicU64,
    unknown_name: AtomicU64,
    device_busy: AtomicU64,
    dial_failed: AtomicU64,
    redialed: AtomicU64,
    forwarded: AtomicU64,
    relay_forwarded: AtomicU64,
    ended_idle: AtomicU64,
    ended_cancelled: AtomicU64,
    revoked_closed: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatsSnapshot {
    pub accepted: u64,
    /// Closed because a cap was full: the pre-hello caps on arrival, or the relay's connection caps.
    pub refused_busy: u64,
    /// Closed because no complete ClientHello arrived within the deadline.
    pub hello_timeout: u64,
    /// Closed because the first bytes were not a usable ClientHello.
    pub hello_bad: u64,
    /// Closed because the name belongs to no registered device.
    pub unknown_name: u64,
    /// Closed because the device already had its maximum of connections.
    pub device_busy: u64,
    /// Closed because the device could not be reached.
    pub dial_failed: u64,
    /// Connections to a device replaced because the one the edge held did not answer.
    pub redialed: u64,
    pub forwarded: u64,
    pub relay_forwarded: u64,
    pub ended_idle: u64,
    /// Forwarded connections cut because the edge stopped.
    pub ended_cancelled: u64,
    /// Connections to devices that left the registry, closed by the edge.
    pub revoked_closed: u64,
}

fn inc(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

impl Stats {
    fn snapshot(&self) -> StatsSnapshot {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        StatsSnapshot {
            accepted: get(&self.accepted),
            refused_busy: get(&self.refused_busy),
            hello_timeout: get(&self.hello_timeout),
            hello_bad: get(&self.hello_bad),
            unknown_name: get(&self.unknown_name),
            device_busy: get(&self.device_busy),
            dial_failed: get(&self.dial_failed),
            redialed: get(&self.redialed),
            forwarded: get(&self.forwarded),
            relay_forwarded: get(&self.relay_forwarded),
            ended_idle: get(&self.ended_idle),
            ended_cancelled: get(&self.ended_cancelled),
            revoked_closed: get(&self.revoked_closed),
        }
    }
}

struct DeviceState {
    slots: Arc<Semaphore>,
    bucket: Arc<TokenBucket>,
    /// One QUIC connection per device, with a stream per forwarded connection. The lock also
    /// serializes dialing, so a burst of connections dials once.
    conn: tokio::sync::Mutex<Option<Connection>>,
    /// Connections that stopped answering and were replaced. They are kept, not closed, because
    /// streams of other clients may still be running on them, but they stay known so that a
    /// revoke closes them too and so they are let go once those streams must have ended.
    retired: Mutex<Vec<(Connection, std::time::Instant)>>,
}

impl DeviceState {
    /// The connection the edge holds to the device, if it is still open.
    async fn live_connection(&self) -> Option<Connection> {
        let cached = self.conn.lock().await;
        cached
            .as_ref()
            .filter(|c| c.close_reason().is_none())
            .cloned()
    }

    /// Stops using `stale` for new streams. It is not closed: streams already open on it belong to
    /// other clients. It is remembered so a revoke reaches it, and let go after `keep`.
    async fn retire(&self, stale: Connection, keep: Duration) {
        {
            let mut cached = self.conn.lock().await;
            if cached
                .as_ref()
                .is_some_and(|c| c.stable_id() == stale.stable_id())
            {
                *cached = None;
            }
        }
        let mut retired = self.retired.lock().unwrap_or_else(|e| e.into_inner());
        retired.retain(|(_, since)| since.elapsed() < keep);
        retired.push((stale, std::time::Instant::now()));
    }

    /// Lets go of retired connections that are older than `keep`: whatever ran on them has hit
    /// the idle timeout by now, and the last handle's drop closes them.
    fn prune_retired(&self, keep: Duration) {
        let mut retired = self.retired.lock().unwrap_or_else(|e| e.into_inner());
        retired.retain(|(_, since)| since.elapsed() < keep);
    }

    /// Closes every retired connection (the device left the registry).
    fn close_retired(&self) -> usize {
        let mut retired = self.retired.lock().unwrap_or_else(|e| e.into_inner());
        let closed = retired.len();
        for (conn, _) in retired.drain(..) {
            conn.close(REVOKED_CODE.into(), b"revoked");
        }
        closed
    }
}

/// An attempt to send the client's first flight and read the device's answer found the connection
/// dead or silent: a path that died while the connection stayed open. Anything the device itself
/// did (finish or reset the stream, refuse) is an answer, not this.
struct Stale(anyhow::Error);

/// A stream to a device that has answered the client's first flight.
struct Reached {
    send: SendStream,
    recv: RecvStream,
    /// The device's first bytes, already read: they go to the client before the pipe starts.
    first: Vec<u8>,
}

/// How much of the device's first flight is read to learn that it answered. A ServerHello with a
/// certificate chain is a few KiB; whatever is left is piped on.
const FIRST_FLIGHT_BYTES: usize = 8 * 1024;

struct RelayRoute {
    hostname: String,
    target: SocketAddr,
    connections: Arc<PendingLimiter>,
}

struct Shared {
    domain_suffix: String,
    registry: Arc<dyn Registry>,
    limits: LimitsConfig,
    pending: Arc<PendingLimiter>,
    endpoint: Endpoint,
    dial_relay: Option<RelayUrl>,
    devices: Mutex<HashMap<EndpointId, Arc<DeviceState>>>,
    relay: Option<RelayRoute>,
    stats: Stats,
    shutdown: watch::Receiver<bool>,
}

enum Route {
    Device(Registered),
    Relay,
    Unknown,
}

/// A running edge. Dropping it stops the listener and cuts open connections; [`Edge::shutdown`]
/// also waits for the listener to stop and closes the iroh endpoint.
pub struct Edge {
    addr: SocketAddr,
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    background: Vec<JoinHandle<()>>,
    _relay: Option<RelayHost>,
}

impl Edge {
    /// Starts everything the config asks for: the iroh endpoint, the co-hosted relay when enabled,
    /// and the listener.
    pub async fn start(config: EdgeConfig, registry: Arc<dyn Registry>) -> Result<Self> {
        config.validate()?;
        let key = crate::identity::load_or_create_key(&config.key_path())?;
        let relay = if config.relay.enabled {
            let params = RelayParams {
                hostname: crate::config::normalize_hostname(&config.relay.hostname)?,
                tls: RelayTls::LetsEncrypt {
                    email: config.relay.acme_email.clone(),
                    production: config.relay.acme_production,
                    cache_dir: config.acme_cache_dir(),
                },
                quic_bind: config.relay.quic_bind,
            };
            Some(RelayHost::spawn(params, registry.clone(), key.public()).await?)
        } else {
            None
        };
        Self::start_with(config, registry, key, relay).await
    }

    /// Like [`Edge::start`] with a key and a relay the caller already made. Tests use it to run
    /// the relay without TLS; `start` is what the command uses.
    pub async fn start_with(
        config: EdgeConfig,
        registry: Arc<dyn Registry>,
        key: SecretKey,
        relay: Option<RelayHost>,
    ) -> Result<Self> {
        config.validate()?;
        let domain = crate::config::normalize_hostname(&config.domain)?;
        let configured_relay: Option<RelayUrl> = config
            .relay_url
            .as_deref()
            .map(str::parse)
            .transpose()
            .context("edge.relay_url")?;
        let dial_relay = configured_relay.or_else(|| relay.as_ref().map(|r| r.url().clone()));
        let relay_mode = match &dial_relay {
            Some(url) => RelayMode::custom([url.clone()]),
            None => {
                tracing::warn!(
                    "no relay is configured: devices are dialed through the relay named on their list line, if any"
                );
                RelayMode::Default
            }
        };
        // The edge only ever opens streams. A device that opens streams or sends datagrams toward
        // it would otherwise be buffered, up to a window per stream, on a connection the edge keeps.
        let transport = QuicTransportConfig::builder()
            .max_concurrent_bidi_streams(0u32.into())
            .max_concurrent_uni_streams(0u32.into())
            .datagram_receive_buffer_size(None)
            .build();
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .relay_mode(relay_mode)
            .transport_config(transport)
            .clear_ip_transports()
            .bind_addr(config.iroh_bind)
            .context("edge.iroh_bind")?
            .bind()
            .await
            .context("starting the iroh endpoint")?;

        let listener = TcpListener::bind(config.listen)
            .await
            .with_context(|| format!("listening on {}", config.listen))?;
        let addr = listener.local_addr()?;

        let limits = config.limits.clone();
        let (stop, shutdown) = watch::channel(false);
        let shared = Arc::new(Shared {
            domain_suffix: format!(".{domain}"),
            registry,
            pending: PendingLimiter::new(limits.max_pending, limits.max_pending_per_source),
            endpoint,
            dial_relay,
            devices: Mutex::new(HashMap::new()),
            relay: relay.as_ref().map(|r| RelayRoute {
                hostname: r.hostname().to_owned(),
                target: r.target(),
                connections: PendingLimiter::new(
                    limits.max_relay_connections,
                    limits.max_pending_per_source,
                ),
            }),
            limits,
            stats: Stats::default(),
            shutdown,
        });
        let task = tokio::spawn(accept_loop(shared.clone(), listener));
        // Nothing dials the edge yet (pairing will, on its own protocol); refusing at the first
        // packet keeps unsolicited QUIC handshakes from queueing inside the endpoint.
        let refuse = tokio::spawn(refuse_incoming(shared.endpoint.clone()));
        let janitor = tokio::spawn(sweep_revoked(shared.clone()));
        Ok(Self {
            addr,
            shared,
            stop,
            task,
            background: vec![refuse, janitor],
            _relay: relay,
        })
    }

    /// The address the TCP listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// The edge's endpoint id, which devices pin as the only peer allowed on the edge protocol.
    pub fn id(&self) -> EndpointId {
        self.shared.endpoint.id()
    }

    pub fn stats(&self) -> StatsSnapshot {
        self.shared.stats.snapshot()
    }

    /// The co-hosted relay, when there is one.
    pub fn relay(&self) -> Option<&RelayHost> {
        self._relay.as_ref()
    }

    /// Stops accepting, cuts open connections and closes the iroh endpoint.
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        let _ = (&mut self.task).await;
        for task in &self.background {
            task.abort();
        }
        self.shared.endpoint.close().await;
    }
}

impl Drop for Edge {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.task.abort();
        for task in &self.background {
            task.abort();
        }
    }
}

async fn accept_loop(shared: Arc<Shared>, listener: TcpListener) {
    let mut shutdown = shared.shutdown.clone();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((tcp, peer)) => {
                    inc(&shared.stats.accepted);
                    tokio::spawn(shared.clone().handle(tcp, peer));
                }
                Err(e) => {
                    tracing::warn!("accept failed: {e}");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
            _ = shutdown.changed() => return,
        }
    }
}

async fn refuse_incoming(endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        incoming.refuse();
    }
}

/// Closes the cached connection to every device that has left the registry and forgets the ones
/// nothing is using, so a revoked device keeps neither a connection nor an entry.
async fn sweep_revoked(shared: Arc<Shared>) {
    let mut shutdown = shared.shutdown.clone();
    loop {
        tokio::select! {
            () = tokio::time::sleep(REVOKE_CHECK) => {}
            _ = shutdown.changed() => return,
        }
        // An unreadable list answers "no" to everyone; that is a reason to refuse new connections,
        // not to cut the ones that are open.
        if !shared.registry.is_readable() {
            continue;
        }
        let stale: Vec<(EndpointId, Arc<DeviceState>)> = {
            let devices = shared.devices.lock().unwrap_or_else(|e| e.into_inner());
            devices
                .iter()
                .filter(|(id, _)| !shared.registry.is_registered(id))
                .map(|(id, state)| (*id, state.clone()))
                .collect()
        };
        let keep = shared.retired_keep();
        {
            let devices = shared.devices.lock().unwrap_or_else(|e| e.into_inner());
            for state in devices.values() {
                state.prune_retired(keep);
            }
        }
        for (id, state) in stale {
            // A dial in progress holds the lock; the next round will get it.
            if let Ok(mut cached) = state.conn.try_lock()
                && let Some(conn) = cached.take()
            {
                conn.close(REVOKED_CODE.into(), b"revoked");
                inc(&shared.stats.revoked_closed);
            }
            for _ in 0..state.close_retired() {
                inc(&shared.stats.revoked_closed);
            }
            let mut devices = shared.devices.lock().unwrap_or_else(|e| e.into_inner());
            // Two references are the map's and the one in `stale`: nothing is forwarding.
            if devices.get(&id).is_some_and(|s| Arc::strong_count(s) <= 2) {
                devices.remove(&id);
            }
        }
    }
}

impl Shared {
    /// Everything that happens to one inbound connection. Every early return drops the socket
    /// without writing a byte: a refused client learns nothing, not even that a TLS server is here.
    async fn handle(self: Arc<Self>, mut tcp: TcpStream, peer: SocketAddr) {
        let _ = tcp.set_nodelay(true);
        let Some(pending) = self.pending.try_enter(peer.ip()) else {
            inc(&self.stats.refused_busy);
            return;
        };
        let Some(hello) = self.read_hello(&mut tcp, pending).await else {
            return;
        };
        match self.route(&hello.name) {
            Route::Relay => self.forward_to_relay(tcp, peer, hello.bytes).await,
            Route::Device(device) => self.forward_to_device(tcp, hello.bytes, device).await,
            Route::Unknown => inc(&self.stats.unknown_name),
        }
    }

    /// Reads the ClientHello under the whole-connection deadline and frees the pending slot as
    /// soon as it is done, whatever the outcome.
    async fn read_hello(&self, tcp: &mut TcpStream, pending: PendingGuard) -> Option<Hello> {
        let read = tokio::time::timeout(
            self.limits.hello_timeout(),
            read_client_hello(tcp, MAX_HELLO_BYTES),
        )
        .await;
        drop(pending);
        match read {
            Err(_elapsed) => inc(&self.stats.hello_timeout),
            Ok(Err(_)) => inc(&self.stats.hello_bad),
            Ok(Ok(hello)) => {
                if let Some(name) = hello.name {
                    return Some(Hello {
                        name,
                        bytes: hello.bytes,
                    });
                }
                inc(&self.stats.hello_bad);
            }
        }
        None
    }

    fn route(&self, name: &str) -> Route {
        if self.relay.as_ref().is_some_and(|r| r.hostname == name) {
            return Route::Relay;
        }
        let Some(label) = name.strip_suffix(&self.domain_suffix) else {
            return Route::Unknown;
        };
        let shaped = label.len() == LABEL_BYTES * 2 && label.bytes().all(|b| b.is_ascii_hexdigit());
        match shaped.then(|| self.registry.lookup(label)).flatten() {
            Some(device) => Route::Device(device),
            None => Route::Unknown,
        }
    }

    fn device_state(&self, id: &EndpointId) -> Arc<DeviceState> {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
            .entry(*id)
            .or_insert_with(|| {
                Arc::new(DeviceState {
                    slots: Arc::new(Semaphore::new(self.limits.max_connections_per_device)),
                    bucket: Arc::new(TokenBucket::new(
                        self.limits.bytes_per_second,
                        self.limits.burst_bytes,
                    )),
                    conn: tokio::sync::Mutex::new(None),
                    retired: Mutex::new(Vec::new()),
                })
            })
            .clone()
    }

    async fn forward_to_device(&self, mut tcp: TcpStream, hello: Vec<u8>, device: Registered) {
        let state = self.device_state(&device.id);
        let Ok(_slot) = state.slots.clone().try_acquire_owned() else {
            inc(&self.stats.device_busy);
            return;
        };
        // Reaching the device, its first answer and handing that to the client share one deadline:
        // a device that grants no stream credit or never answers must not pin the slot and the
        // socket.
        let setup = async {
            let reached = self.reach_device(&state, &device, &hello).await?;
            tcp.write_all(&reached.first).await?;
            Ok::<_, anyhow::Error>(reached)
        };
        let Ok(Ok(Reached { send, recv, .. })) =
            tokio::time::timeout(self.limits.dial_timeout(), setup).await
        else {
            inc(&self.stats.dial_failed);
            return;
        };
        inc(&self.stats.forwarded);
        let label = device_label(&device.id);
        tracing::debug!(device = &label[..LOG_PREFIX], "forwarding a connection");
        let end = pipe(
            tcp.into_split(),
            (recv, send),
            PipeLimits {
                idle: self.limits.idle_timeout(),
                bucket: Some(state.bucket.clone()),
            },
            self.stopped(),
        )
        .await;
        self.note_end(end);
    }

    async fn forward_to_relay(&self, tcp: TcpStream, peer: SocketAddr, hello: Vec<u8>) {
        let Some(relay) = &self.relay else { return };
        let Some(_slot) = relay.connections.try_enter(peer.ip()) else {
            inc(&self.stats.refused_busy);
            return;
        };
        let upstream =
            tokio::time::timeout(RELAY_CONNECT_TIMEOUT, TcpStream::connect(relay.target)).await;
        let Ok(Ok(mut upstream)) = upstream else {
            inc(&self.stats.dial_failed);
            return;
        };
        if upstream.write_all(&hello).await.is_err() {
            return;
        }
        inc(&self.stats.relay_forwarded);
        let end = pipe(
            tcp.into_split(),
            upstream.into_split(),
            PipeLimits {
                idle: self.limits.idle_timeout(),
                bucket: None,
            },
            self.stopped(),
        )
        .await;
        self.note_end(end);
    }

    fn note_end(&self, end: PipeEnd) {
        match end {
            PipeEnd::Idle => inc(&self.stats.ended_idle),
            PipeEnd::Cancelled => inc(&self.stats.ended_cancelled),
            PipeEnd::Closed => {}
        }
    }

    /// How long a retired connection is remembered: whatever ran on it has hit the idle timeout
    /// (or a dial timeout, for a stream still being set up) by then.
    fn retired_keep(&self) -> Duration {
        self.limits.idle_timeout() + self.limits.dial_timeout()
    }

    /// Sends the client's first flight to the device on a new stream and waits for the first bytes
    /// of its answer. The connection the edge already holds gets `reply_timeout` to answer; if it
    /// does not (or is found broken), it is a dead path that still looks alive, so it is put aside
    /// and the same bytes go out again on a fresh connection. Only one retry: a fresh connection
    /// that stays silent is the device being down, not the cache being stale. A device that
    /// answers by finishing or resetting the stream is refusing the client, not dead, so that is
    /// handed back as an answer and the held connection is kept.
    async fn reach_device(
        &self,
        state: &DeviceState,
        device: &Registered,
        hello: &[u8],
    ) -> Result<Reached> {
        if let Some(held) = state.live_connection().await {
            match self
                .hello_and_reply(state, &held, hello, self.held_reply_wait())
                .await
            {
                Ok(reached) => return Ok(reached),
                Err(Stale(e)) => {
                    tracing::debug!("the held connection did not answer, redialing: {e:#}");
                    // A device that was removed while this client waited is not dialed again.
                    if !self.registry.is_registered(&device.id) {
                        return Err(anyhow!("the device is no longer registered"));
                    }
                    state.retire(held, self.retired_keep()).await;
                    inc(&self.stats.redialed);
                }
            }
        }
        let fresh = self.dial(state, device).await?;
        self.hello_and_reply(state, &fresh, hello, self.limits.dial_timeout())
            .await
            .map_err(|Stale(e)| e)
    }

    /// What a connection the edge already holds gets to answer: the configured wait, but never
    /// more than half the setup budget, so a redial always has time left.
    fn held_reply_wait(&self) -> Duration {
        self.limits
            .reply_timeout()
            .min(self.limits.dial_timeout() / 2)
    }

    /// Opens a stream, sends the first flight and reads the first bytes of the answer, all within
    /// `wait`. The byte budget is charged first, outside the wait: a rate limit holding the
    /// edge back is not a silent device.
    async fn hello_and_reply(
        &self,
        state: &DeviceState,
        conn: &Connection,
        hello: &[u8],
        wait: Duration,
    ) -> Result<Reached, Stale> {
        state.bucket.take(hello.len()).await;
        let exchange = async {
            let (mut send, mut recv) = conn.open_bi().await.map_err(|e| Stale(e.into()))?;
            match send.write_all(hello).await {
                Ok(()) => {}
                Err(e) if is_connection_lost_write(&e) => return Err(Stale(e.into())),
                // The device stopped reading: it is refusing this client. Hand back the stream.
                Err(_) => {
                    return Ok(Reached {
                        send,
                        recv,
                        first: Vec::new(),
                    });
                }
            }
            let mut first = vec![0u8; FIRST_FLIGHT_BYTES];
            match recv.read(&mut first).await {
                Ok(Some(n)) => first.truncate(n),
                // The device finished the stream or reset it without a byte: a refusal.
                Ok(None) => first.clear(),
                Err(e) if is_connection_lost_read(&e) => return Err(Stale(e.into())),
                Err(_) => first.clear(),
            }
            Ok(Reached { send, recv, first })
        };
        let reached = tokio::time::timeout(wait, exchange)
            .await
            .map_err(|_| Stale(anyhow!("no answer within {wait:?}")))??;
        state.bucket.take(reached.first.len()).await;
        Ok(reached)
    }

    /// Dials the device and keeps the connection for the next client, or returns the one another
    /// client dialed while this one waited for the lock (the lock also serializes dialing, so a
    /// burst of connections dials once).
    async fn dial(&self, state: &DeviceState, device: &Registered) -> Result<Connection> {
        let mut cached = state.conn.lock().await;
        if let Some(conn) = cached.as_ref().filter(|c| c.close_reason().is_none()) {
            return Ok(conn.clone());
        }
        let mut addr = EndpointAddr::new(device.id);
        if let Some(relay) = device.relay.clone().or_else(|| self.dial_relay.clone()) {
            addr = addr.with_relay_url(relay);
        }
        let conn = self.endpoint.connect(addr, EDGE_ALPN).await?;
        *cached = Some(conn.clone());
        Ok(conn)
    }

    /// Completes when the edge is stopping.
    async fn stopped(&self) {
        let mut shutdown = self.shutdown.clone();
        // An error means the sender is gone, which is also a stop.
        let _ = shutdown.wait_for(|stop| *stop).await;
    }
}

fn is_connection_lost_write(e: &iroh::endpoint::WriteError) -> bool {
    matches!(e, iroh::endpoint::WriteError::ConnectionLost(_))
}

fn is_connection_lost_read(e: &iroh::endpoint::ReadError) -> bool {
    matches!(e, iroh::endpoint::ReadError::ConnectionLost(_))
}

struct Hello {
    name: String,
    bytes: Vec<u8>,
}
