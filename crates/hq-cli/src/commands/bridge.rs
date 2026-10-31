//! `hq bridge`: pair this machine with another by QR code and reach it by key afterwards
//! (`docs/PAIRING.md`). Built only with the `bridge` feature, and nothing here runs at start-up:
//! a command opens a listener when it is run and closes it when it ends.

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use hq_bridge::{
    Bridge, BridgeConfig, EdgeDoor, EndpointId, JoinResult, RelayMode, Services, Ticket, Tuning,
    code::printable, echo, echo_handler, qr, run_forward,
};
use hq_core::config::{DoorListen, DoorTarget, HqConfig};
use hq_db::{Database, bridge as peers};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

const DEFAULT_BIND: &str = "0.0.0.0:0";
const KEY_FILE: &str = "bridge.key";
const BRIDGE_SUBDIR: &str = "bridge";
const DEFAULT_NAME: &str = "hq";
const DEFAULT_PINGS: usize = 5;
const SHOWN_CHARS: usize = 120;
const MIN_TTL_SECS: u64 = 10;
const MAX_TTL_SECS: u64 = 3_600;
/// A pasted link is a few hundred characters; this is far more than any ticket.
const LINK_READ_MAX: u64 = 4_096;

#[derive(Args, Clone)]
pub struct NetArgs {
    /// How the two machines find each other besides a direct path: none (the default), default
    /// (n0's public relays, meant for development and testing only), auto (the relay of the edge
    /// this device registered with, else none) or a relay URL
    #[arg(long, default_value = "none")]
    relay: String,
    /// Where the registered edge's streams are piped when this machine serves: a unix socket path
    /// or a loopback ip:port. By default the connector door (`mcp_connector.tls.door_listen`, else
    /// a private socket under the vault's `_data/connector_door`). `off` does not accept the edge
    /// protocol at all
    #[arg(long, value_name = "PATH|ADDR|off")]
    edge_door: Option<String>,
    /// Where this machine listens (repeatable; default: every address, any port)
    #[arg(long)]
    bind: Vec<SocketAddr>,
    /// This machine's long-term key (default: ~/.hq/bridge/bridge.key)
    #[arg(long, hide = true)]
    key: Option<PathBuf>,
    /// What this machine is called to the machines it pairs with
    #[arg(long)]
    name: Option<String>,
    /// Leave this machine's IP addresses out of tickets and cards: smaller, and they show no
    /// address, but the other side then needs a relay to find the direct path
    #[arg(long)]
    no_addrs: bool,
}

#[derive(Subcommand)]
pub enum BridgeAction {
    /// Show a pairing link and QR code and wait for one device. Type the six digits the other
    /// device shows; it is paired only if they match
    Pair {
        #[command(flatten)]
        net: NetArgs,
        /// How long the ticket works, in seconds
        #[arg(long, default_value_t = hq_bridge::code::DEFAULT_TTL.as_secs(),
              value_parser = clap::value_parser!(u64).range(MIN_TTL_SECS..=MAX_TTL_SECS))]
        ttl: u64,
    },
    /// Pair with the machine that shows a link (from its QR code or copied text)
    Join {
        /// The link. Leave it out, or give -, to paste it on standard input: a link on the
        /// command line is visible to every user of this machine for as long as the command runs
        link: Option<String>,
        #[command(flatten)]
        net: NetArgs,
    },
    /// Register this device with an `hq edge`: scan or paste the link its `hq edge invite` shows.
    /// The person at the edge types the six digits this machine shows. Afterwards the edge can
    /// reach this device's connector door, and nothing else
    Register {
        /// The link. Leave it out, or give -, to paste it on standard input
        link: Option<String>,
        /// Replace the edge this device is already registered with. Without it, a registered
        /// device is left as it is: a pasted link must not swap the real edge for another
        #[arg(long)]
        replace: bool,
        #[command(flatten)]
        net: NetArgs,
    },
    /// Show the edge this device is registered with
    Edge {
        #[arg(long)]
        json: bool,
    },
    /// Forget the edge. A running `serve` stops serving it at its next stream
    ForgetEdge,
    /// Keep this machine reachable by its paired peers until interrupted: the echo service, the
    /// services in `bridge.services` and the local ports in `bridge.forwards`
    Serve {
        #[command(flatten)]
        net: NetArgs,
    },
    /// Connect to a paired peer by its key and time a few round trips
    Ping {
        /// The peer's id, from `hq bridge peers`
        peer: String,
        #[arg(long, default_value_t = DEFAULT_PINGS)]
        count: usize,
        #[command(flatten)]
        net: NetArgs,
    },
    /// List the paired machines
    Peers {
        /// Include revoked ones
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// The phone app's helper: keep this machine reachable by its paired peers and take commands
    /// as JSON lines on standard input, answering on standard output (`docs/HQ_MOBILE.md`). It
    /// ends when its input does. Not meant to be typed by hand
    Run {
        #[command(flatten)]
        net: NetArgs,
    },
    /// Revoke a peer. A running `serve` stops serving it at its next request; the bridge of a
    /// running daemon closes its connections at once when it revokes through its own API
    Revoke { id: String },
}

fn key_path(net: &NetArgs) -> Result<PathBuf> {
    if let Some(key) = &net.key {
        return Ok(key.clone());
    }
    let dir = dirs::home_dir()
        .ok_or_else(|| anyhow!("no home directory; pass --key"))?
        .join(".hq")
        .join(BRIDGE_SUBDIR);
    std::fs::create_dir_all(&dir)?;
    set_private(&dir)?;
    Ok(dir.join(KEY_FILE))
}

#[cfg(unix)]
fn set_private(dir: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// `auto` becomes the registered edge's relay, or `none` when there is no edge or it has none.
/// Everything else is as given.
fn resolve_relay(text: &str, db: &Database) -> Result<String> {
    if text != "auto" {
        return Ok(text.to_string());
    }
    Ok(peers::edge_get(db)?
        .and_then(|edge| edge.relay)
        .unwrap_or_else(|| "none".to_string()))
}

fn relay_mode(text: &str) -> Result<RelayMode> {
    match text {
        "none" => Ok(RelayMode::Disabled),
        "default" => {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "note: n0's public relays are for development and testing, not for production."
            );
            Ok(RelayMode::Default)
        }
        url => Ok(RelayMode::custom([url
            .parse()
            .context("--relay is none, default or a relay URL")?])),
    }
}

fn device_name(net: &NetArgs) -> String {
    net.name
        .clone()
        .unwrap_or_else(|| std::env::var("HOSTNAME").unwrap_or_else(|_| DEFAULT_NAME.to_string()))
}

/// Where a serving machine pipes the registered edge's streams: `--edge-door`, else the connector
/// door's address from the config, else its default (a private socket under the vault). Only the
/// connector's door: a socket path or a loopback address with a fixed port, nothing else on this
/// machine or beyond it.
fn edge_door(net: &NetArgs) -> Result<Option<EdgeDoor>> {
    edge_door_with(net, HqConfig::load().ok().as_ref())
}

/// [`edge_door`] with the configuration given, so a test does not read the machine's own.
fn edge_door_with(net: &NetArgs, config: Option<&HqConfig>) -> Result<Option<EdgeDoor>> {
    if net.edge_door.as_deref() == Some("off") {
        return Ok(None);
    }
    let asked = net
        .edge_door
        .clone()
        .or_else(|| config.and_then(|c| c.mcp_connector.tls.door_listen.clone()));
    let listen = DoorListen::parse(asked.as_deref()).map_err(|e| anyhow!("--edge-door: {e}"))?;
    let vault = config.map(|c| c.vault_path.clone()).unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".hq")
            .join("vault")
    });
    let target = match listen.resolve(&vault) {
        DoorTarget::Unix(path) => hq_bridge::DoorTarget::Unix(path),
        DoorTarget::Tcp(addr) => hq_bridge::DoorTarget::Tcp(addr),
    };
    Ok(Some(EdgeDoor::new(target)))
}

/// The services in `bridge.services`, checked, or `None` when there are none. A service may not
/// lead to the main web port: paired devices get the web app through `bridge.web_listen`, the door
/// that keeps the connector's paths and credentials out.
fn bridge_services(config: Option<&HqConfig>) -> Result<Option<Services>> {
    let Some(config) = config else {
        return Ok(None);
    };
    let targets = config.bridge.services().map_err(|e| anyhow!(e))?;
    if targets.is_empty() {
        return Ok(None);
    }
    let mut out = BTreeMap::new();
    for (name, target) in targets {
        let target = match target {
            DoorTarget::Unix(path) => hq_bridge::DoorTarget::Unix(path),
            DoorTarget::Tcp(addr) if addr.port() == config.ws_port => bail!(
                "bridge.services.{name} leads to the web server's own port ({}); point it at bridge.web_listen",
                config.ws_port
            ),
            DoorTarget::Tcp(addr) => hq_bridge::DoorTarget::Tcp(addr),
        };
        out.insert(name, target);
    }
    Ok(Some(Services::new(out)))
}

/// Opens every forward in `bridge.forwards` on this bridge. A port that cannot be bound fails the
/// whole start, so a half-working setup is never mistaken for a working one.
async fn start_forwards(bridge: &Bridge, config: Option<&HqConfig>) -> Result<()> {
    let Some(config) = config else {
        return Ok(());
    };
    for f in config.bridge.forwards().map_err(|e| anyhow!(e))? {
        let listener = tokio::net::TcpListener::bind(f.listen)
            .await
            .with_context(|| format!("bridge.forwards: could not listen on {}", f.listen))?;
        eprintln!("Forwarding {} to {} on {}.", f.listen, f.service, f.peer);
        tokio::spawn(run_forward(bridge.dialer(), f.peer, f.service, listener));
    }
    Ok(())
}

async fn start(net: &NetArgs, db: Database, serve: bool) -> Result<Bridge> {
    let bind: Vec<SocketAddr> = if net.bind.is_empty() {
        vec![DEFAULT_BIND.parse()?]
    } else {
        net.bind.clone()
    };
    let relay = relay_mode(&resolve_relay(&net.relay, &db)?)?;
    let door = if serve { edge_door(net)? } else { None };
    let services = if serve {
        bridge_services(HqConfig::load().ok().as_ref())?
    } else {
        None
    };
    Bridge::start_with_services(
        BridgeConfig {
            key: key_path(net)?,
            bind,
            relay,
            name: device_name(net),
            with_addrs: !net.no_addrs,
            advertise: Vec::new(),
        },
        db,
        serve.then(echo_handler),
        door,
        services,
    )
    .await
}

pub async fn run(config: &HqConfig, action: BridgeAction) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    match action {
        BridgeAction::Pair { net, ttl } => pair(&net, db, Duration::from_secs(ttl)).await,
        BridgeAction::Join { link, net } => join(&net, db, &read_link(link)?).await,
        BridgeAction::Register { link, replace, net } => {
            register(&net, db, &read_link(link)?, replace).await
        }
        BridgeAction::Edge { json } => show_edge(&db, json),
        BridgeAction::ForgetEdge => forget_edge(&db),
        BridgeAction::Serve { net } => serve(&net, db).await,
        BridgeAction::Run { net } => run_control(&net, db).await,
        BridgeAction::Ping { peer, count, net } => ping(&net, db, &peer, count).await,
        BridgeAction::Peers { all, json } => list(&db, all, json),
        BridgeAction::Revoke { id } => revoke(&db, &id),
    }
}

/// The person types the digits shown on the other device, so a habitual "y" cannot pair it.
pub(crate) async fn typed_digits_match(expected: &str) -> bool {
    use std::io::Write;
    print!(
        "Type the {} digits shown on the other device (empty to decline): ",
        expected.len()
    );
    let _ = std::io::stdout().flush();
    let expected = expected.to_string();
    tokio::task::spawn_blocking(move || {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        line.trim() == expected
    })
    .await
    .unwrap_or(false)
}

async fn pair(net: &NetArgs, db: Database, ttl: Duration) -> Result<()> {
    let bridge = start(net, db, false).await?;
    let mut open = bridge
        .open_ticket(Tuning {
            ttl,
            ..Tuning::default()
        })
        .await?;
    let link = open.ticket.link();
    println!("This machine: {}", bridge.id());
    println!(
        "\nScan this on the other machine, or give it the text below ({} s, one use):\n",
        ttl.as_secs()
    );
    if let Some(art) = qr::render(&link) {
        println!("{art}");
    }
    println!("{link}\n");
    // The channel closes when the ticket is spent, expired or burned and nothing is running.
    let request = tokio::select! {
        request = open.requests.recv() => request,
        _ = tokio::signal::ctrl_c() => None,
    };
    let Some(request) = request else {
        println!("No device paired: the ticket ended.");
        bridge.shutdown().await;
        return Ok(());
    };
    println!(
        "\"{}\" ({}) wants to pair.",
        printable(&request.name, SHOWN_CHARS),
        request.joiner
    );
    let accept = typed_digits_match(&request.sas).await;
    match request.decide.send(accept) {
        Ok(()) if accept => println!("Paired {}.", request.joiner),
        Ok(()) => println!("Declined {}.", request.joiner),
        Err(_) => println!("Too late: the other device gave up waiting. Nothing was paired."),
    }
    // The first ctrl-c handler stays installed for the life of the process, so this wait has to
    // listen for it too or the person could not stop it.
    tokio::select! {
        _ = open.closed() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    bridge.shutdown().await;
    Ok(())
}

fn read_link(arg: Option<String>) -> Result<String> {
    use std::io::{BufRead, Read};
    match arg.as_deref() {
        Some(link) if link != "-" => Ok(link.to_string()),
        _ => {
            // One line: a person pastes the link and presses Enter, without an end of input.
            eprintln!("Paste the pairing link, then press Enter:");
            let mut text = String::new();
            std::io::stdin()
                .lock()
                .take(LINK_READ_MAX)
                .read_line(&mut text)?;
            Ok(text)
        }
    }
}

async fn join(net: &NetArgs, db: Database, link: &str) -> Result<()> {
    let ticket = Ticket::parse(link)?;
    let bridge = start(net, db, false).await?;
    let result = bridge
        .join(&ticket, &Tuning::default(), |sas| {
            println!(
                "Digits: {sas}   (type them on the other machine; it pairs only if they match)"
            );
        })
        .await;
    // Close the endpoint on every path, so a failure does not end in a second, noisy error.
    bridge.shutdown().await;
    match result? {
        JoinResult::Paired(joined) => println!(
            "Paired with \"{}\" ({}).",
            printable(&joined.card.name, SHOWN_CHARS),
            joined.card.id
        ),
        JoinResult::Refused(why) => bail!("refused: {why}"),
        JoinResult::Declined { .. } => bail!("the other machine did not accept"),
    }
    Ok(())
}

/// Registers with an edge and says what that gives this device, and what it does not yet.
async fn register(net: &NetArgs, db: Database, link: &str, replace: bool) -> Result<()> {
    let ticket = Ticket::parse(link)?;
    if let Some(current) = peers::edge_get(&db)?
        && !replace
    {
        bail!(
            "this device is already registered with {} ({}). To register with another edge instead, run this again with --replace; `hq bridge forget-edge` removes the current one",
            printable(&current.domain, SHOWN_CHARS),
            current.id
        );
    }
    let bridge = start(net, db.clone(), false).await?;
    let result = bridge
        .register(&ticket, &Tuning::default(), |sas| {
            println!(
                "Digits: {sas}   (type them on the edge machine; it registers this device only if they match)"
            );
        })
        .await;
    bridge.shutdown().await;
    match result? {
        JoinResult::Paired(joined) => {
            let edge = peers::edge_get(&db)?
                .ok_or_else(|| anyhow!("the edge was accepted but is not recorded"))?;
            println!(
                "Registered with the edge \"{}\" ({}).",
                printable(&joined.card.name, SHOWN_CHARS),
                edge.id
            );
            println!("This device is reached at https://{}.", edge.hostname);
            println!(
                "To let Claude connect, enable the connector's TLS door for that name: see docs/CLAUDE_CONNECTOR.md. Run `hq bridge serve --relay auto` to stay reachable."
            );
        }
        JoinResult::Refused(why) => bail!("refused: {why}"),
        JoinResult::Declined { .. } => bail!("the edge did not accept"),
    }
    Ok(())
}

fn show_edge(db: &Database, json: bool) -> Result<()> {
    let edge = peers::edge_get(db)?;
    if json {
        let out = edge.map(|e| {
            serde_json::json!({
                "id": e.id, "domain": e.domain, "hostname": e.hostname,
                "relay": e.relay, "registered_at": e.registered_at,
            })
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    match edge {
        Some(e) => println!(
            "Registered with {} ({}); reached at https://{}",
            printable(&e.domain, SHOWN_CHARS),
            e.id,
            e.hostname
        ),
        None => println!("Not registered with an edge."),
    }
    Ok(())
}

fn forget_edge(db: &Database) -> Result<()> {
    if peers::edge_clear(db)? {
        println!("Forgot the edge. A machine serving it stops at its next stream.");
    } else {
        println!("Not registered with an edge.");
    }
    Ok(())
}

async fn serve(net: &NetArgs, db: Database) -> Result<()> {
    let bridge = start(net, db, true).await?;
    start_forwards(&bridge, HqConfig::load().ok().as_ref()).await?;
    println!("Serving paired peers as {}; ctrl-c to stop.", bridge.id());
    tokio::signal::ctrl_c().await?;
    bridge.shutdown().await;
    Ok(())
}

async fn run_control(net: &NetArgs, db: Database) -> Result<()> {
    let relay = resolve_relay(&net.relay, &db)?;
    let bridge = start(net, db, true).await?;
    start_forwards(&bridge, HqConfig::load().ok().as_ref()).await?;
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    super::bridge_control::serve(bridge, relay, device_name(net), input, tokio::io::stdout()).await
}

async fn ping(net: &NetArgs, db: Database, peer: &str, count: usize) -> Result<()> {
    let bridge = start(net, db, false).await?;
    let result = ping_peer(&bridge, peer, count).await;
    bridge.shutdown().await;
    result
}

async fn ping_peer(bridge: &Bridge, peer: &str, count: usize) -> Result<()> {
    let started = Instant::now();
    let conn = bridge.connect(peer).await?;
    println!("Connected in {:?}.", started.elapsed());
    let mut times = Vec::new();
    for _ in 0..count.max(1) {
        let t = Instant::now();
        echo(&conn, b"ping").await?;
        times.push(t.elapsed());
    }
    times.sort();
    println!(
        "{} round trips: fastest {:?}, median {:?}, slowest {:?}",
        times.len(),
        times[0],
        times[times.len() / 2],
        times[times.len() - 1]
    );
    Ok(())
}

fn list(db: &Database, all: bool, json: bool) -> Result<()> {
    let rows = peers::list(db, all)?;
    if json {
        let out: Vec<_> = rows
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id, "name": p.label, "paired_at": p.paired_at,
                    "last_seen_at": p.last_seen_at, "revoked_at": p.revoked_at,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("No paired machines.");
    }
    for p in rows {
        let state = if p.is_active() { "" } else { "  (revoked)" };
        println!("{}  {}{state}", p.id, printable(&p.label, SHOWN_CHARS));
    }
    Ok(())
}

fn revoke(db: &Database, id: &str) -> Result<()> {
    let id: EndpointId = id
        .parse()
        .map_err(|_| anyhow!("{id} is not a machine id (see `hq bridge peers`)"))?;
    let id = id.to_string();
    if peers::revoke(db, &id)? {
        println!("Revoked {id}. A machine serving it stops at its next request.");
    } else {
        println!("{id} is not an active peer.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(relay: &str, edge_door: Option<&str>) -> NetArgs {
        NetArgs {
            relay: relay.into(),
            edge_door: edge_door.map(Into::into),
            bind: Vec::new(),
            key: None,
            name: None,
            no_addrs: false,
        }
    }

    #[test]
    fn the_edge_and_the_device_agree_on_the_protocol_name_and_the_label() {
        assert_eq!(hq_bridge::EDGE_ALPN, hq_edge::EDGE_ALPN);
        assert_eq!(hq_bridge::LABEL_BYTES, hq_edge::LABEL_BYTES);
        let dir = tempfile::tempdir().unwrap();
        let id = hq_bridge::key::load_or_create_key(&dir.path().join("k"))
            .unwrap()
            .public();
        assert_eq!(hq_bridge::device_label(&id), hq_edge::device_label(&id));
    }

    #[test]
    fn a_service_may_not_lead_to_the_main_web_port() {
        let mut config = HqConfig::default();
        assert!(bridge_services(Some(&config)).unwrap().is_none());
        config.ws_port = 5678;
        config.bridge.services =
            BTreeMap::from([("web".to_string(), "127.0.0.1:5678".to_string())]);
        assert!(bridge_services(Some(&config)).is_err());
        config.bridge.services =
            BTreeMap::from([("web".to_string(), "127.0.0.1:5690".to_string())]);
        assert!(bridge_services(Some(&config)).unwrap().is_some());
        config.bridge.services = BTreeMap::from([("web".to_string(), "0.0.0.0:5690".to_string())]);
        assert!(bridge_services(Some(&config)).is_err());
    }

    #[test]
    fn the_edge_door_is_a_socket_path_or_a_loopback_port_or_off() {
        assert!(
            edge_door_with(&net("none", Some("off")), None)
                .unwrap()
                .is_none()
        );
        for ok in [
            "127.0.0.1:7001",
            "[::1]:7001",
            "/run/hq/door.sock",
            "unix:/run/hq/door.sock",
        ] {
            assert!(
                edge_door_with(&net("none", Some(ok)), None)
                    .unwrap()
                    .is_some(),
                "{ok}"
            );
        }
        for bad in [
            "0.0.0.0:5679",
            "192.168.1.9:5679",
            "[::]:5679",
            "127.0.0.1:0",
            "127.0.0.1",
            "localhost:5679",
            "relative.sock",
        ] {
            assert!(
                edge_door_with(&net("none", Some(bad)), None).is_err(),
                "{bad}"
            );
        }
        // Left alone it is the connector door's default: a private socket, the same one the web
        // server listens on.
        let default = edge_door_with(&net("none", None), None)
            .unwrap()
            .unwrap()
            .target;
        assert!(
            matches!(&default, hq_bridge::DoorTarget::Unix(p)
                if p.ends_with("connector_door/door.sock")),
            "{default:?}"
        );
    }

    #[test]
    fn relay_auto_follows_the_registered_edge_and_nothing_else_changes() {
        let db = Database::open_memory().unwrap();
        assert_eq!(resolve_relay("auto", &db).unwrap(), "none");
        peers::edge_set(&db, "ed01", "edge.example.com", "aa.edge.example.com", None).unwrap();
        assert_eq!(
            resolve_relay("auto", &db).unwrap(),
            "none",
            "an edge with no relay"
        );
        peers::edge_set(
            &db,
            "ed01",
            "edge.example.com",
            "aa.edge.example.com",
            Some("https://relay.edge.example.com/"),
        )
        .unwrap();
        assert_eq!(
            resolve_relay("auto", &db).unwrap(),
            "https://relay.edge.example.com/"
        );
        for as_given in ["none", "default", "https://other.example.org"] {
            assert_eq!(resolve_relay(as_given, &db).unwrap(), as_given);
        }
    }
}
