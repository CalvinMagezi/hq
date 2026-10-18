//! Spike D: the smallest program that shows iroh building and dialing on a phone OS.
//!
//! It plays the device role of Spike C without the coding-agent host, which a phone never runs:
//! it dials the hub by id and address, then answers whatever the hub asks down the same connection
//! with a canned `host.status`. Run it in the iOS simulator or on an Android emulator against a
//! hub started with `bridgespike hub` (see ../spike-c).

use anyhow::{Context, Result, anyhow};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey,
    endpoint::{Connection, presets},
};
use std::{
    net::SocketAddr,
    path::Path,
    time::{Duration, Instant},
};

const HUB_ALPN: &[u8] = b"hq/hub/1";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const DEFAULT_SECS: u64 = 60;
const PATH_WAIT: Duration = Duration::from_secs(5);
const PATH_POLL: Duration = Duration::from_millis(200);

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Reads a 32-byte key from `path`, or makes one there, so the hub can be told this device's id.
fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    if !path.exists() {
        let key = SecretKey::generate();
        std::fs::write(path, key.to_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        return Ok(key);
    }
    let bytes: [u8; 32] = std::fs::read(path)?
        .try_into()
        .map_err(|_| anyhow!("{} is not a 32-byte key", path.display()))?;
    Ok(SecretKey::from_bytes(&bytes))
}

fn path_kind(conn: &Connection) -> &'static str {
    let paths = conn.paths();
    match paths.iter().find(|p| p.is_selected()) {
        Some(p) if p.is_ip() => "direct",
        Some(p) if p.is_relay() => "relay",
        _ => "none",
    }
}

/// Answers every stream the hub opens with a canned `host.status`.
async fn answer(conn: Connection) {
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        tokio::spawn(async move {
            if recv.read_to_end(MAX_REQUEST_BYTES).await.is_err() {
                return;
            }
            let reply = serde_json::json!({"result": {
                "protocol_version": 1,
                "host_version": "mobile-hello",
                "platform": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "agents": 0
            }});
            let _ = send.write_all(reply.to_string().as_bytes()).await;
            let _ = send.finish();
        });
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hub: EndpointId = opt(&args, "--hub")
        .ok_or_else(|| {
            anyhow!("usage: mobilehello --hub ID --addr IP:PORT [--key FILE] [--secs N]")
        })?
        .parse()
        .map_err(|e| anyhow!("--hub: {e}"))?;
    let addr: SocketAddr = opt(&args, "--addr")
        .ok_or_else(|| anyhow!("missing --addr"))?
        .parse()
        .context("--addr is ip:port")?;
    let secs: u64 = opt(&args, "--secs").map_or(Ok(DEFAULT_SECS), |s| s.parse())?;
    let key = match opt(&args, "--key") {
        Some(path) => load_or_create_key(Path::new(&path))?,
        None => SecretKey::generate(),
    };
    println!(
        "platform={} arch={} device id: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        key.public()
    );
    let ep = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;
    let started = Instant::now();
    let conn = ep
        .connect(EndpointAddr::new(hub).with_ip_addr(addr), HUB_ALPN)
        .await?;
    println!("connected in {:?}", started.elapsed());
    let deadline = Instant::now() + PATH_WAIT;
    while path_kind(&conn) != "direct" && Instant::now() < deadline {
        tokio::time::sleep(PATH_POLL).await;
    }
    println!("path={}", path_kind(&conn));
    let _ = tokio::time::timeout(Duration::from_secs(secs), answer(conn)).await;
    println!("done");
    Ok(())
}
