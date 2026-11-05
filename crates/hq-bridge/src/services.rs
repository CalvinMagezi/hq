//! Named services between paired machines: a paired device opens a stream on [`SVC_ALPN`], names a
//! service in its first bytes, and this machine pipes the stream to the loopback target its own
//! configuration gives that name. The device never says where a stream goes. A forward is the
//! other half: a local port whose connections each become one such stream to a peer's service.
//! Which peer may open the protocol (paired ones, and not the edge) is decided by the gate in
//! `bridge.rs`; the security notes are in `docs/security/PAIRING.md`, section 14.

use crate::bridge::{Dialer, Live, LiveGuard, peer_revoked_us, refusal, refused};
use crate::door::{DEFAULT_MAX_STREAMS, DoorTarget, Local, RECHECK, check_target, connect};
use anyhow::{Context, Result, anyhow, bail};
use hq_db::Database;
use iroh::{
    EndpointId,
    endpoint::{Connection, RecvStream, SendStream},
    protocol::{AcceptError, ProtocolHandler},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, sync::Semaphore};

/// The protocol paired machines speak for services. Kept apart from `hq/bridge/1` (the echo that
/// `hq bridge ping` uses), so a machine that offers no service does not speak it at all.
pub const SVC_ALPN: &[u8] = b"hq/svc/1";
/// Longest service name. Matches `hq_core::config::MAX_SERVICE_NAME`.
pub const MAX_NAME: usize = 32;
/// The service answered and the stream is now piped to it.
const STATUS_OK: u8 = 0;
/// No service by that name here.
const STATUS_UNKNOWN: u8 = 1;
/// The service is configured but nothing answered on its address.
const STATUS_DOWN: u8 = 2;
/// How long a stream may take to name its service.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// A direction of a service stream with no bytes for this long is ended. Long, because a browser
/// holding an event stream sends nothing upstream for as long as it listens.
// ponytail: per-direction idle; idle-on-both-directions if long-lived silent uploads ever matter.
const SERVICE_IDLE: Duration = Duration::from_secs(60 * 60);
/// How long dialing a peer may take before a forwarded connection gives up.
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a forward waits after a failed accept before it tries again.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(200);
/// Local connections one forward handles at once; more wait in the listener's backlog.
const MAX_FORWARDED: usize = 256;

/// Lowercase letters, digits and `-`, 1 to [`MAX_NAME`] bytes.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The services this machine offers its paired peers, by name.
#[derive(Debug, Clone)]
pub struct Services {
    pub targets: BTreeMap<String, DoorTarget>,
    /// Streams piped at once, over all peers.
    pub max_streams: usize,
}

impl Services {
    pub fn new(targets: BTreeMap<String, DoorTarget>) -> Self {
        Self {
            targets,
            max_streams: DEFAULT_MAX_STREAMS,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.targets.is_empty() {
            bail!("no services to offer");
        }
        for (name, target) in &self.targets {
            if !valid_name(name) {
                bail!("{name:?} is not a service name");
            }
            check_target(target, &format!("service {name}"))?;
        }
        if self.max_streams == 0 || self.max_streams > Semaphore::MAX_PERMITS {
            bail!(
                "the service stream cap must be between 1 and {}",
                Semaphore::MAX_PERMITS
            );
        }
        Ok(())
    }
}

/// The service protocol's connection handler.
#[derive(Clone)]
pub(crate) struct ServiceProtocol {
    pub db: Database,
    pub live: Arc<Live>,
    pub services: Arc<Services>,
    pub permits: Arc<Semaphore>,
    /// Streams dropped because the cap was full.
    pub over_cap: Arc<AtomicU64>,
    pub revoker: crate::bridge::Revoker,
}

impl std::fmt::Debug for ServiceProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServiceProtocol")
    }
}

impl ProtocolHandler for ServiceProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let id: EndpointId = conn.remote_id();
        // Register first, then read the list: a revoke in between is seen by one or the other.
        let token = self.live.register(id, conn.weak_handle());
        let _guard = LiveGuard::new(self.live.clone(), id, token);
        if refused(&self.db, &conn, id).await {
            return Ok(());
        }
        let mut check = tokio::time::interval_at(tokio::time::Instant::now() + RECHECK, RECHECK);
        loop {
            tokio::select! {
                stream = conn.accept_bi() => {
                    let Ok((send, recv)) = stream else { break };
                    if refused(&self.db, &conn, id).await {
                        break;
                    }
                    let Ok(permit) = self.permits.clone().try_acquire_owned() else {
                        self.over_cap.fetch_add(1, Ordering::SeqCst);
                        continue;
                    };
                    let services = self.services.clone();
                    tokio::spawn(async move {
                        serve_stream(send, recv, &services).await;
                        drop(permit);
                    });
                }
                // A revoke made in another process (the CLI) reaches open streams here.
                _ = check.tick() => {
                    if refused(&self.db, &conn, id).await {
                        break;
                    }
                }
            }
        }
        if peer_revoked_us(conn.close_reason().as_ref()) {
            self.revoker.learned(&id).await;
        }
        Ok(())
    }
}

/// Reads the service name, answers with one status byte, and pipes on success.
async fn serve_stream(mut send: SendStream, mut recv: RecvStream, services: &Services) {
    let Ok(Ok(name)) = tokio::time::timeout(HEADER_TIMEOUT, read_name(&mut recv)).await else {
        return;
    };
    let Some(target) = services.targets.get(&name) else {
        let _ = send.write_all(&[STATUS_UNKNOWN]).await;
        let _ = send.finish();
        return;
    };
    let Some(local) = connect(target).await else {
        let _ = send.write_all(&[STATUS_DOWN]).await;
        let _ = send.finish();
        return;
    };
    if send.write_all(&[STATUS_OK]).await.is_ok() {
        local.pipe(send, recv, SERVICE_IDLE).await;
    }
}

async fn read_name(recv: &mut RecvStream) -> Result<String> {
    let mut len = [0u8; 1];
    recv.read_exact(&mut len).await?;
    let len = usize::from(len[0]);
    if len == 0 || len > MAX_NAME {
        bail!("bad service name length");
    }
    let mut name = vec![0u8; len];
    recv.read_exact(&mut name).await?;
    let name = String::from_utf8(name)?;
    if !valid_name(&name) {
        bail!("bad service name");
    }
    Ok(name)
}

/// Opens a stream to `service` on the peer behind `conn` and waits for its answer.
pub async fn open_service(conn: &Connection, service: &str) -> Result<(SendStream, RecvStream)> {
    if !valid_name(service) {
        bail!("{service:?} is not a service name");
    }
    let (mut send, mut recv) = conn.open_bi().await?;
    let mut header = vec![service.len() as u8];
    header.extend_from_slice(service.as_bytes());
    send.write_all(&header).await?;
    let mut status = [0u8; 1];
    tokio::time::timeout(HEADER_TIMEOUT, recv.read_exact(&mut status))
        .await
        .context("the peer did not answer")??;
    match status[0] {
        STATUS_OK => Ok((send, recv)),
        STATUS_UNKNOWN => Err(anyhow!("the peer offers no service called {service}")),
        STATUS_DOWN => Err(anyhow!("the peer's {service} service is not answering")),
        other => Err(anyhow!(
            "the peer answered {other}, which this version does not know"
        )),
    }
}

/// Where a forward takes its local connections: a loopback port, which any local process can open,
/// or a unix socket in a private directory, which only this user's processes can.
pub enum ForwardListener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
}

impl From<TcpListener> for ForwardListener {
    fn from(listener: TcpListener) -> Self {
        Self::Tcp(listener)
    }
}

#[cfg(unix)]
impl From<tokio::net::UnixListener> for ForwardListener {
    fn from(listener: tokio::net::UnixListener) -> Self {
        Self::Unix(listener)
    }
}

impl ForwardListener {
    /// The next local connection, or `None` for one to drop.
    async fn accept(&self) -> std::io::Result<Option<Local>> {
        match self {
            Self::Tcp(listener) => {
                let (tcp, from) = listener.accept().await?;
                // Bound to loopback, so another address is a surprise and is dropped.
                if !from.ip().is_loopback() {
                    return Ok(None);
                }
                let _ = tcp.set_nodelay(true);
                Ok(Some(Local::Tcp(tcp)))
            }
            #[cfg(unix)]
            Self::Unix(listener) => Ok(Some(Local::Unix(listener.accept().await?.0))),
        }
    }
}

/// The one connection a forward keeps to its peer, and whether the peer was last found
/// unreachable.
#[derive(Default)]
struct Link {
    conn: tokio::sync::Mutex<Option<Connection>>,
    /// A fail-fast forward gave up on the peer; a background try that reaches it clears this.
    down: AtomicBool,
    /// A background try is running.
    retrying: AtomicBool,
}

/// Serves `listener` for as long as the task lives: each connection becomes one stream to
/// `service` on `peer`. One connection to the peer is kept and opened again when it breaks.
/// With `fail_fast`, a peer that does not answer within it is marked unreachable, and while it is
/// connections end at once, so a caller with a fallback (the backend chain) moves on.
pub async fn run_forward(
    dialer: Dialer,
    peer: String,
    service: String,
    listener: impl Into<ForwardListener>,
    fail_fast: Option<Duration>,
) {
    let listener = listener.into();
    let link = Arc::new(Link::default());
    let permits = Arc::new(Semaphore::new(MAX_FORWARDED));
    loop {
        let Ok(permit) = permits.clone().acquire_owned().await else {
            return;
        };
        let local = match listener.accept().await {
            Ok(Some(local)) => local,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!("bridge forward: accept failed: {e}");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
                continue;
            }
        };
        let (dialer, peer, service, link) =
            (dialer.clone(), peer.clone(), service.clone(), link.clone());
        tokio::spawn(async move {
            match open_stream(&dialer, &peer, &service, &link, fail_fast).await {
                Ok((send, recv)) => local.pipe(send, recv, SERVICE_IDLE).await,
                // Expected while the peer is away, and as often as the caller asks: not a warning.
                Err(e) if e.is::<Unreachable>() => tracing::debug!(%service, "bridge forward: {e}"),
                Err(e) => tracing::warn!(%service, "bridge forward: {e:#}"),
            }
            drop(permit);
        });
    }
}

/// The stream for one local connection. Without `fail_fast` it waits as long as a dial may take.
async fn open_stream(
    dialer: &Dialer,
    peer: &str,
    service: &str,
    link: &Arc<Link>,
    fail_fast: Option<Duration>,
) -> Result<(SendStream, RecvStream)> {
    let Some(limit) = fail_fast else {
        return stream_to(dialer, peer, service, &link.conn).await;
    };
    if link.down.load(Ordering::SeqCst) {
        retry_in_background(dialer, peer, service, link);
        return Err(Unreachable.into());
    }
    let outcome = tokio::time::timeout(limit, stream_to(dialer, peer, service, &link.conn)).await;
    if let Ok(Ok(stream)) = outcome {
        return Ok(stream);
    }
    // Only marked: other connections may be streaming over the kept connection, so closing it is
    // left to the background try, which alone uses it while the mark is set.
    link.down.store(true, Ordering::SeqCst);
    retry_in_background(dialer, peer, service, link);
    match outcome {
        Ok(Err(e)) => Err(e),
        _ => Err(anyhow!("{peer} did not answer within {limit:?}")),
    }
}

/// A fail-fast forward ended a connection because its peer is marked unreachable.
#[derive(Debug)]
struct Unreachable;

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the peer is unreachable; trying it again in the background")
    }
}

impl std::error::Error for Unreachable {}

/// The kept connection's id, if there is one.
async fn kept_id(link: &Link) -> Option<usize> {
    link.conn.lock().await.as_ref().map(Connection::stable_id)
}

/// Tries the peer's service once in the background, one try at a time, and clears the
/// unreachable mark when it answers. Tries start only when a local connection asks, so a phone
/// with nothing to send does not keep dialing.
fn retry_in_background(dialer: &Dialer, peer: &str, service: &str, link: &Arc<Link>) {
    if link.retrying.swap(true, Ordering::SeqCst) {
        return;
    }
    let (dialer, peer, service, link) = (
        dialer.clone(),
        peer.to_string(),
        service.to_string(),
        link.clone(),
    );
    tokio::spawn(async move {
        let tried = kept_id(&link).await;
        // The stream only proves the service answers; dropping it ends it on the far side.
        if stream_to(&dialer, &peer, &service, &link.conn)
            .await
            .is_ok()
        {
            link.down.store(false, Ordering::SeqCst);
        } else {
            // A kept connection that did not answer (a closed lid looks open) is not tried again:
            // the next try dials afresh. Only that one is closed, never a newer one.
            let mut slot = link.conn.lock().await;
            if tried.is_some()
                && slot.as_ref().map(Connection::stable_id) == tried
                && let Some(conn) = slot.take()
            {
                conn.close(0u32.into(), b"unanswered");
            }
        }
        link.retrying.store(false, Ordering::SeqCst);
    });
}

/// A stream to the peer's service on the kept connection, dialing again when that connection is
/// gone. The list is read every time: a revoke made in another process (the CLI) cannot reach
/// this process's connections, so the kept one is closed here once the peer is off the list.
async fn stream_to(
    dialer: &Dialer,
    peer: &str,
    service: &str,
    kept: &tokio::sync::Mutex<Option<Connection>>,
) -> Result<(SendStream, RecvStream)> {
    let mut slot = kept.lock().await;
    if let Some((code, reason)) = refusal(&dialer.standing(peer).await) {
        if let Some(conn) = slot.take() {
            conn.close(code.into(), reason);
        }
        bail!("{peer} is not a paired peer");
    }
    let alive = slot
        .as_ref()
        .filter(|c| c.close_reason().is_none())
        .cloned();
    let conn = match alive {
        Some(conn) => conn,
        None => {
            // A "revoked" refusal here is heard by the dialer, which revokes on this side.
            let conn = tokio::time::timeout(DIAL_TIMEOUT, dialer.connect_service(peer))
                .await
                .context("the peer did not answer in time")??;
            *slot = Some(conn.clone());
            conn
        }
    };
    drop(slot);
    match open_service(&conn, service).await {
        // The connection broke under us: one fresh dial. A refusal by the service is final.
        Err(_) if conn.close_reason().is_some() => {
            let fresh = tokio::time::timeout(DIAL_TIMEOUT, dialer.connect_service(peer))
                .await
                .context("the peer did not answer in time")??;
            *kept.lock().await = Some(fresh.clone());
            open_service(&fresh, service).await
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_short_lowercase_ascii() {
        for ok in ["web", "gate", "a-1", &"x".repeat(MAX_NAME)] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "Web", "a b", "a/b", "é", &"x".repeat(MAX_NAME + 1)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn services_lead_to_loopback_ports_or_socket_paths_only() {
        let one = |t: DoorTarget| Services::new(BTreeMap::from([("web".to_string(), t)]));
        assert!(
            one(DoorTarget::Tcp("127.0.0.1:5690".parse().unwrap()))
                .validate()
                .is_ok()
        );
        assert!(
            one(DoorTarget::Unix("/run/hq/g.sock".into()))
                .validate()
                .is_ok()
        );
        for bad in [
            DoorTarget::Tcp("0.0.0.0:5690".parse().unwrap()),
            DoorTarget::Tcp("10.0.0.2:22".parse().unwrap()),
            DoorTarget::Tcp("127.0.0.1:0".parse().unwrap()),
            DoorTarget::Unix("rel.sock".into()),
        ] {
            assert!(one(bad.clone()).validate().is_err(), "{bad:?}");
        }
        assert!(Services::new(BTreeMap::new()).validate().is_err());
        let bad_name = Services::new(BTreeMap::from([(
            "Web".to_string(),
            DoorTarget::Tcp("127.0.0.1:1".parse().unwrap()),
        )]));
        assert!(bad_name.validate().is_err());
    }
}
