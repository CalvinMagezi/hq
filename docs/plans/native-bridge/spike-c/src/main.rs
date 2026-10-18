//! Spike C: the hub and device roles of the HQ bridge over real networks.
//!
//! `hub` is HQ's side: it binds one fixed UDP port, keeps a persistent key, accepts only paired
//! device ids and calls `host.status` down each live connection. `device` is the laptop's side:
//! it runs a real `hq_host::Host`, dials the hub by key and address, serves the host gate on that
//! connection and reconnects with backoff. `stranger` dials with a key nobody paired, to show
//! what an unpaired peer gets.
//!
//! Two filters sit in front of the protocol: QUIC address validation (a retry token, so spoofed
//! source addresses cost the hub almost nothing) and a hook that refuses any peer id that is not
//! paired as soon as the TLS handshake names it, before any protocol handler runs.

use anyhow::{Context, Result, anyhow, bail};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey,
    endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks, Incoming, IncomingAddr, presets},
    protocol::{AcceptError, IncomingFilterOutcome, ProtocolHandler, Router},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const HUB_ALPN: &[u8] = b"hq/hub/1";
const REJECT_CODE: u32 = 403;
const SUPERSEDED_CODE: u32 = 409;
/// One measurement round may not take longer than this, so a dead peer cannot stall the others.
const ROUND_TIMEOUT: Duration = Duration::from_secs(60);
const KEY_BYTES: usize = 32;
const MAX_REPLY_BYTES: usize = 2_000_000;
const DEFAULT_STATUS_SECS: u64 = 30;
const PATH_WAIT: Duration = Duration::from_secs(10);
const PATH_POLL: Duration = Duration::from_millis(200);
const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A connection that lived this long counts as healthy, so the next retry starts fast again.
const HEALTHY_AFTER: Duration = Duration::from_secs(20);

/// Which peer ids may talk to this endpoint.
#[derive(Debug, Clone)]
enum Allowed {
    /// A file with one id per line, read on every handshake so a pairing needs no restart.
    File(PathBuf),
    One(EndpointId),
    Anyone,
}

#[derive(Debug, Clone)]
struct PairedOnly {
    allowed: Allowed,
    rejected: Arc<AtomicU64>,
}

impl PairedOnly {
    fn allows(&self, id: &EndpointId) -> bool {
        match &self.allowed {
            Allowed::Anyone => true,
            Allowed::One(one) => one == id,
            Allowed::File(path) => load_ids(path).contains(id),
        }
    }
}

impl EndpointHooks for PairedOnly {
    async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        let id = conn.remote_id();
        if self.allows(&id) {
            return AfterHandshakeOutcome::accept();
        }
        self.rejected.fetch_add(1, Ordering::SeqCst);
        println!(
            "  [guard] refused unpaired {} right after the handshake",
            id.fmt_short()
        );
        AfterHandshakeOutcome::Reject {
            error_code: REJECT_CODE.into(),
            reason: b"not paired".to_vec(),
        }
    }
}

fn load_ids(path: &Path) -> HashSet<EndpointId> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|l| l.trim().parse::<EndpointId>().ok())
        .collect()
}

/// Asks for proof of the source address before doing any further work on an IP connection.
fn address_validation(incoming: &Incoming) -> IncomingFilterOutcome {
    match incoming.remote_addr() {
        IncomingAddr::Ip(_) if !incoming.remote_addr_validated() => IncomingFilterOutcome::Retry,
        _ => IncomingFilterOutcome::Accept,
    }
}

/// Reads the key from `path`, or makes one there with mode 0600.
fn load_or_create_key(path: &Path) -> Result<SecretKey> {
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

fn path_kind(conn: &Connection) -> &'static str {
    let paths = conn.paths();
    let selected = paths.iter().find(|p| p.is_selected());
    match selected {
        Some(p) if p.is_ip() => "direct",
        Some(p) if p.is_relay() => "relay",
        _ => "none",
    }
}

/// The transport's own round-trip time on the selected path, apart from anything the host adds.
fn path_rtt(conn: &Connection) -> Option<Duration> {
    let paths = conn.paths();
    paths.iter().find(|p| p.is_selected()).map(|p| p.rtt())
}

/// Waits briefly for a direct path to take over from a relay path, then reports what is in use.
async fn settle_path(conn: &Connection) -> &'static str {
    let deadline = Instant::now() + PATH_WAIT;
    while path_kind(conn) != "direct" && Instant::now() < deadline {
        tokio::time::sleep(PATH_POLL).await;
    }
    path_kind(conn)
}

/// One gate request over a fresh stream of an open connection.
async fn call(conn: &Connection, method: &str, params: Value) -> Result<Value> {
    let (mut send, mut recv) = conn.open_bi().await?;
    let request = serde_json::to_string(&[method.to_string(), params.to_string()])?;
    send.write_all(request.as_bytes()).await?;
    send.finish()?;
    let raw = recv.read_to_end(MAX_REPLY_BYTES).await?;
    let reply: Value = serde_json::from_slice(&raw)?;
    if let Some(e) = reply.get("error") {
        return Err(anyhow!("{} / {}", e["code"], e["message"]));
    }
    Ok(reply["result"].clone())
}

/// Serves gate requests on every stream the peer opens, whichever side dialed.
async fn serve_gate(conn: Connection, dir: PathBuf) {
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        let dir = dir.clone();
        tokio::spawn(async move {
            let Ok(raw) = recv.read_to_end(hq_host::MAX_GATE_INPUT + 1).await else {
                return;
            };
            let text = String::from_utf8_lossy(&raw).into_owned();
            let reply = match hq_host::gate_parse_request(&text) {
                Err(hq_host::Denied(why)) => {
                    json!({"error":{"code":"denied","message":why}}).to_string()
                }
                Ok((method, params)) => tokio::task::spawn_blocking(move || {
                    hq_host::gate_forward(&dir, &method, params)
                })
                .await
                .unwrap_or_else(|e| {
                    json!({"error":{"code":"internal","message":e.to_string()}}).to_string()
                }),
            };
            let _ = send.write_all(reply.as_bytes()).await;
            let _ = send.finish();
        });
    }
}

#[derive(Debug, Clone)]
struct Hub {
    live: Arc<Mutex<HashMap<EndpointId, Connection>>>,
    reached_handler: Arc<AtomicU64>,
}

impl ProtocolHandler for Hub {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let id = conn.remote_id();
        self.reached_handler.fetch_add(1, Ordering::SeqCst);
        // A device that reconnects supersedes its old connection, which may be a dead one
        // that has not timed out yet. Closing it keeps one live connection per device.
        let previous = self.live.lock().unwrap().insert(id, conn.clone());
        if let Some(old) = previous {
            old.close(SUPERSEDED_CODE.into(), b"replaced by a newer connection");
        }
        println!("  [hub] device {} connected", id.fmt_short());
        let why = conn.closed().await;
        // Only remove our own entry: a newer connection may have taken the slot.
        let mut live = self.live.lock().unwrap();
        if live
            .get(&id)
            .is_some_and(|c| c.stable_id() == conn.stable_id())
        {
            live.remove(&id);
        }
        println!("  [hub] device {} gone: {why}", id.fmt_short());
        Ok(())
    }
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn need(args: &[String], name: &str) -> Result<String> {
    opt(args, name).ok_or_else(|| anyhow!("missing {name}"))
}

fn relay_mode(args: &[String]) -> Result<RelayMode> {
    match opt(args, "--relay").as_deref() {
        None | Some("none") => Ok(RelayMode::Disabled),
        // n0's public relays are for development and testing; use them only to compare.
        Some("default") => Ok(RelayMode::Default),
        Some(other) => bail!("--relay is none or default, not {other}"),
    }
}

async fn build_endpoint(
    key: SecretKey,
    bind: &str,
    relay: RelayMode,
    hook: PairedOnly,
) -> Result<Endpoint> {
    Ok(Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .relay_mode(relay)
        .clear_ip_transports()
        .bind_addr(bind)?
        .hooks(hook)
        .bind()
        .await?)
}

fn percentile(sorted: &[Duration], p: usize) -> Duration {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

/// `n` sequential `host.status` calls, sorted by duration.
async fn measure(conn: &Connection, n: usize) -> Result<Vec<Duration>> {
    let mut times = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        call(conn, "host.status", json!({})).await?;
        times.push(t.elapsed());
    }
    times.sort();
    Ok(times)
}

async fn report_devices(live: &Arc<Mutex<HashMap<EndpointId, Connection>>>, calls: usize) {
    let devices: Vec<(EndpointId, Connection)> = live
        .lock()
        .unwrap()
        .iter()
        .map(|(k, v)| (*k, v.clone()))
        .collect();
    let rounds: Vec<_> = devices
        .into_iter()
        .map(|(id, conn)| tokio::spawn(report_one(id, conn, calls)))
        .collect();
    for round in rounds {
        let _ = round.await;
    }
}

async fn report_one(id: EndpointId, conn: Connection, calls: usize) {
    match tokio::time::timeout(ROUND_TIMEOUT, measure(&conn, calls)).await {
        Ok(Ok(t)) => println!(
            "[hub] device {} path={} quic_rtt={:?} calls={} p50={:?} p95={:?} max={:?}",
            id.fmt_short(),
            path_kind(&conn),
            path_rtt(&conn).unwrap_or_default(),
            t.len(),
            percentile(&t, 50),
            percentile(&t, 95),
            t[t.len() - 1]
        ),
        Ok(Err(e)) => println!("[hub] device {} status failed: {e:#}", id.fmt_short()),
        Err(_) => println!(
            "[hub] device {} did not answer within {ROUND_TIMEOUT:?}",
            id.fmt_short()
        ),
    }
}

async fn hub(args: &[String]) -> Result<()> {
    let port: u16 = need(args, "--port")?.parse().context("--port")?;
    let key = load_or_create_key(Path::new(&need(args, "--key")?))?;
    let paired = PathBuf::from(need(args, "--paired")?);
    let calls: usize = opt(args, "--calls")
        .map_or(Ok(1), |s| s.parse())
        .context("--calls")?;
    let every = Duration::from_secs(
        opt(args, "--every")
            .map_or(Ok(DEFAULT_STATUS_SECS), |s| s.parse())
            .context("--every")?,
    );
    let hook = PairedOnly {
        allowed: Allowed::File(paired),
        rejected: Arc::default(),
    };
    let rejected = hook.rejected.clone();
    let ep = build_endpoint(key, &format!("0.0.0.0:{port}"), relay_mode(args)?, hook).await?;
    let handler = Hub {
        live: Arc::default(),
        reached_handler: Arc::default(),
    };
    let (live, reached) = (handler.live.clone(), handler.reached_handler.clone());
    let _router = Router::builder(ep.clone())
        .incoming_filter(Arc::new(address_validation))
        .accept(HUB_ALPN, handler)
        .spawn();
    println!("hub id: {}", ep.id());
    println!("hub listening on UDP {port}; devices dial <this host's public ip>:{port}");
    loop {
        tokio::time::sleep(every).await;
        report_devices(&live, calls).await;
        println!(
            "[hub] refused after handshake: {}, reached the protocol handler: {}",
            rejected.load(Ordering::SeqCst),
            reached.load(Ordering::SeqCst)
        );
    }
}

fn hub_addr(args: &[String]) -> Result<EndpointAddr> {
    let id: EndpointId = need(args, "--hub")?
        .parse()
        .map_err(|e| anyhow!("--hub: {e}"))?;
    let sock: SocketAddr = need(args, "--addr")?.parse().context("--addr is ip:port")?;
    Ok(EndpointAddr::new(id).with_ip_addr(sock))
}

async fn device(args: &[String]) -> Result<()> {
    let target = hub_addr(args)?;
    let key = load_or_create_key(Path::new(&need(args, "--key")?))?;
    println!(
        "device id: {}   (add this line to the hub's paired file)",
        key.public()
    );
    let dir = tempfile::tempdir()?;
    let host_dir = dir.path().join("host");
    std::fs::create_dir_all(&host_dir)?;
    let host = Arc::new(hq_host::Host::new().with_require_sandbox(false));
    let server = hq_host::Server::bind(&host_dir, host)?;
    std::thread::spawn(move || server.serve());
    let hook = PairedOnly {
        allowed: Allowed::One(target.id),
        rejected: Arc::default(),
    };
    let ep = build_endpoint(key, "0.0.0.0:0", relay_mode(args)?, hook).await?;
    let mut backoff = BACKOFF_START;
    loop {
        let started = Instant::now();
        match ep.connect(target.clone(), HUB_ALPN).await {
            Err(e) => println!("[device] cannot reach the hub: {e:#}; retry in {backoff:?}"),
            Ok(conn) => {
                println!("[device] connected in {:?}", started.elapsed());
                let watcher = conn.clone();
                tokio::spawn(async move {
                    println!("[device] path={}", settle_path(&watcher).await);
                });
                let lived = Instant::now();
                serve_gate(conn, host_dir.clone()).await;
                println!("[device] connection ended after {:?}", lived.elapsed());
                if lived.elapsed() >= HEALTHY_AFTER {
                    backoff = BACKOFF_START;
                }
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Dials with a key nobody paired and says what happened.
async fn stranger(args: &[String]) -> Result<()> {
    let target = hub_addr(args)?;
    let hook = PairedOnly {
        allowed: Allowed::Anyone,
        rejected: Arc::default(),
    };
    let ep = build_endpoint(SecretKey::generate(), "0.0.0.0:0", relay_mode(args)?, hook).await?;
    let started = Instant::now();
    let conn = match ep.connect(target, HUB_ALPN).await {
        Ok(c) => c,
        Err(e) => {
            println!(
                "[stranger] refused while connecting after {:?}: {e:#}",
                started.elapsed()
            );
            return Ok(());
        }
    };
    println!(
        "[stranger] handshake completed in {:?}; asking for host.status",
        started.elapsed()
    );
    match call(&conn, "host.status", json!({})).await {
        Ok(_) => bail!("the hub answered an unpaired peer"),
        Err(e) => println!("[stranger] no answer, as it should be: {e:#}"),
    }
    println!(
        "[stranger] connection closed by hub: {:?}",
        conn.close_reason()
    );
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let role = args.first().map(String::as_str).unwrap_or("");
    match role {
        "hub" => hub(&args).await,
        "device" => device(&args).await,
        "stranger" => stranger(&args).await,
        _ => bail!(
            "usage: bridgespike hub --port P --key FILE --paired FILE [--relay none|default] [--calls N] [--every SECS]\n       bridgespike device --hub ID --addr IP:PORT --key FILE [--relay none|default]\n       bridgespike stranger --hub ID --addr IP:PORT [--relay none|default]"
        ),
    }
}
