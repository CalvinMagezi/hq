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

use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use tokio::{
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

/// How often an open connection checks that its device is still registered.
const REVOKE_CHECK: Duration = Duration::from_secs(1);
/// How long to wait for the relay's loopback listener.
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
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
    forwarded: AtomicU64,
    relay_forwarded: AtomicU64,
    ended_idle: AtomicU64,
    ended_cancelled: AtomicU64,
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
    pub forwarded: u64,
    pub relay_forwarded: u64,
    pub ended_idle: u64,
    /// Forwarded connections cut because the device was revoked or the edge stopped.
    pub ended_cancelled: u64,
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
            forwarded: get(&self.forwarded),
            relay_forwarded: get(&self.relay_forwarded),
            ended_idle: get(&self.ended_idle),
            ended_cancelled: get(&self.ended_cancelled),
        }
    }
}

struct DeviceState {
    slots: Arc<Semaphore>,
    bucket: Arc<TokenBucket>,
    /// One QUIC connection per device, with a stream per forwarded connection. The lock also
    /// serializes dialing, so a burst of connections dials once.
    conn: tokio::sync::Mutex<Option<Connection>>,
}

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
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .relay_mode(relay_mode)
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
        Ok(Self {
            addr,
            shared,
            stop,
            task,
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
        self.shared.endpoint.close().await;
    }
}

impl Drop for Edge {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        self.task.abort();
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
                })
            })
            .clone()
    }

    async fn forward_to_device(&self, tcp: TcpStream, hello: Vec<u8>, device: Registered) {
        let state = self.device_state(&device.id);
        let Ok(_slot) = state.slots.clone().try_acquire_owned() else {
            inc(&self.stats.device_busy);
            return;
        };
        let opened = tokio::time::timeout(
            self.limits.dial_timeout(),
            self.open_stream(&state, &device),
        )
        .await;
        let Ok(Ok((mut send, recv))) = opened else {
            inc(&self.stats.dial_failed);
            return;
        };
        if send.write_all(&hello).await.is_err() {
            inc(&self.stats.dial_failed);
            return;
        }
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
            self.revoked(label, device.id),
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
        if tokio::io::AsyncWriteExt::write_all(&mut upstream, &hello)
            .await
            .is_err()
        {
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

    /// Opens a stream to the device on its cached connection, dialing (once, under the lock) when
    /// there is none or it has closed.
    async fn open_stream(
        &self,
        state: &DeviceState,
        device: &Registered,
    ) -> Result<(SendStream, RecvStream)> {
        let mut cached = state.conn.lock().await;
        if let Some(conn) = cached.as_ref().filter(|c| c.close_reason().is_none())
            && let Ok(streams) = conn.open_bi().await
        {
            return Ok(streams);
        }
        let mut addr = EndpointAddr::new(device.id);
        if let Some(relay) = device.relay.clone().or_else(|| self.dial_relay.clone()) {
            addr = addr.with_relay_url(relay);
        }
        let conn = self.endpoint.connect(addr, EDGE_ALPN).await?;
        let streams = conn.open_bi().await?;
        *cached = Some(conn);
        Ok(streams)
    }

    /// Completes when the edge is stopping.
    async fn stopped(&self) {
        let mut shutdown = self.shutdown.clone();
        // An error means the sender is gone, which is also a stop.
        let _ = shutdown.wait_for(|stop| *stop).await;
    }

    /// Completes when the device is no longer registered under the same id, or the edge stops.
    async fn revoked(&self, label: String, id: EndpointId) {
        let still_registered = async {
            loop {
                tokio::time::sleep(REVOKE_CHECK).await;
                if self.registry.lookup(&label).map(|r| r.id) != Some(id) {
                    return;
                }
            }
        };
        tokio::select! {
            () = still_registered => {}
            () = self.stopped() => {}
        }
    }
}

struct Hello {
    name: String,
    bytes: Vec<u8>,
}
