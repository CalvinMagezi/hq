//! The network side of the spike: the pairing exchange, the paired-only gate, and reconnecting by
//! key. One endpoint speaks two protocols: `hq/pair/1` (open only while a ticket is open) and
//! `hq/echo/1` (open only to paired keys).

use crate::pairing::{self, CODE_BYTES, Nonce, Ticket, Verdict, Window};
use anyhow::{Context, Result, anyhow, bail};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey,
    endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks, QuicTransportConfig, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, mpsc, oneshot};

pub const PAIR_ALPN: &[u8] = b"hq/pair/1";
pub const ECHO_ALPN: &[u8] = b"hq/echo/1";
const REJECT_UNPAIRED: u32 = 403;
const REJECT_CLOSED: u32 = 410;
const REJECT_BUSY: u32 = 429;
const REJECT_SLOW: u32 = 408;
const REJECT_BAD: u32 = 400;
const REJECT_REVOKED: u32 = 403;
const KEY_BYTES: usize = 32;
const MAX_LINE_BYTES: usize = 512;
const MAX_ECHO_BYTES: usize = 64 * 1024;
/// How long a pairing connection has to open its stream and finish each step of the exchange.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the person at the host has to type the digits and answer.
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(120);
/// Pairing connections served at once; more are closed on arrival.
const MAX_PAIRING_CONNECTIONS: usize = 4;
/// How often an open connection checks that its device is still on the paired list.
const REVOKE_CHECK: Duration = Duration::from_secs(1);
const PROTOCOL_VERSION: u8 = 2;
const PATH_WAIT: Duration = Duration::from_secs(10);
const PATH_POLL: Duration = Duration::from_millis(200);
const ONLINE_WAIT: Duration = Duration::from_secs(5);

/// First message from the joiner: the code, and a commitment to a nonce it reveals later.
#[derive(Debug, Serialize, Deserialize)]
struct Hello {
    v: u8,
    code: String,
    commit: String,
}

/// The host's answer: refused with a reason, or accepted with its own nonce.
#[derive(Debug, Serialize, Deserialize)]
struct Reply {
    ok: bool,
    #[serde(default)]
    why: String,
    #[serde(default)]
    nonce: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Reveal {
    nonce: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Final {
    result: String,
}

/// What a joiner stores about the host it paired with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stored {
    pub id: String,
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub addrs: Vec<String>,
    pub paired_unix: u64,
}

/// A device asking to be paired, waiting for the person at the host. The person types the digits
/// shown on the other device; `decide(true)` is sent only if they match `sas`. Pairing is recorded
/// by the handler once it receives `true` in time: a `send` that fails means it was too late.
pub struct PairRequest {
    pub joiner: EndpointId,
    pub sas: String,
    pub decide: oneshot::Sender<bool>,
}

pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if !path.exists() {
        let key = SecretKey::generate();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        std::io::Write::write_all(&mut file, &key.to_bytes())?;
        return Ok(key);
    }
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        bail!(
            "{} is readable by others (mode {:o}); chmod 600 it",
            path.display(),
            mode & 0o777
        );
    }
    let bytes: [u8; KEY_BYTES] = std::fs::read(path)?
        .try_into()
        .map_err(|_| anyhow!("{} is not a {KEY_BYTES}-byte key", path.display()))?;
    Ok(SecretKey::from_bytes(&bytes))
}

/// The ids on the paired list. A missing file is an empty list; a file that is not a plain file,
/// is owned by someone other than `owner`, or can be written by others is an error, so the gate
/// fails closed instead of trusting a list anyone could have edited.
fn trusted_ids(path: &Path, owner: u32) -> Result<Vec<EndpointId>> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    if !meta.file_type().is_file() {
        bail!("{} is not a plain file", path.display());
    }
    if meta.uid() != owner {
        bail!(
            "{} is not owned by the user running the host",
            path.display()
        );
    }
    if meta.permissions().mode() & 0o022 != 0 {
        bail!("{} can be written by others; chmod 600 it", path.display());
    }
    Ok(std::fs::read_to_string(path)?
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect())
}

fn is_paired(path: &Path, owner: u32, id: &EndpointId) -> bool {
    match trusted_ids(path, owner) {
        Ok(ids) => ids.contains(id),
        Err(e) => {
            eprintln!("  [gate] paired list refused: {e:#}");
            false
        }
    }
}

fn append_private(path: &Path, owner: u32, line: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    trusted_ids(path, owner)?;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    writeln!(file, "{line}")?;
    Ok(())
}

/// Who may connect to which protocol: anyone while a ticket is open, but only to pair; paired keys
/// to everything else; nobody else, closed before any handler runs.
#[derive(Clone, Debug)]
struct Gate {
    window: Arc<Mutex<Window>>,
    paired: PathBuf,
    owner: u32,
    refused: Arc<AtomicU64>,
}

impl EndpointHooks for Gate {
    async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        let id = conn.remote_id();
        if conn.alpn() == PAIR_ALPN {
            if self.window.lock().unwrap().is_open(Instant::now()) {
                return AfterHandshakeOutcome::accept();
            }
            self.refused.fetch_add(1, Ordering::SeqCst);
            return AfterHandshakeOutcome::Reject {
                error_code: REJECT_CLOSED.into(),
                reason: b"pairing is closed".to_vec(),
            };
        }
        if is_paired(&self.paired, self.owner, &id) {
            return AfterHandshakeOutcome::accept();
        }
        self.refused.fetch_add(1, Ordering::SeqCst);
        AfterHandshakeOutcome::Reject {
            error_code: REJECT_UNPAIRED.into(),
            reason: b"not paired".to_vec(),
        }
    }
}

async fn read_line(recv: &mut iroh::endpoint::RecvStream) -> Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if line.len() > MAX_LINE_BYTES {
            bail!("a line longer than {MAX_LINE_BYTES} bytes");
        }
        let n = recv.read(&mut byte).await?.unwrap_or(0);
        if n == 0 || byte[0] == b'\n' {
            return Ok(String::from_utf8(line)?);
        }
        line.push(byte[0]);
    }
}

async fn send_line<T: Serialize>(send: &mut iroh::endpoint::SendStream, value: &T) -> Result<()> {
    let mut text = serde_json::to_string(value)?;
    text.push('\n');
    send.write_all(text.as_bytes()).await?;
    Ok(())
}

#[derive(Clone, Debug)]
struct PairHandler {
    window: Arc<Mutex<Window>>,
    host: EndpointId,
    code: [u8; CODE_BYTES],
    requests: mpsc::Sender<PairRequest>,
    paired: PathBuf,
    owner: u32,
    hello_timeout: Duration,
    decision_timeout: Duration,
    slots: Arc<Semaphore>,
    /// How many pairing connections got past the gate to this handler.
    reached: Arc<AtomicU64>,
}

fn decode_nonce(text: &str) -> Option<Nonce> {
    data_encoding::BASE32_NOPAD
        .decode(text.as_bytes())
        .ok()?
        .try_into()
        .ok()
}

impl PairHandler {
    async fn step<T>(&self, what: &str, f: impl Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout(self.hello_timeout, f).await {
            Ok(result) => result,
            Err(_) => bail!("timed out waiting for {what}"),
        }
    }

    async fn refuse(
        &self,
        conn: &Connection,
        send: &mut iroh::endpoint::SendStream,
        why: &str,
    ) -> Result<()> {
        let reply = Reply {
            ok: false,
            why: why.into(),
            nonce: String::new(),
        };
        send_line(send, &reply).await?;
        send.finish()?;
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
        Ok(())
    }

    async fn exchange(&self, conn: &Connection) -> Result<()> {
        let joiner = conn.remote_id();
        let (mut send, mut recv) = self
            .step("a stream", async { Ok(conn.accept_bi().await?) })
            .await?;
        let hello: Hello =
            serde_json::from_str(&self.step("the hello", read_line(&mut recv)).await?)?;
        let commitment = data_encoding::BASE32_NOPAD
            .decode(hello.commit.as_bytes())
            .unwrap_or_default();
        if hello.v != PROTOCOL_VERSION || commitment.len() != pairing::COMMIT_BYTES {
            return self
                .refuse(conn, &mut send, "unsupported pairing version")
                .await;
        }
        let offered = data_encoding::BASE32_NOPAD
            .decode(hello.code.as_bytes())
            .unwrap_or_default();
        let verdict = self.window.lock().unwrap().check(Instant::now(), &offered);
        if verdict != Verdict::Accepted {
            return self.refuse(conn, &mut send, verdict.reason()).await;
        }
        let host_nonce = pairing::new_nonce();
        let reply = Reply {
            ok: true,
            why: String::new(),
            nonce: data_encoding::BASE32_NOPAD.encode(&host_nonce),
        };
        send_line(&mut send, &reply).await?;
        let reveal: Reveal =
            serde_json::from_str(&self.step("the reveal", read_line(&mut recv)).await?)?;
        let Some(joiner_nonce) =
            decode_nonce(&reveal.nonce).filter(|n| pairing::opens(&commitment, n))
        else {
            conn.close(REJECT_BAD.into(), b"bad reveal");
            bail!(
                "{} revealed a nonce that does not match its commitment",
                joiner.fmt_short()
            );
        };
        let sas = pairing::sas(&self.host, &joiner, &self.code, &joiner_nonce, &host_nonce);
        let (decide, answer) = oneshot::channel();
        self.step("the host person", async {
            self.requests
                .send(PairRequest {
                    joiner,
                    sas,
                    decide,
                })
                .await
                .map_err(|_| anyhow!("host is gone"))
        })
        .await?;
        let result = match tokio::time::timeout(self.decision_timeout, answer).await {
            Ok(Ok(true)) => self.record(conn, &joiner),
            Ok(Ok(false)) => "declined",
            _ => "timeout",
        };
        send_line(
            &mut send,
            &Final {
                result: result.into(),
            },
        )
        .await?;
        send.finish()?;
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.closed()).await;
        Ok(())
    }

    /// Writes the pairing only now, after an answer arrived in time and while the joiner is still
    /// connected, so a late answer or a vanished device leaves the list unchanged.
    fn record(&self, conn: &Connection, joiner: &EndpointId) -> &'static str {
        if conn.close_reason().is_some() {
            return "gone";
        }
        match append_private(&self.paired, self.owner, &joiner.to_string()) {
            Ok(()) => "accepted",
            Err(e) => {
                eprintln!("  [pair] could not record {}: {e:#}", joiner.fmt_short());
                "error"
            }
        }
    }
}

impl ProtocolHandler for PairHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.reached.fetch_add(1, Ordering::SeqCst);
        let Ok(_slot) = self.slots.clone().try_acquire_owned() else {
            conn.close(REJECT_BUSY.into(), b"busy");
            return Ok(());
        };
        if let Err(e) = self.exchange(&conn).await {
            eprintln!(
                "  [pair] exchange with {} failed: {}",
                conn.remote_id().fmt_short(),
                pairing::printable(&format!("{e:#}"))
            );
            conn.close(REJECT_SLOW.into(), b"pairing did not finish");
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Echo {
    paired: PathBuf,
    owner: u32,
    reached: Arc<AtomicU64>,
}

impl ProtocolHandler for Echo {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        self.reached.fetch_add(1, Ordering::SeqCst);
        let id = conn.remote_id();
        let mut check = tokio::time::interval(REVOKE_CHECK);
        loop {
            tokio::select! {
                stream = conn.accept_bi() => {
                    let Ok((mut send, mut recv)) = stream else { break };
                    if !is_paired(&self.paired, self.owner, &id) {
                        conn.close(REJECT_REVOKED.into(), b"revoked");
                        break;
                    }
                    tokio::spawn(async move {
                        if let Ok(data) = recv.read_to_end(MAX_ECHO_BYTES).await {
                            let _ = send.write_all(&data).await;
                            let _ = send.finish();
                        }
                    });
                }
                _ = check.tick() => {
                    if !is_paired(&self.paired, self.owner, &id) {
                        conn.close(REJECT_REVOKED.into(), b"revoked");
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct HostConfig {
    pub key: PathBuf,
    pub paired: PathBuf,
    pub bind: SocketAddr,
    pub relay: RelayMode,
    pub ttl: Duration,
    pub attempts: u32,
    pub advertise: Vec<SocketAddr>,
    /// Whether the ticket lists the host's addresses. Without them it holds only the id and the
    /// relay, which is smaller and shows no IP address; the joiner finds the direct path through
    /// the relay.
    pub with_addrs: bool,
    pub keepalive: Option<Duration>,
    pub hello_timeout: Duration,
    pub decision_timeout: Duration,
}

pub struct Host {
    pub endpoint: Endpoint,
    pub ticket: Ticket,
    pub requests: mpsc::Receiver<PairRequest>,
    pub refused: Arc<AtomicU64>,
    /// Pairing connections that reached the handler; the rest were closed at the gate.
    pub reached_pair: Arc<AtomicU64>,
    /// Paired connections that reached the echo handler.
    pub reached_echo: Arc<AtomicU64>,
    _router: Router,
}

fn transport(keepalive: Option<Duration>) -> Option<QuicTransportConfig> {
    keepalive.map(|every| {
        QuicTransportConfig::builder()
            .keep_alive_interval(every)
            .build()
    })
}

impl Host {
    pub async fn start(cfg: HostConfig) -> Result<Self> {
        let key = load_or_create_key(&cfg.key)?;
        let owner = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&cfg.key)?);
        let code = pairing::new_code();
        let window = Arc::new(Mutex::new(Window::new(
            code,
            Instant::now(),
            cfg.ttl,
            cfg.attempts,
        )));
        let refused = Arc::new(AtomicU64::new(0));
        let gate = Gate {
            window: window.clone(),
            paired: cfg.paired.clone(),
            owner,
            refused: refused.clone(),
        };
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(key)
            .relay_mode(cfg.relay.clone())
            .clear_ip_transports()
            .bind_addr(cfg.bind)?
            .hooks(gate);
        if let Some(config) = transport(cfg.keepalive) {
            builder = builder.transport_config(config);
        }
        let endpoint = builder.bind().await?;
        if !matches!(cfg.relay, RelayMode::Disabled) {
            let _ = tokio::time::timeout(ONLINE_WAIT, endpoint.online()).await;
        }
        let (tx, requests) = mpsc::channel(4);
        let reached_pair = Arc::new(AtomicU64::new(0));
        let reached_echo = Arc::new(AtomicU64::new(0));
        let handler = PairHandler {
            window,
            host: endpoint.id(),
            code,
            requests: tx,
            paired: cfg.paired.clone(),
            owner,
            hello_timeout: cfg.hello_timeout,
            decision_timeout: cfg.decision_timeout,
            slots: Arc::new(Semaphore::new(MAX_PAIRING_CONNECTIONS)),
            reached: reached_pair.clone(),
        };
        let echo = Echo {
            paired: cfg.paired,
            owner,
            reached: reached_echo.clone(),
        };
        let router = Router::builder(endpoint.clone())
            .accept(PAIR_ALPN, handler)
            .accept(ECHO_ALPN, echo)
            .spawn();

        let own = endpoint.addr();
        let mut addrs: Vec<String> = Vec::new();
        if cfg.with_addrs {
            addrs.extend(own.ip_addrs().map(|a| a.to_string()));
            // A socket bound to one address is reachable at it; one bound to every address is not
            // named by that, so the person gives the address to advertise (`--advertise`).
            addrs.extend(
                endpoint
                    .bound_sockets()
                    .iter()
                    .filter(|a| !a.ip().is_unspecified())
                    .map(|a| a.to_string()),
            );
            addrs.extend(cfg.advertise.iter().map(|a| a.to_string()));
        }
        addrs.sort();
        addrs.dedup();
        let relays = own.relay_urls().map(|r| r.to_string()).collect();
        let ticket = Ticket::new(endpoint.id(), &code, relays, addrs);
        Ok(Self {
            endpoint,
            ticket,
            requests,
            refused,
            reached_pair,
            reached_echo,
            _router: router,
        })
    }
}

fn target_from(id: EndpointId, relays: &[String], addrs: &[String]) -> Result<EndpointAddr> {
    let mut target = EndpointAddr::new(id);
    for addr in addrs {
        target = target.with_ip_addr(
            addr.parse()
                .with_context(|| format!("bad address {addr}"))?,
        );
    }
    for relay in relays {
        target = target.with_relay_url(
            relay
                .parse()
                .with_context(|| format!("bad relay {relay}"))?,
        );
    }
    Ok(target)
}

async fn client_endpoint(
    key: SecretKey,
    bind: SocketAddr,
    relay: RelayMode,
    keepalive: Option<Duration>,
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .relay_mode(relay)
        .clear_ip_transports()
        .bind_addr(bind)?;
    if let Some(config) = transport(keepalive) {
        builder = builder.transport_config(config);
    }
    Ok(builder.bind().await?)
}

pub struct JoinConfig {
    pub key: PathBuf,
    pub store: PathBuf,
    pub bind: SocketAddr,
    pub relay: RelayMode,
    pub keepalive: Option<Duration>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum JoinResult {
    Paired { sas: String },
    Refused(String),
    Declined { sas: String },
}

/// Pairs with the host named in `ticket`. `show_sas` runs as soon as the digits are known, before
/// the person at the host has answered.
pub async fn join(
    ticket: &Ticket,
    cfg: JoinConfig,
    show_sas: impl FnOnce(&str),
) -> Result<JoinResult> {
    let key = load_or_create_key(&cfg.key)?;
    let me = key.public();
    let host = ticket.endpoint_id()?;
    let code = ticket.code_bytes()?;
    let endpoint = client_endpoint(key, cfg.bind, cfg.relay, cfg.keepalive).await?;
    let conn = endpoint
        .connect(target_from(host, &ticket.relays, &ticket.addrs)?, PAIR_ALPN)
        .await
        .context("reaching the host")?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let joiner_nonce = pairing::new_nonce();
    send_line(
        &mut send,
        &Hello {
            v: PROTOCOL_VERSION,
            code: ticket.code.clone(),
            commit: data_encoding::BASE32_NOPAD.encode(&pairing::commit(&joiner_nonce)),
        },
    )
    .await?;
    let reply: Reply =
        serde_json::from_str(&tokio::time::timeout(HELLO_TIMEOUT, read_line(&mut recv)).await??)?;
    if !reply.ok {
        return Ok(JoinResult::Refused(pairing::printable(&reply.why)));
    }
    let host_nonce =
        decode_nonce(&reply.nonce).ok_or_else(|| anyhow!("the host sent no usable nonce"))?;
    send_line(
        &mut send,
        &Reveal {
            nonce: data_encoding::BASE32_NOPAD.encode(&joiner_nonce),
        },
    )
    .await?;
    let expected = pairing::sas(&host, &me, &code, &joiner_nonce, &host_nonce);
    show_sas(&expected);
    let last: Final = serde_json::from_str(
        &tokio::time::timeout(DECISION_TIMEOUT + HELLO_TIMEOUT, read_line(&mut recv)).await??,
    )?;
    conn.close(0u32.into(), b"done");
    if last.result != "accepted" {
        return Ok(JoinResult::Declined { sas: expected });
    }
    let stored = Stored {
        id: host.to_string(),
        relays: ticket.relays.clone(),
        addrs: ticket.addrs.clone(),
        paired_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    std::fs::write(&cfg.store, serde_json::to_vec_pretty(&stored)?)?;
    Ok(JoinResult::Paired { sas: expected })
}

pub fn load_store(path: &Path) -> Result<Stored> {
    serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .context("the store is not valid")
}

pub fn path_kind(conn: &Connection) -> &'static str {
    let paths = conn.paths();
    match paths.iter().find(|p| p.is_selected()) {
        Some(p) if p.is_ip() => "direct",
        Some(p) if p.is_relay() => "relay",
        _ => "none",
    }
}

pub fn path_rtt(conn: &Connection) -> Option<Duration> {
    conn.paths()
        .iter()
        .find(|p| p.is_selected())
        .map(|p| p.rtt())
}

/// Waits briefly for a direct path to take over from a relay path, then reports what is in use.
pub async fn settle_path(conn: &Connection) -> &'static str {
    let deadline = Instant::now() + PATH_WAIT;
    while path_kind(conn) != "direct" && Instant::now() < deadline {
        tokio::time::sleep(PATH_POLL).await;
    }
    path_kind(conn)
}

pub struct Session {
    pub endpoint: Endpoint,
    pub conn: Connection,
    pub connect_time: Duration,
}

/// Reconnects to the paired host by key alone: no code, no ticket.
pub async fn dial(cfg: &JoinConfig) -> Result<Session> {
    let key = load_or_create_key(&cfg.key)?;
    let stored = load_store(&cfg.store)?;
    let host: EndpointId = stored
        .id
        .parse()
        .map_err(|_| anyhow!("the store names no machine"))?;
    let endpoint = client_endpoint(key, cfg.bind, cfg.relay.clone(), cfg.keepalive).await?;
    let started = Instant::now();
    let conn = endpoint
        .connect(target_from(host, &stored.relays, &stored.addrs)?, ECHO_ALPN)
        .await?;
    // The gate answers after the handshake, so the first stream is where a refusal shows.
    echo(&conn, b"hello").await?;
    Ok(Session {
        endpoint,
        conn,
        connect_time: started.elapsed(),
    })
}

/// What a key that was never paired gets when it dials the host's echo protocol.
pub async fn stranger(ticket: &Ticket, bind: SocketAddr, relay: RelayMode) -> Result<String> {
    let endpoint = client_endpoint(SecretKey::generate(), bind, relay, None).await?;
    let host = ticket.endpoint_id()?;
    let conn = endpoint
        .connect(target_from(host, &ticket.relays, &ticket.addrs)?, ECHO_ALPN)
        .await?;
    match echo(&conn, b"let me in").await {
        Ok(()) => Ok("ANSWERED".into()),
        Err(e) => Ok(pairing::printable(&format!("{e:#}"))),
    }
}

/// Opens a pairing connection and then says nothing, as a stalled or hostile client would. Returns
/// how long the host took to close it.
pub async fn lurk(ticket: &Ticket, bind: SocketAddr, relay: RelayMode) -> Result<Duration> {
    let endpoint = client_endpoint(SecretKey::generate(), bind, relay, None).await?;
    let conn = endpoint
        .connect(
            target_from(ticket.endpoint_id()?, &ticket.relays, &ticket.addrs)?,
            PAIR_ALPN,
        )
        .await?;
    let started = Instant::now();
    conn.closed().await;
    Ok(started.elapsed())
}

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

    const LOOPBACK: &str = "127.0.0.1:0";

    /// A joiner that commits to one nonce and reveals another, which is what a machine grinding for
    /// matching digits would have to do.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reveal_that_does_not_match_the_commitment_never_reaches_the_person() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start(HostConfig {
            key: dir.path().join("host.key"),
            paired: dir.path().join("paired.txt"),
            bind: LOOPBACK.parse().unwrap(),
            relay: RelayMode::Disabled,
            ttl: Duration::from_secs(60),
            attempts: 5,
            advertise: vec![],
            with_addrs: true,
            keepalive: None,
            hello_timeout: HELLO_TIMEOUT,
            decision_timeout: DECISION_TIMEOUT,
        })
        .await
        .unwrap();
        let ticket = host.ticket.clone();
        let endpoint = client_endpoint(
            SecretKey::generate(),
            LOOPBACK.parse().unwrap(),
            RelayMode::Disabled,
            None,
        )
        .await
        .unwrap();
        let conn = endpoint
            .connect(
                target_from(ticket.endpoint_id().unwrap(), &ticket.relays, &ticket.addrs).unwrap(),
                PAIR_ALPN,
            )
            .await
            .unwrap();
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        let committed = pairing::new_nonce();
        let hello = Hello {
            v: PROTOCOL_VERSION,
            code: ticket.code.clone(),
            commit: data_encoding::BASE32_NOPAD.encode(&pairing::commit(&committed)),
        };
        send_line(&mut send, &hello).await.unwrap();
        let reply: Reply = serde_json::from_str(&read_line(&mut recv).await.unwrap()).unwrap();
        assert!(reply.ok);
        let other = pairing::new_nonce();
        let reveal = Reveal {
            nonce: data_encoding::BASE32_NOPAD.encode(&other),
        };
        send_line(&mut send, &reveal).await.unwrap();

        tokio::time::timeout(Duration::from_secs(10), conn.closed())
            .await
            .expect("the host kept a connection that cheated");
        assert!(
            host.requests.try_recv().is_err(),
            "the person was asked about a cheating device"
        );
        assert!(!dir.path().join("paired.txt").exists());
    }
}
