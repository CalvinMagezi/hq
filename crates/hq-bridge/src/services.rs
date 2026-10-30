//! Named services between paired machines: a paired device opens a stream on [`SVC_ALPN`], names a
//! service in its first bytes, and this machine pipes the stream to the loopback target its own
//! configuration gives that name. The device never says where a stream goes. A forward is the
//! other half: a local port whose connections each become one such stream to a peer's service.
//! Which peer may open the protocol (paired ones, and not the edge) is decided by the gate in
//! `bridge.rs`; the security notes are in `docs/security/PAIRING.md`, section 14.

use crate::bridge::{Dialer, Live, LiveGuard, REJECT_REVOKED, still_paired};
use crate::door::{DEFAULT_MAX_STREAMS, DoorTarget, RECHECK, check_target, connect, pipe};
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
        atomic::{AtomicU64, Ordering},
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
        if !still_paired(&self.db, id).await {
            conn.close(REJECT_REVOKED.into(), b"revoked");
            return Ok(());
        }
        let mut check = tokio::time::interval_at(tokio::time::Instant::now() + RECHECK, RECHECK);
        loop {
            tokio::select! {
                stream = conn.accept_bi() => {
                    let Ok((send, recv)) = stream else { break };
                    if !still_paired(&self.db, id).await {
                        conn.close(REJECT_REVOKED.into(), b"revoked");
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
                    if !still_paired(&self.db, id).await {
                        conn.close(REJECT_REVOKED.into(), b"revoked");
                        break;
                    }
                }
            }
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
        local.pipe(send, recv).await;
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

/// Serves `listener` (a loopback port) for as long as the task lives: each connection becomes one
/// stream to `service` on `peer`. One connection to the peer is kept and opened again when it
/// breaks.
pub async fn run_forward(dialer: Dialer, peer: String, service: String, listener: TcpListener) {
    let conn: Arc<tokio::sync::Mutex<Option<Connection>>> = Arc::default();
    loop {
        let (tcp, from) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::warn!("bridge forward: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        // Bound to loopback, so another address is a surprise and is dropped.
        if !from.ip().is_loopback() {
            continue;
        }
        let (dialer, peer, service, conn) =
            (dialer.clone(), peer.clone(), service.clone(), conn.clone());
        tokio::spawn(async move {
            let _ = tcp.set_nodelay(true);
            match stream_to(&dialer, &peer, &service, &conn).await {
                Ok((send, recv)) => pipe(send, recv, tcp).await,
                Err(e) => tracing::warn!(%service, "bridge forward: {e:#}"),
            }
        });
    }
}

/// A stream to the peer's service on the kept connection, dialing again once if it broke.
async fn stream_to(
    dialer: &Dialer,
    peer: &str,
    service: &str,
    kept: &tokio::sync::Mutex<Option<Connection>>,
) -> Result<(SendStream, RecvStream)> {
    let mut slot = kept.lock().await;
    if let Some(conn) = slot.as_ref().filter(|c| c.close_reason().is_none()) {
        let conn = conn.clone();
        drop(slot);
        if let Ok(stream) = open_service(&conn, service).await {
            return Ok(stream);
        }
        slot = kept.lock().await;
    }
    let conn = dialer.connect_service(peer).await?;
    *slot = Some(conn.clone());
    drop(slot);
    open_service(&conn, service).await
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
