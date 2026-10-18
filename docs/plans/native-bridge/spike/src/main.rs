//! Spike: HQ (on a server) drives a host on a laptop through an iroh connection
//! instead of ssh over a tailnet. Both roles live in this process, on separate
//! iroh endpoints with separate keys, talking over real QUIC on loopback. The
//! laptop side runs a REAL `hq_host::Host` + `Server`, and forwards through the
//! UNMODIFIED `hq_host::gate_parse_request` / `gate_forward`.

use anyhow::{Context, Result, anyhow};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, TransportAddr,
    endpoint::{Connection, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const ALPN: &[u8] = b"hq/host-gate/1";

/// The laptop side of the bridge: the replacement for `authorized_keys` +
/// `hq host gate` over ssh.
#[derive(Debug, Clone)]
struct Gate {
    host_dir: PathBuf,
    /// Endpoint ids this machine has paired with (the pinned-key list).
    paired: Arc<HashSet<EndpointId>>,
}

impl ProtocolHandler for Gate {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let peer = conn.remote_id();
        if !self.paired.contains(&peer) {
            println!("  [laptop] REFUSED unpaired peer {}", peer.fmt_short());
            conn.close(403u32.into(), b"not paired");
            return Ok(());
        }
        serve_gate(conn, self.host_dir.clone()).await;
        Ok(())
    }
}

/// Serves gate requests on every stream the peer opens on `conn`, whichever
/// side dialed. One connection, many requests: each request is its own stream,
/// so a long events.poll never blocks an agent.send_text.
async fn serve_gate(conn: Connection, dir: PathBuf) {
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        let dir = dir.clone();
        tokio::spawn(async move {
            let Ok(raw) = recv.read_to_end(hq_host::MAX_GATE_INPUT + 1).await else {
                return;
            };
            let text = String::from_utf8_lossy(&raw).into_owned();
            let reply = match hq_host::gate_parse_request(&text) {
                Ok((method, params)) => tokio::task::spawn_blocking(move || {
                    hq_host::gate_forward(&dir, &method, params)
                })
                .await
                .unwrap_or_else(|e| {
                    json!({"error":{"code":"internal","message":e.to_string()}}).to_string()
                }),
                Err(hq_host::Denied(why)) => {
                    json!({"error":{"code":"denied","message":why}}).to_string()
                }
            };
            let _ = send.write_all(reply.as_bytes()).await;
            let _ = send.finish();
        });
    }
}

/// The hub side of the inverted bridge: HQ listens, devices dial in, and HQ
/// keeps the connection to call back down it. HQ never needs the device's address.
const HUB_ALPN: &[u8] = b"hq/hub/1";

#[derive(Debug, Clone)]
struct Hub {
    known: Arc<HashSet<EndpointId>>,
    live: Arc<Mutex<HashMap<EndpointId, Connection>>>,
}

impl ProtocolHandler for Hub {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let id = conn.remote_id();
        if !self.known.contains(&id) {
            println!("  [hub] REFUSED unknown device {}", id.fmt_short());
            conn.close(403u32.into(), b"not paired");
            return Ok(());
        }
        self.live.lock().unwrap().insert(id, conn.clone());
        conn.closed().await; // hold the connection for HQ to call down
        self.live.lock().unwrap().remove(&id);
        Ok(())
    }
}

/// The server side: one request over a fresh stream on an open connection.
async fn call(conn: &Connection, method: &str, params: Value) -> Result<Value> {
    let (mut send, mut recv) = conn.open_bi().await?;
    let req = serde_json::to_string(&[method.to_string(), params.to_string()])?;
    send.write_all(req.as_bytes()).await?;
    send.finish()?;
    let raw = recv.read_to_end(2_000_000).await?;
    let v: Value = serde_json::from_slice(&raw)?;
    if let Some(e) = v.get("error") {
        return Err(anyhow!("{} / {}", e["code"], e["message"]));
    }
    Ok(v["result"].clone())
}

fn direct_addr(ep: &Endpoint) -> EndpointAddr {
    let mut addr = EndpointAddr::new(ep.id());
    for s in ep.bound_sockets() {
        // 0.0.0.0 is not dialable; use loopback with the bound port.
        let sa = std::net::SocketAddr::new(std::net::Ipv4Addr::LOCALHOST.into(), s.port());
        addr = addr.with_ip_addr(sa);
    }
    let _ = TransportAddr::Ip; // keep import honest across minor API shifts
    addr
}

async fn endpoint(sk: SecretKey) -> Result<Endpoint> {
    Ok(Endpoint::builder(presets::Minimal)
        .secret_key(sk)
        .relay_mode(RelayMode::Disabled) // sandbox has no route to any relay
        .bind()
        .await?)
}

#[tokio::main]
async fn main() -> Result<()> {
    // ---- the laptop's real host ----
    let dir = tempfile::tempdir()?;
    let host_dir = dir.path().join("host");
    std::fs::create_dir_all(&host_dir)?;
    let host = Arc::new(hq_host::Host::new().with_require_sandbox(false));
    let server = hq_host::Server::bind(&host_dir, host)?;
    std::thread::spawn(move || server.serve());
    println!("[laptop] real hq-host serving at {}", host_dir.display());

    // ---- baseline: the gate straight into the host, no network at all ----
    let t = Instant::now();
    for _ in 0..20 {
        hq_host::gate_forward(&host_dir, "host.status", json!({}));
    }
    println!(
        "[0] baseline, gate_forward on the same machine, no transport: {:?} per call",
        t.elapsed() / 20
    );

    // ---- identities: pairing = laptop learns the server's EndpointId ----
    let laptop_key = SecretKey::generate();
    let hq_key = SecretKey::generate();
    let stranger_key = SecretKey::generate();
    let paired: HashSet<EndpointId> = [hq_key.public()].into_iter().collect();
    println!("[pairing] laptop id  {}", laptop_key.public().fmt_short());
    println!(
        "[pairing] HQ id      {}  (the only key the laptop trusts)",
        hq_key.public().fmt_short()
    );

    let laptop_ep = endpoint(laptop_key).await?;
    let laptop_addr = direct_addr(&laptop_ep);
    let _router = Router::builder(laptop_ep)
        .accept(
            ALPN,
            Gate {
                host_dir: host_dir.clone(),
                paired: Arc::new(paired),
            },
        )
        .spawn();

    // ---- HQ (server) dials the laptop BY KEY ----
    let hq_ep = endpoint(hq_key).await?;
    let t = Instant::now();
    let conn = hq_ep.connect(laptop_addr.clone(), ALPN).await?;
    println!(
        "\n[1] HQ -> laptop: encrypted QUIC connection up in {:?}, peer authenticated as {}",
        t.elapsed(),
        conn.remote_id().fmt_short()
    );

    let st = call(&conn, "host.status", json!({})).await?;
    println!(
        "[2] host.status through the bridge -> protocol {} host {} agents {}",
        st["protocol_version"], st["host_version"], st["agents"]
    );

    // ---- a real PTY agent on the laptop, spawned through the bridge ----
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work)?;
    let info = call(
        &conn,
        "agent.spawn",
        json!({
            "name": "demo", "argv": ["sh","-c","echo hello-from-the-laptop; sleep 30"],
            "cwd": work.display().to_string(), "rows": 24, "cols": 80
        }),
    )
    .await?;
    println!(
        "[3] agent.spawn -> name {} pid {} status {}",
        info["name"], info["pid"], info["status"]
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    let screen = call(
        &conn,
        "agent.read",
        json!({"name":"demo","source":"recent"}),
    )
    .await?;
    println!(
        "[4] agent.read  -> {:?}",
        screen["text"].as_str().unwrap_or("").trim()
    );
    call(
        &conn,
        "agent.send_text",
        json!({"name":"demo","text":"typed-from-hq\n"}),
    )
    .await?;

    // ---- multiplexing: a long poll does not block other calls ----
    let seq = call(&conn, "events.poll", json!({})).await?["last_seq"]
        .as_u64()
        .unwrap_or(0);
    let c2 = conn.clone();
    let poll = tokio::spawn(async move {
        call(
            &c2,
            "events.poll",
            json!({"after": seq, "timeout_ms": 8000}),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let t = Instant::now();
    let _ = call(&conn, "host.status", json!({})).await?; // answered while the poll is parked
    let parked = t.elapsed();
    call(
        &conn,
        "agent.spawn",
        json!({"name":"second","argv":["sh","-c","true"],"cwd":work.display().to_string()}),
    )
    .await?;
    let ev = poll.await??;
    println!(
        "[5] long-poll parked on the same connection; a status call answered in {parked:?} meanwhile; event delivered: {}",
        ev["events"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    // ---- the allowlist still holds over the new transport ----
    for m in ["host.stop", "agent.report", "host.unlock"] {
        let r = call(&conn, m, json!({})).await;
        println!(
            "[6] {m:<13} -> {}",
            r.err()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "ALLOWED (bad!)".into())
        );
    }

    // ---- an unpaired key is refused ----
    let stranger = endpoint(stranger_key).await?;
    let sc = stranger.connect(laptop_addr.clone(), ALPN).await?; // QUIC handshake succeeds; app layer closes it
    let r = call(&sc, "host.status", json!({})).await;
    println!(
        "[7] unpaired key calling host.status -> {}",
        r.err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "ALLOWED (bad!)".into())
    );

    // ---- latency over loopback = protocol overhead only ----
    let n = 300;
    let t = Instant::now();
    for _ in 0..n {
        call(&conn, "host.status", json!({}))
            .await
            .context("bench")?;
    }
    let per = t.elapsed() / n;
    println!(
        "[8] {n} sequential host.status calls: {per:?} each (loopback; compare ~ssh RTT + 40ms gate spawn today)"
    );

    // ================= Part B: the inverted bridge =================
    // The laptop dials OUT to HQ. HQ is never told the laptop's address, so
    // this is the shape that survives NAT / CGNAT on the laptop's side.
    println!("\n--- inverted: laptop dials HQ, HQ calls back down that connection ---");
    let hub_key = SecretKey::generate();
    let dev_key = SecretKey::generate();
    let dev_id = dev_key.public();
    let hub_ep = endpoint(hub_key).await?;
    let hub_addr = direct_addr(&hub_ep);
    let live = Arc::new(Mutex::new(HashMap::new()));
    let _hub = Router::builder(hub_ep)
        .accept(
            HUB_ALPN,
            Hub {
                known: Arc::new([dev_id].into_iter().collect()),
                live: live.clone(),
            },
        )
        .spawn();

    let dev_ep = endpoint(dev_key).await?;
    let out = dev_ep.connect(hub_addr.clone(), HUB_ALPN).await?;
    tokio::spawn(serve_gate(out, host_dir.clone())); // laptop serves what HQ sends down
    let down = loop {
        if let Some(c) = live.lock().unwrap().get(&dev_id).cloned() {
            break c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let st = call(&down, "host.status", json!({})).await?;
    let list = call(&down, "agent.list", json!({})).await?;
    println!(
        "[B1] HQ -> laptop over the laptop's own outbound connection: host {} agents listed {}",
        st["host_version"],
        list["agents"].as_array().map(|a| a.len()).unwrap_or(0)
    );
    let denied = call(&down, "host.stop", json!({})).await;
    println!(
        "[B2] host.stop down the same pipe -> {}",
        denied
            .err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "ALLOWED (bad!)".into())
    );
    let t = Instant::now();
    for _ in 0..100 {
        call(&down, "host.status", json!({})).await?;
    }
    println!(
        "[B3] 100 calls HQ->laptop over the inverted connection: {:?} each",
        t.elapsed() / 100
    );
    let intruder = endpoint(SecretKey::generate()).await?;
    let ic = intruder.connect(hub_addr, HUB_ALPN).await?;
    ic.closed().await;
    println!("[B4] unknown device dialing the hub was closed by the hub");

    call(&conn, "agent.kill", json!({"name":"demo"})).await.ok();
    conn.close(0u32.into(), b"bye");
    hq_ep.close().await;
    Ok(())
}
