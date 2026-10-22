//! Ends TLS for the connector on any byte stream and serves the router over it.
//!
//! The stream is whatever carries the forwarder's bytes: an iroh bidirectional stream in
//! production, a TCP or in-memory stream in tests. The acceptor reads the ClientHello first and
//! chooses: an `acme-tls/1` hello for the connector's name is the CA checking a pending
//! challenge and gets the challenge certificate, then the connection ends without a byte of
//! HTTP; every other hello for that name gets the issued certificate and the router, behind
//! the gate; a hello for any other name is dropped.

use super::certs::{ChallengeSlot, LiveCert, sni_is};
use super::gate::{Gate, not_found};
use super::{ACME_TLS_ALPN, TlsError};
use axum::Router;
use axum::body::Body;
use hyper::body::Incoming;
use hyper::server::conn::{http1, http2};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio_rustls::LazyConfigAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::crypto::aws_lc_rs;
use tokio_rustls::rustls::server::{Acceptor, ResolvesServerCert};
use tower::ServiceExt;

/// Every limit that keeps one stalled or hostile connection from holding the door.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// ClientHello, and separately the rest of the handshake. A peer that opens a stream and
    /// says nothing is closed when the first runs out.
    pub handshake: Duration,
    /// HTTP/1 request line and headers.
    pub request_headers: Duration,
    /// One request, body and handler together. Longer than the 40 s a call waits for the
    /// owner's approval.
    pub request: Duration,
    /// A connection with no request in flight for this long is wound down, whatever the
    /// protocol: a session that completed its handshake and says nothing, an HTTP/2 session
    /// that answers pings and sends nothing, an HTTP/1 keep-alive nobody uses.
    pub idle: Duration,
    /// A connection's whole life. When it runs out the connection is wound down gracefully (an
    /// answer in flight is finished, then it closes).
    pub connection: Duration,
    /// How long a connection that is winding down for being idle may take to finish.
    pub shutdown_grace: Duration,
    pub http2_keep_alive_interval: Duration,
    pub http2_keep_alive_timeout: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            handshake: Duration::from_secs(10),
            request_headers: Duration::from_secs(10),
            request: Duration::from_secs(120),
            idle: Duration::from_secs(60),
            connection: Duration::from_secs(5 * 60),
            shutdown_grace: Duration::from_secs(10),
            http2_keep_alive_interval: Duration::from_secs(30),
            http2_keep_alive_timeout: Duration::from_secs(20),
        }
    }
}

/// Web connections served at once. A forwarder carries one stream per client, so this bounds
/// what the whole internet can ask of the device.
pub const MAX_CONNECTIONS: usize = 64;
/// Connections that are still saying hello. Bounded apart from the served ones, so a flood of
/// silent peers cannot use up the room a served connection needs, and the other way round.
const MAX_HELLOS: usize = 256;
/// Room kept for the CA's validation connections alone (Let's Encrypt validates from several
/// places at once), so a full door can still renew its certificate.
const RESERVED_VALIDATIONS: usize = 8;
const MAX_HTTP2_STREAMS: u32 = 32;
/// Requests being answered at once, across all connections.
const MAX_IN_FLIGHT_REQUESTS: usize = 16;
/// A request body past this is refused. The largest legitimate call is a note written through
/// `/mcp`; with the in-flight cap this bounds what the door holds in memory.
const MAX_REQUEST_BODY_BYTES: usize = 1 << 20;
/// How often the idle and lifetime checks look at a connection, at most.
const WATCH_TICK: Duration = Duration::from_secs(1);
const MIN_WATCH_TICK: Duration = Duration::from_millis(10);
const H2: &[u8] = b"h2";

/// How a connection ended, for tests and counters. Nothing about the peer is in it.
#[derive(Debug, PartialEq, Eq)]
pub enum Served {
    /// HTTP was served until the peer or a timeout ended it.
    Http,
    /// The CA's validation connection: handshake done, closed without HTTP.
    Challenge,
}

pub struct ConnectorAcceptor {
    host: String,
    live: Arc<LiveCert>,
    challenge: Arc<ChallengeSlot>,
    web: Arc<ServerConfig>,
    validation: Arc<ServerConfig>,
    gate: Arc<Gate>,
    router: Router,
    hellos: Arc<Semaphore>,
    web_slots: Arc<Semaphore>,
    validation_slots: Arc<Semaphore>,
    requests: Arc<Semaphore>,
    pub(crate) timeouts: Timeouts,
}

impl ConnectorAcceptor {
    pub(crate) fn new(
        host: &str,
        live: Arc<LiveCert>,
        challenge: Arc<ChallengeSlot>,
        gate: Gate,
        router: Router,
    ) -> Result<Self, TlsError> {
        let web = server_config(live.clone(), vec![b"h2".to_vec(), b"http/1.1".to_vec()])?;
        let validation = server_config(challenge.clone(), vec![ACME_TLS_ALPN.to_vec()])?;
        Ok(Self {
            host: host.to_ascii_lowercase(),
            live,
            challenge,
            web,
            validation,
            gate: Arc::new(gate),
            router,
            hellos: Arc::new(Semaphore::new(MAX_HELLOS)),
            web_slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            validation_slots: Arc::new(Semaphore::new(RESERVED_VALIDATIONS)),
            requests: Arc::new(Semaphore::new(MAX_IN_FLIGHT_REQUESTS)),
            timeouts: Timeouts::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_max_connections(&mut self, n: usize) {
        self.web_slots = Arc::new(Semaphore::new(n));
    }

    #[cfg(test)]
    pub(crate) fn set_max_hellos(&mut self, n: usize) {
        self.hellos = Arc::new(Semaphore::new(n));
    }

    /// Serves one connection to its end. The caller spawns it per stream; dropping the future
    /// closes the stream. An `Err` is a connection that ended badly (a peer that vanished, a
    /// timeout, a refused hello), which is routine on a public door: log it at debug.
    pub async fn serve<S>(&self, stream: S) -> Result<Served, TlsError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let hello_slot = self
            .hellos
            .clone()
            .try_acquire_owned()
            .map_err(|_| TlsError::Busy)?;
        let hello = tokio::time::timeout(
            self.timeouts.handshake,
            LazyConfigAcceptor::new(Acceptor::default(), stream),
        )
        .await
        .map_err(|_| TlsError::Timeout("the TLS hello"))?
        .map_err(|e| TlsError::Handshake(e.to_string()))?;

        let (config, challenge) = {
            let client = hello.client_hello();
            self.choose(client.server_name(), client.alpn())?
        };
        drop(hello_slot);
        let pool = if challenge {
            &self.validation_slots
        } else {
            &self.web_slots
        };
        let _slot = pool
            .clone()
            .try_acquire_owned()
            .map_err(|_| TlsError::Busy)?;

        let tls = tokio::time::timeout(self.timeouts.handshake, hello.into_stream(config))
            .await
            .map_err(|_| TlsError::Timeout("the TLS handshake"))?
            .map_err(|e| TlsError::Handshake(e.to_string()))?;

        if challenge {
            let mut tls = tls;
            // The peer is the CA and we are done; a stalled close must not hold the slot.
            let _ = tokio::time::timeout(self.timeouts.handshake, tls.shutdown()).await;
            return Ok(Served::Challenge);
        }
        self.serve_http(tls).await?;
        Ok(Served::Http)
    }

    /// Which config answers this hello, and whether it is the CA's validation. A hello is
    /// refused here, before any certificate is chosen, unless it names this door.
    pub(crate) fn choose<'a>(
        &self,
        sni: Option<&str>,
        alpn: Option<impl Iterator<Item = &'a [u8]>>,
    ) -> Result<(Arc<ServerConfig>, bool), TlsError> {
        if !sni_is(sni, &self.host) {
            return Err(TlsError::Refused("the server name is not ours"));
        }
        let offered: Vec<&[u8]> = alpn.map(Iterator::collect).unwrap_or_default();
        if offered == [ACME_TLS_ALPN] {
            if !self.challenge.is_armed() {
                return Err(TlsError::Refused("no challenge is pending"));
            }
            return Ok((self.validation.clone(), true));
        }
        if !self.live.is_installed() {
            return Err(TlsError::Refused("no certificate yet"));
        }
        Ok((self.web.clone(), false))
    }

    async fn serve_http<S>(&self, tls: tokio_rustls::server::TlsStream<S>) -> Result<(), TlsError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let http2 = tls.get_ref().1.alpn_protocol() == Some(H2);
        let activity = Arc::new(Activity::new());
        let service = {
            let (gate, router) = (self.gate.clone(), self.router.clone());
            let (requests, activity) = (self.requests.clone(), activity.clone());
            let request_timeout = self.timeouts.request;
            service_fn(move |req: hyper::Request<Incoming>| {
                let (gate, router) = (gate.clone(), router.clone());
                let (requests, working) = (requests.clone(), Working::begin(&activity));
                async move {
                    let _working = working;
                    let (mut parts, body) = req.into_parts();
                    if !gate.admit(&mut parts) {
                        return Ok::<_, Infallible>(not_found());
                    }
                    let Ok(_slot) = requests.try_acquire_owned() else {
                        return Ok(status(axum::http::StatusCode::SERVICE_UNAVAILABLE));
                    };
                    let body =
                        Body::new(http_body_util::Limited::new(body, MAX_REQUEST_BODY_BYTES));
                    let request = hyper::Request::from_parts(parts, body);
                    match tokio::time::timeout(request_timeout, router.oneshot(request)).await {
                        Ok(Ok(response)) => Ok(response),
                        Ok(Err(never)) => match never {},
                        Err(_) => Ok(status(axum::http::StatusCode::REQUEST_TIMEOUT)),
                    }
                }
            })
        };

        let io = TokioIo::new(tls);
        // The protocol is what ALPN settled on, not what the first bytes look like: sniffing
        // them would leave a peer that sends none with no timer running.
        let ended = if http2 {
            let conn = http2::Builder::new(TokioExecutor::new())
                .timer(TokioTimer::new())
                .max_concurrent_streams(MAX_HTTP2_STREAMS)
                .keep_alive_interval(self.timeouts.http2_keep_alive_interval)
                .keep_alive_timeout(self.timeouts.http2_keep_alive_timeout)
                .serve_connection(io, service);
            tokio::pin!(conn);
            self.drive(conn.as_mut(), |c| c.graceful_shutdown(), &activity)
                .await
        } else {
            let conn = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(self.timeouts.request_headers)
                .serve_connection(io, service);
            tokio::pin!(conn);
            self.drive(conn.as_mut(), |c| c.graceful_shutdown(), &activity)
                .await
        };
        ended.map_err(|e| TlsError::Http(e.to_string()))
    }

    /// Runs a connection until it ends, winding it down when it has been idle too long or has
    /// lived too long. A wind-down that does not finish in time drops the connection.
    async fn drive<C>(
        &self,
        mut conn: Pin<&mut C>,
        wind_down: impl Fn(Pin<&mut C>),
        activity: &Activity,
    ) -> Result<(), hyper::Error>
    where
        C: Future<Output = Result<(), hyper::Error>>,
    {
        let t = self.timeouts;
        let tick = (t.idle / 4).clamp(MIN_WATCH_TICK, WATCH_TICK);
        let started = Instant::now();
        let grace = loop {
            tokio::select! {
                ended = conn.as_mut() => return ended,
                () = tokio::time::sleep(tick) => {}
            }
            if started.elapsed() >= t.connection {
                break t.request;
            }
            if activity.idle_for().is_some_and(|idle| idle >= t.idle) {
                break t.shutdown_grace;
            }
        };
        wind_down(conn.as_mut());
        match tokio::time::timeout(grace, conn.as_mut()).await {
            Ok(ended) => ended,
            Err(_) => Ok(()),
        }
    }
}

/// When a connection last had a request in flight.
struct Activity {
    started: Instant,
    in_flight: AtomicUsize,
    /// Milliseconds since `started` at which the last request ended (0: none yet).
    last_end_ms: AtomicU64,
}

impl Activity {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            in_flight: AtomicUsize::new(0),
            last_end_ms: AtomicU64::new(0),
        }
    }

    /// How long the connection has been idle, or `None` while a request is in flight.
    fn idle_for(&self) -> Option<Duration> {
        if self.in_flight.load(Ordering::SeqCst) > 0 {
            return None;
        }
        let since = Duration::from_millis(self.last_end_ms.load(Ordering::SeqCst));
        Some(self.started.elapsed().saturating_sub(since))
    }
}

/// Marks a request in flight for as long as it is held.
struct Working(Arc<Activity>);

impl Working {
    fn begin(activity: &Arc<Activity>) -> Self {
        activity.in_flight.fetch_add(1, Ordering::SeqCst);
        Self(activity.clone())
    }
}

impl Drop for Working {
    fn drop(&mut self) {
        let now = u64::try_from(self.0.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.0.last_end_ms.store(now, Ordering::SeqCst);
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

fn status(code: axum::http::StatusCode) -> axum::response::Response {
    axum::response::IntoResponse::into_response(code)
}

/// A server config on rustls' aws-lc provider, named here because the process links two and
/// rustls panics when it must pick one.
fn server_config(
    resolver: Arc<dyn ResolvesServerCert>,
    alpn: Vec<Vec<u8>>,
) -> Result<Arc<ServerConfig>, TlsError> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Config(e.to_string()))?
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    config.alpn_protocols = alpn;
    Ok(Arc::new(config))
}
