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
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
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
    /// ClientHello to the end of the handshake. A peer that opens a stream and says nothing
    /// is closed when this runs out.
    pub handshake: Duration,
    /// Request line and headers, once the connection is up.
    pub request_headers: Duration,
    /// One request, body and handler together. Longer than the 40 s a call waits for the
    /// owner's approval.
    pub request: Duration,
    /// A connection's whole life. When it runs out the connection is wound down gracefully (an
    /// answer in flight is finished, then it closes), so an idle HTTP/2 session cannot hold one
    /// of the door's slots for longer than this.
    pub connection: Duration,
    pub http2_keep_alive_interval: Duration,
    pub http2_keep_alive_timeout: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            handshake: Duration::from_secs(10),
            request_headers: Duration::from_secs(10),
            request: Duration::from_secs(120),
            connection: Duration::from_secs(5 * 60),
            http2_keep_alive_interval: Duration::from_secs(30),
            http2_keep_alive_timeout: Duration::from_secs(20),
        }
    }
}

/// Connections served at once. A forwarder carries one stream per client, so this bounds what
/// the whole internet can ask of the device.
pub const MAX_CONNECTIONS: usize = 64;
const MAX_HTTP2_STREAMS: u32 = 32;

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
    permits: Arc<Semaphore>,
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
            permits: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            timeouts: Timeouts::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_max_connections(&mut self, n: usize) {
        self.permits = Arc::new(Semaphore::new(n));
    }

    /// Serves one connection to its end. The caller spawns it per stream; dropping the future
    /// closes the stream. An `Err` is a connection that ended badly (a peer that vanished, a
    /// timeout, a refused hello), which is routine on a public door: log it at debug.
    pub async fn serve<S>(&self, stream: S) -> Result<Served, TlsError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let Ok(_permit) = self.permits.clone().try_acquire_owned() else {
            return Err(TlsError::Busy);
        };
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
        let tls = tokio::time::timeout(self.timeouts.handshake, hello.into_stream(config))
            .await
            .map_err(|_| TlsError::Timeout("the TLS handshake"))?
            .map_err(|e| TlsError::Handshake(e.to_string()))?;

        if challenge {
            let mut tls = tls;
            let _ = tls.shutdown().await;
            return Ok(Served::Challenge);
        }
        self.serve_http(tls).await?;
        Ok(Served::Http)
    }

    /// Which config answers this hello, and whether it is the CA's validation. A hello is
    /// refused here, before any certificate is chosen, unless it names this door.
    fn choose<'a>(
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
        let gate = self.gate.clone();
        let router = self.router.clone();
        let request_timeout = self.timeouts.request;
        let service = service_fn(move |req: hyper::Request<Incoming>| {
            let (gate, router) = (gate.clone(), router.clone());
            async move {
                let (mut parts, body) = req.into_parts();
                if !gate.admit(&mut parts) {
                    return Ok::<_, Infallible>(not_found());
                }
                let request = hyper::Request::from_parts(parts, Body::new(body));
                match tokio::time::timeout(request_timeout, router.oneshot(request)).await {
                    Ok(Ok(response)) => Ok(response),
                    Ok(Err(never)) => match never {},
                    Err(_) => Ok(request_timed_out()),
                }
            }
        });

        let mut builder = auto::Builder::new(TokioExecutor::new());
        builder
            .http1()
            .timer(TokioTimer::new())
            .header_read_timeout(self.timeouts.request_headers);
        builder
            .http2()
            .timer(TokioTimer::new())
            .max_concurrent_streams(MAX_HTTP2_STREAMS)
            .keep_alive_interval(self.timeouts.http2_keep_alive_interval)
            .keep_alive_timeout(self.timeouts.http2_keep_alive_timeout);
        let connection = builder.serve_connection(TokioIo::new(tls), service);
        tokio::pin!(connection);
        let lifetime = tokio::time::sleep(self.timeouts.connection);
        tokio::pin!(lifetime);
        let ended = tokio::select! {
            ended = connection.as_mut() => ended,
            () = &mut lifetime => {
                connection.as_mut().graceful_shutdown();
                match tokio::time::timeout(self.timeouts.request, connection.as_mut()).await {
                    Ok(ended) => ended,
                    Err(_) => return Ok(()),
                }
            }
        };
        ended.map_err(|e| TlsError::Http(e.to_string()))
    }
}

fn request_timed_out() -> axum::response::Response {
    axum::response::IntoResponse::into_response(axum::http::StatusCode::REQUEST_TIMEOUT)
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
