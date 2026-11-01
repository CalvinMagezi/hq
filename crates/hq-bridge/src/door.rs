//! The edge protocol on the device's side: the streams an `hq edge` opens for the connector.
//!
//! The edge is a public forwarder that passes each client's TLS bytes to this device over one
//! iroh stream. The device ends TLS itself, in the web server's connector door, which listens on
//! a unix socket (or, as an opt-in, a loopback port). This module is the pipe between the two: it
//! takes a stream from the registered edge and copies it there. It never looks at the bytes. Which peer may open such a
//! stream (the registered edge, and no paired peer) is decided by the gate in `bridge.rs`.

use anyhow::{Result, bail};
use hq_db::Database;
use iroh::{
    EndpointId,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::Semaphore,
};

/// How often an open connection checks that its peer is still the registered edge.
pub(crate) const RECHECK: Duration = Duration::from_secs(1);

use crate::bridge::{Live, LiveGuard, REASON_NOT_EDGE, REJECT_REVOKED, still_edge};

/// The protocol name the edge speaks to a device. It is `hq_edge::EDGE_ALPN`; a test in `hq-cli`,
/// which sees both crates, fails if the two ever differ.
pub const EDGE_ALPN: &[u8] = b"hq/edge/1";

/// How many leading bytes of a device's endpoint id make its DNS label on the edge. It is
/// `hq_edge::LABEL_BYTES` (same test).
pub const LABEL_BYTES: usize = 8;

/// How long the loopback connection to the connector door may take. It is on this machine.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// A forwarded connection with no bytes in one direction for this long ends that direction. A little
/// longer than the edge's own idle timeout (120 s), so the edge decides first.
const IDLE: Duration = Duration::from_secs(150);
/// Streams piped at once. The edge caps a device at 16; this is a backstop for a misbehaving edge.
pub const DEFAULT_MAX_STREAMS: usize = 64;
const CHUNK: usize = 16 * 1024;

/// Where the edge's streams go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoorTarget {
    /// A unix socket the web server's connector door listens on, in a directory only this user
    /// can enter.
    Unix(PathBuf),
    /// A loopback address with a fixed port. Weaker: any local process can bind a port first.
    Tcp(SocketAddr),
}

/// Where the edge's streams go, and how many at once.
#[derive(Debug, Clone)]
pub struct EdgeDoor {
    /// The connector door's address (`mcp_connector.tls.door_listen`).
    pub target: DoorTarget,
    pub max_streams: usize,
    /// How often an open connection checks that its peer is still the registered edge.
    pub recheck: Duration,
}

impl EdgeDoor {
    pub fn new(target: DoorTarget) -> Self {
        Self {
            target,
            max_streams: DEFAULT_MAX_STREAMS,
            recheck: RECHECK,
        }
    }

    /// The edge may be pointed at the connector's door and nowhere else: an absolute socket path,
    /// or a loopback address with a fixed port, and a stream cap the semaphore can hold.
    pub fn validate(&self) -> Result<()> {
        check_target(&self.target, "the edge door")?;
        if self.recheck.is_zero() {
            bail!("the edge door's recheck interval must not be zero");
        }
        if self.max_streams == 0 || self.max_streams > Semaphore::MAX_PERMITS {
            bail!(
                "the edge door's stream cap must be between 1 and {}",
                Semaphore::MAX_PERMITS
            );
        }
        Ok(())
    }
}

/// A pipe may lead to an absolute socket path or a loopback address with a fixed port, nothing
/// else on this machine and nothing off it.
pub(crate) fn check_target(target: &DoorTarget, what: &str) -> Result<()> {
    match target {
        DoorTarget::Unix(path) if !path.is_absolute() || path.file_name().is_none() => {
            bail!("{what} must be an absolute socket path")
        }
        DoorTarget::Tcp(addr) if !addr.ip().is_loopback() || addr.port() == 0 => {
            bail!("{what} must be a loopback address with a fixed port")
        }
        _ => Ok(()),
    }
}

/// The edge's connection handler. A stream is accepted only while the edge is still the registered
/// one, so forgetting it ends the service for the connection already open as well.
#[derive(Clone)]
pub(crate) struct EdgeService {
    pub db: Database,
    pub live: Arc<Live>,
    pub door: EdgeDoor,
    pub permits: Arc<Semaphore>,
    /// Connections that passed the gate to this handler.
    pub reached: Arc<AtomicU64>,
    /// Streams refused because the cap was full.
    pub over_cap: Arc<AtomicU64>,
}

impl std::fmt::Debug for EdgeService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EdgeService")
    }
}

impl ProtocolHandler for EdgeService {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.reached.fetch_add(1, Ordering::SeqCst);
        let id: EndpointId = conn.remote_id();
        // Register first, then read the record: a forget that lands between the gate and here
        // either finds this connection in the registry or is seen by the read below.
        let token = self.live.register(id, conn.weak_handle());
        let _guard = LiveGuard::new(self.live.clone(), id, token);
        if !still_edge(&self.db, id).await {
            conn.close(REJECT_REVOKED.into(), REASON_NOT_EDGE);
            return Ok(());
        }
        // The first look is the one above; the next is one period away, not at once.
        let mut check = tokio::time::interval_at(
            tokio::time::Instant::now() + self.door.recheck,
            self.door.recheck,
        );
        loop {
            tokio::select! {
                stream = conn.accept_bi() => {
                    let Ok((send, recv)) = stream else { break };
                    if !still_edge(&self.db, id).await {
                        conn.close(REJECT_REVOKED.into(), REASON_NOT_EDGE);
                        break;
                    }
                    let Ok(permit) = self.permits.clone().try_acquire_owned() else {
                        // Over the cap: the stream is dropped, which the edge shows its client as a
                        // close.
                        self.over_cap.fetch_add(1, Ordering::SeqCst);
                        continue;
                    };
                    let target = self.door.target.clone();
                    tokio::spawn(async move {
                        forward(send, recv, &target).await;
                        drop(permit);
                    });
                }
                // A forget made in another process (the CLI) reaches an open connection here.
                _ = check.tick() => {
                    if !still_edge(&self.db, id).await {
                        conn.close(REJECT_REVOKED.into(), REASON_NOT_EDGE);
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Connects to the door and copies both ways until both directions end. A door that does not
/// answer closes the stream at once.
async fn forward(send: SendStream, recv: RecvStream, target: &DoorTarget) {
    if let Some(conn) = connect(target).await {
        conn.pipe(send, recv, IDLE).await;
    }
}

/// A connection to a loopback target, either kind.
pub(crate) enum Local {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
}

impl Local {
    pub(crate) async fn pipe(self, send: SendStream, recv: RecvStream, idle: Duration) {
        match self {
            Self::Tcp(tcp) => pipe(send, recv, tcp, idle).await,
            #[cfg(unix)]
            Self::Unix(unix) => pipe(send, recv, unix, idle).await,
        }
    }
}

/// Connects to `target` on this machine, or `None` if it does not answer in time.
pub(crate) async fn connect(target: &DoorTarget) -> Option<Local> {
    match target {
        DoorTarget::Tcp(addr) => {
            let connected = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await;
            let tcp = connected.ok()?.ok()?;
            let _ = tcp.set_nodelay(true);
            Some(Local::Tcp(tcp))
        }
        #[cfg(unix)]
        DoorTarget::Unix(path) => {
            let connected =
                tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::UnixStream::connect(path)).await;
            Some(Local::Unix(connected.ok()?.ok()?))
        }
        #[cfg(not(unix))]
        DoorTarget::Unix(_) => None,
    }
}

pub(crate) async fn pipe<S>(send: SendStream, recv: RecvStream, door: S, idle: Duration)
where
    S: AsyncRead + AsyncWrite + Send + Unpin,
{
    let (door_read, door_write) = tokio::io::split(door);
    let up = copy_until_idle(recv, door_write, idle);
    let down = copy_until_idle(door_read, send, idle);
    tokio::join!(up, down);
}

/// Copies `from` to `to` until `from` ends, either side fails, or no byte arrives for `idle`.
/// The writer is shut down at the end, so the other side sees the end of the data.
async fn copy_until_idle<R, W>(mut from: R, mut to: W, idle: Duration)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    loop {
        match tokio::time::timeout(idle, from.read(&mut buf)).await {
            Ok(Ok(n)) if n > 0 => {
                if to.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            _ => break,
        }
    }
    let _ = to.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn door(target: DoorTarget) -> EdgeDoor {
        EdgeDoor::new(target)
    }

    #[test]
    fn the_edge_can_only_be_pointed_at_a_socket_path_or_a_loopback_port() {
        for ok in [
            DoorTarget::Unix("/run/hq/door.sock".into()),
            DoorTarget::Tcp("127.0.0.1:5679".parse().unwrap()),
            DoorTarget::Tcp("[::1]:5679".parse().unwrap()),
        ] {
            assert!(door(ok.clone()).validate().is_ok(), "{ok:?}");
        }
        for bad in [
            DoorTarget::Unix("relative.sock".into()),
            DoorTarget::Unix("/".into()),
            DoorTarget::Tcp("0.0.0.0:5679".parse().unwrap()),
            DoorTarget::Tcp("192.168.1.9:5679".parse().unwrap()),
            DoorTarget::Tcp("93.184.216.34:443".parse().unwrap()),
            DoorTarget::Tcp("127.0.0.1:0".parse().unwrap()),
        ] {
            assert!(door(bad.clone()).validate().is_err(), "{bad:?}");
        }
        let mut capped = door(DoorTarget::Unix("/run/hq/door.sock".into()));
        capped.max_streams = 0;
        assert!(capped.validate().is_err());
        capped.max_streams = usize::MAX;
        assert!(capped.validate().is_err());
    }
}
