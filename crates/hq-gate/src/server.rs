//! The listening side: accept, challenge, authenticate, hand the request to the handler.

use crate::keys::{AuthorizedKey, load_authorized};
use crate::proto::{
    AUTH_LINE_MAX, GateError, Lines, MAGIC, MAX_REQUEST_BYTES, NONCE_BYTES, PUBLIC_KEY_BYTES,
    ReadError, SIGNATURE_BYTES, decode_exact, encode, signed_message,
};
use ed25519_dalek::{Signature, VerifyingKey};
use rand::RngCore;
use std::io::Write;
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Time and count limits. The defaults are the protocol's; tests use shorter ones.
#[derive(Debug, Clone)]
pub struct Limits {
    pub auth_deadline: Duration,
    pub read_deadline: Duration,
    pub write_deadline: Duration,
    pub max_connections: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            auth_deadline: Duration::from_secs(5),
            read_deadline: Duration::from_secs(10),
            write_deadline: Duration::from_secs(10),
            max_connections: 16,
        }
    }
}

/// How long the accept loop rests after a failed accept (for example out of file descriptors),
/// so it does not spin.
const ACCEPT_ERROR_PAUSE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// An address on the tunnel interface. An unspecified address (all interfaces) and an address
    /// that is not loopback or private (a public one) are refused.
    pub listen: SocketAddr,
    /// Source addresses that may connect at all. Must not be empty.
    pub allowed_sources: Vec<IpAddr>,
    pub authorized_keys: PathBuf,
    pub limits: Limits,
}

impl ServerConfig {
    /// The checks that make it safe to bind. Callers run this before they start anything else, so
    /// a bad setting stops them early; `Listener::bind` runs it again.
    pub fn validate(&self) -> Result<(), GateError> {
        let listen = self.listen.ip().to_canonical();
        if listen.is_unspecified() {
            return Err(GateError::Config(
                "the gate must listen on one address of the tunnel interface, not on every interface"
                    .into(),
            ));
        }
        if !is_private_or_loopback(listen) {
            return Err(GateError::Config(format!(
                "{listen} is not a loopback or private address; the gate belongs on a private tunnel, never on a public address"
            )));
        }
        if self.allowed_sources.is_empty() {
            return Err(GateError::Config(
                "name at least one source address that may connect to the gate".into(),
            ));
        }
        Ok(())
    }
}

/// Loopback, RFC 1918, carrier-grade NAT (the range overlay networks such as Tailscale use),
/// link-local, and the IPv6 unique-local and link-local ranges.
fn is_private_or_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let carrier_grade_nat = v4.octets()[0] == 100 && v4.octets()[1] & 0xc0 == 0x40;
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || carrier_grade_nat
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
        }
    }
}

/// Turns a verified request line into the reply line. It is called only after the signature
/// checked out.
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, request: &str) -> String;
}

impl<F: Fn(&str) -> String + Send + Sync + 'static> Handler for F {
    fn handle(&self, request: &str) -> String {
        self(request)
    }
}

struct Shared {
    cfg: ServerConfig,
    handler: Arc<dyn Handler>,
    active: AtomicUsize,
}

/// One of the connection slots, given back when the connection's thread ends for any reason,
/// including a panic in the handler.
struct Slot(Arc<Shared>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Listener {
    listener: TcpListener,
    cfg: ServerConfig,
    stop: Arc<AtomicBool>,
}

/// Wakes a running `serve` so it returns.
#[derive(Clone)]
pub struct ShutdownHandle {
    stop: Arc<AtomicBool>,
    addr: SocketAddr,
}

impl ShutdownHandle {
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_secs(1));
    }
}

impl Listener {
    /// Binds the listener, after checking the configuration that makes it safe to bind.
    pub fn bind(cfg: ServerConfig) -> Result<Self, GateError> {
        cfg.validate()?;
        let listener = TcpListener::bind(cfg.listen)?;
        Ok(Self {
            listener,
            cfg,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr().unwrap_or(self.cfg.listen)
    }

    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            stop: self.stop.clone(),
            addr: self.local_addr(),
        }
    }

    /// Accepts connections until shut down. One thread per connection, at most
    /// `limits.max_connections` at once.
    pub fn serve(self, handler: Arc<dyn Handler>) {
        let shared = Arc::new(Shared {
            cfg: self.cfg,
            handler,
            active: AtomicUsize::new(0),
        });
        for incoming in self.listener.incoming() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = incoming else {
                std::thread::sleep(ACCEPT_ERROR_PAUSE);
                continue;
            };
            accept(&shared, stream);
        }
    }
}

/// Applies the cheap checks before a connection gets a thread, then runs it. A source outside the
/// allow-list is closed without a word, and never gets a thread.
fn accept(shared: &Arc<Shared>, stream: TcpStream) {
    let Ok(peer) = stream.peer_addr() else { return };
    if !source_allowed(&shared.cfg.allowed_sources, peer.ip()) {
        return;
    }
    if shared.active.fetch_add(1, Ordering::SeqCst) >= shared.cfg.limits.max_connections {
        shared.active.fetch_sub(1, Ordering::SeqCst);
        return;
    }
    let slot = Slot(shared.clone());
    let shared = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("hq-gate-connection".into())
        .spawn(move || {
            let _slot = slot;
            if let Err(why) = connection(&shared, stream, peer) {
                tracing::debug!(%peer, %why, "gate connection ended");
            }
        });
    if let Err(why) = spawned {
        tracing::warn!(%why, "gate: could not start a thread for a connection");
    }
}

/// Addresses compare in their canonical form, so an IPv4 peer seen as `::ffff:a.b.c.d` still
/// matches an IPv4 entry.
fn source_allowed(allowed: &[IpAddr], ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    allowed.iter().any(|a| a.to_canonical() == ip)
}

/// The outcome of the authentication half of an exchange.
enum Auth {
    Verified(String),
    Failed(&'static str),
}

fn connection(shared: &Shared, stream: TcpStream, peer: SocketAddr) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    stream.set_nodelay(true).map_err(io)?;
    stream
        .set_write_timeout(Some(shared.cfg.limits.write_deadline))
        .map_err(io)?;
    let mut writer = stream.try_clone().map_err(io)?;
    let mut nonce = [0u8; NONCE_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    writer
        .write_all(format!("{MAGIC} {}\n", encode(&nonce)).as_bytes())
        .map_err(io)?;
    let mut lines = Lines::new(stream);
    match authenticate(shared, &mut lines, &nonce, peer.ip()) {
        Auth::Failed(why) => Err(why.to_string()),
        Auth::Verified(request) => {
            let reply = shared.handler.handle(&request);
            writer.write_all(reply.as_bytes()).map_err(io)?;
            writer.write_all(b"\n").map_err(io)?;
            let _ = writer.flush();
            let _ = writer.shutdown(Shutdown::Both);
            Ok(())
        }
    }
}

/// Source and key checks first, then the request, then the signature. Nothing here parses the
/// request itself.
fn authenticate(shared: &Shared, lines: &mut Lines, nonce: &[u8; NONCE_BYTES], ip: IpAddr) -> Auth {
    let limits = &shared.cfg.limits;
    let auth_deadline = Instant::now() + limits.auth_deadline;
    let Ok(auth_line) = lines.next_line(AUTH_LINE_MAX, auth_deadline) else {
        return Auth::Failed("no usable authentication line");
    };
    let Some((key, signature)) = parse_auth_line(&auth_line) else {
        return Auth::Failed("malformed authentication line");
    };
    let Some(entry) = find_key(shared, &key, ip) else {
        return Auth::Failed("key not authorized for this source");
    };
    let read_deadline = Instant::now() + limits.read_deadline;
    let request = match lines.next_line(MAX_REQUEST_BYTES, read_deadline) {
        Ok(request) => request,
        Err(ReadError::TooLong) => return Auth::Failed("request too long"),
        Err(_) => return Auth::Failed("no request"),
    };
    let message = signed_message(nonce, &request);
    if entry.key.verify_strict(&message, &signature).is_err() {
        tracing::warn!(%ip, label = %entry.label, "gate: a listed key sent a signature that did not verify");
        return Auth::Failed("signature did not verify");
    }
    tracing::info!(%ip, label = %entry.label, "gate request accepted");
    Auth::Verified(request)
}

fn parse_auth_line(line: &str) -> Option<(VerifyingKey, Signature)> {
    let (key, signature) = line.split_once(' ')?;
    let key = VerifyingKey::from_bytes(&decode_exact::<PUBLIC_KEY_BYTES>(key)?).ok()?;
    let signature = Signature::from_bytes(&decode_exact::<SIGNATURE_BYTES>(signature)?);
    Some((key, signature))
}

/// The authorized entry for `key`, if the file lists it and allows this source. The file is read
/// on every connection, so removing a line takes effect at once. A file that cannot be read or is
/// too open authorizes nobody.
fn find_key(shared: &Shared, key: &VerifyingKey, ip: IpAddr) -> Option<AuthorizedKey> {
    let entries = match load_authorized(&shared.cfg.authorized_keys) {
        Ok(entries) => entries,
        Err(why) => {
            tracing::warn!(%why, "gate: the authorized keys file cannot be used, refusing everyone");
            return None;
        }
    };
    entries
        .into_iter()
        .find(|e| &e.key == key && (e.from.is_empty() || source_allowed(&e.from, ip)))
}
