//! `hq bridge`: pair this machine with another by QR code and reach it by key afterwards
//! (`docs/PAIRING.md`). Built only with the `bridge` feature, and nothing here runs at start-up:
//! a command opens a listener when it is run and closes it when it ends.

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use hq_bridge::{
    Bridge, BridgeConfig, EdgeDoor, EndpointId, JoinResult, RelayMode, Role, SetupConfig, Ticket,
    Tuning,
    code::printable,
    echo, echo_handler, qr,
    setup::{SVC_GATE, SVC_WEB},
};
use hq_core::config::{DoorListen, DoorTarget, HqConfig};
use hq_db::{Database, bridge as peers};
use std::{
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
/// How long a hub that just paired waits for the machine's setup.
const SETUP_WAIT: Duration = Duration::from_secs(60);
/// Any host name: only the directory of `gate_key_path` is wanted.
const KEY_DIR_PROBE: &str = "probe";

/// This machine's part in setup after pairing (`docs/PAIRING.md`, "What pairing sets up").
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum RoleArg {
    /// Runs coding agents for a hub: offers its host gate, and sets itself up with the hub
    Machine,
    /// Holds HQ, like the phone app: offers its web app and `/mcp`, and answers setup
    Hub,
}

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
    /// This machine's part in setup after pairing: machine (the default, except for `run`) or
    /// hub (the default for `run`, the phone app's helper)
    #[arg(long = "as", value_enum)]
    role: Option<RoleArg>,
}

impl NetArgs {
    fn role_or(&self, default: Role) -> Role {
        match self.role {
            Some(RoleArg::Machine) => Role::Machine,
            Some(RoleArg::Hub) => Role::Hub,
            None => default,
        }
    }
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
fn bridge_services(
    config: Option<&HqConfig>,
    role: Option<Role>,
) -> Result<Option<hq_bridge::Services>> {
    let Some(config) = config else {
        return Ok(None);
    };
    // What setup offers, unless the config names that service itself; the shared check then
    // validates both alike.
    let offered = match role {
        Some(Role::Machine) => loopback_gate(config).map(|a| (SVC_GATE, a)),
        Some(Role::Hub) => config
            .bridge
            .web_listen()
            .ok()
            .flatten()
            .map(|a| (SVC_WEB, a)),
        None => None,
    };
    let mut bridge = config.bridge.clone();
    if let Some((name, addr)) = offered {
        bridge
            .services
            .entry(name.to_string())
            .or_insert_with(|| addr.to_string());
    }
    hq_bridge::services_from_config(&bridge, config.ws_port)
}

/// The host gate's address when the hub can use it through the bridge: on loopback, and admitting
/// that loopback address (bridge traffic arrives from it). The owner's gate is never moved here.
fn loopback_gate(config: &HqConfig) -> Option<SocketAddr> {
    let host = &config.agent_host;
    let addr: SocketAddr = host.gate_listen.as_deref()?.parse().ok()?;
    let admitted = host
        .gate_allowed_sources
        .iter()
        .any(|a| a.parse::<std::net::IpAddr>().ok() == Some(addr.ip()));
    (addr.ip().is_loopback() && addr.port() != 0 && admitted).then_some(addr)
}

/// What this machine brings to setup, from the config. Every path is the one HQ already uses.
fn setup_config(config: Option<&HqConfig>, role: Role) -> SetupConfig {
    let fallback = HqConfig::default();
    let config = config.unwrap_or(&fallback);
    let host = &config.agent_host;
    let key_dir = hq_tools::agent_host::gate_key_path(KEY_DIR_PROBE)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let mut cfg = SetupConfig::new(
        role,
        hq_tools::agent_host::gate_authorized_keys_path(host),
        key_dir,
    );
    cfg.gate = role == Role::Machine && loopback_gate(config).is_some();
    cfg.web = role == Role::Hub && matches!(config.bridge.web_listen(), Ok(Some(_)));
    cfg.reserved_names = host.hosts.keys().cloned().collect();
    cfg.reserved_names
        .extend(config.remote_mcp.iter().map(|s| s.name.clone()));
    let forwards = config.bridge.forwards().unwrap_or_default();
    cfg.reserved_ports = forwards.iter().map(|f| f.listen.port()).collect();
    cfg.configured_forwards = forwards
        .into_iter()
        .map(|f| (canonical_id(&f.peer), f.service))
        .collect();
    cfg
}

/// A peer id as the database spells it, whatever spelling the config used.
fn canonical_id(peer: &str) -> String {
    let peer = peer.trim();
    peer.parse::<EndpointId>()
        .or_else(|_| peer.to_ascii_lowercase().parse())
        .map(|id| id.to_string())
        .unwrap_or_else(|_| peer.to_string())
}

/// One line on what a machine without a usable gate is missing, so the hub can reach it.
fn gate_hint(config: Option<&HqConfig>, role: Role) {
    if role == Role::Machine && config.and_then(loopback_gate).is_none() {
        eprintln!(
            "note: the host gate is not offered to the hub. Set agent_host.gate_listen to a loopback address (127.0.0.1:<port>) with that address in agent_host.gate_allowed_sources, and run `hq host serve`."
        );
    }
}

/// Opens every forward in `bridge.forwards` on this bridge. A port that cannot be bound fails the
/// whole start, so a half-working setup is never mistaken for a working one.
async fn start_forwards(bridge: &Bridge, config: Option<&HqConfig>) -> Result<()> {
    let Some(config) = config else {
        return Ok(());
    };
    let forwards = hq_bridge::start_forwards(bridge, &config.bridge).await?;
    for f in &forwards.status {
        if f.running_elsewhere {
            bail!(
                "bridge.forwards: could not listen on {}: already in use",
                f.listen
            );
        }
        eprintln!("Forwarding {} to {} on {}.", f.listen, f.service, f.peer);
    }
    Ok(())
}

/// Starts this machine's bridge. With `setup`, it takes part in setup after pairing in that role.
async fn start(net: &NetArgs, db: Database, serve: bool, setup: Option<Role>) -> Result<Bridge> {
    let bind: Vec<SocketAddr> = if net.bind.is_empty() {
        vec![DEFAULT_BIND.parse()?]
    } else {
        net.bind.clone()
    };
    let relay = relay_mode(&resolve_relay(&net.relay, &db)?)?;
    let door = if serve { edge_door(net)? } else { None };
    let config = HqConfig::load().ok();
    let services = if serve {
        bridge_services(config.as_ref(), setup)?
    } else {
        None
    };
    let setup = setup.map(|role| setup_config(config.as_ref(), role));
    Bridge::start_with_setup(
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
        setup,
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
    let role = net.role_or(Role::Machine);
    let bridge = start(net, db, false, Some(role)).await?;
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
    let joiner = request.joiner.to_string();
    if accept && peers::is_paired(bridge.db(), &joiner).unwrap_or(false) {
        after_pairing(&bridge, &joiner, role).await;
    }
    bridge.shutdown().await;
    Ok(())
}

/// Setup right after a pairing: a machine runs it, a hub waits for the machine to. A failure
/// leaves the pairing as it is; the next `hq bridge serve` tries again.
async fn after_pairing(bridge: &Bridge, peer: &str, role: Role) {
    if role == Role::Hub {
        return match bridge.wait_setup(peer, SETUP_WAIT).await {
            Some(s) => println!(
                "Set up \"{}\": it is a host here, and its agents can reach this HQ.",
                printable(&s.peer_name, SHOWN_CHARS)
            ),
            None => println!(
                "The machine has not set itself up yet; it does when it next runs `hq bridge serve`."
            ),
        };
    }
    match bridge.set_up(peer).await {
        Ok(outcome) => report_setup(&outcome),
        Err(e) => println!(
            "Paired, but not set up yet: {e:#}. `hq bridge serve` tries again when the other device is serving."
        ),
    }
}

fn report_setup(outcome: &hq_bridge::SetupOutcome) {
    let s = &outcome.setup;
    let name = printable(&s.peer_name, SHOWN_CHARS);
    println!(
        "Set up with \"{name}\": its web app and /mcp are at http://127.0.0.1:{}/ while `hq bridge serve` runs here.",
        s.local_port
    );
    if !s.peer_offers {
        println!(
            "note: \"{name}\" does not offer its web app yet (bridge.web_listen is unset there)."
        );
    }
    if outcome.key_missing() {
        println!(
            "note: \"{name}\" kept the key it made for this machine earlier and this machine has no copy. Revoke and pair again to get a new one."
        );
    }
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
    let role = net.role_or(Role::Machine);
    let bridge = start(net, db, false, Some(role)).await?;
    let result = bridge
        .join(&ticket, &Tuning::default(), |sas| {
            println!(
                "Digits: {sas}   (type them on the other machine; it pairs only if they match)"
            );
        })
        .await;
    let outcome = match result {
        Ok(JoinResult::Paired(joined)) => {
            println!(
                "Paired with \"{}\" ({}).",
                printable(&joined.card.name, SHOWN_CHARS),
                joined.card.id
            );
            after_pairing(&bridge, &joined.card.id, role).await;
            Ok(())
        }
        Ok(JoinResult::Refused(why)) => Err(anyhow!("refused: {why}")),
        Ok(JoinResult::Declined { .. }) => Err(anyhow!("the other machine did not accept")),
        Err(e) => Err(e),
    };
    // Close the endpoint on every path, so a failure does not end in a second, noisy error.
    bridge.shutdown().await;
    outcome
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
    let bridge = start(net, db.clone(), false, None).await?;
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
    let role = net.role_or(Role::Machine);
    let bridge = start(net, db, true, Some(role)).await?;
    start_set_up(&bridge, role).await?;
    println!("Serving paired peers as {}; ctrl-c to stop.", bridge.id());
    tokio::signal::ctrl_c().await?;
    bridge.shutdown().await;
    Ok(())
}

/// The forwards, configured and set up, and on a machine setup with every paired peer that has
/// none yet (a peer that is not a hub, or not serving, is tried again at the next start).
async fn start_set_up(bridge: &Bridge, role: Role) -> Result<()> {
    let config = HqConfig::load().ok();
    start_forwards(bridge, config.as_ref()).await?;
    bridge.keep_setup()?;
    gate_hint(config.as_ref(), role);
    if role != Role::Machine {
        return Ok(());
    }
    for (peer, result) in bridge.set_up_all().await? {
        match result {
            Ok(outcome) => report_setup(&outcome),
            Err(e) => tracing::info!(%peer, "bridge setup not done: {e:#}"),
        }
    }
    Ok(())
}

async fn run_control(net: &NetArgs, db: Database) -> Result<()> {
    let relay = resolve_relay(&net.relay, &db)?;
    // The phone app's helper: a hub unless told otherwise.
    let role = net.role_or(Role::Hub);
    let bridge = start(net, db, true, Some(role)).await?;
    start_set_up(&bridge, role).await?;
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    super::bridge_control::serve(bridge, relay, device_name(net), input, tokio::io::stdout()).await
}

async fn ping(net: &NetArgs, db: Database, peer: &str, count: usize) -> Result<()> {
    let bridge = start(net, db, false, None).await?;
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
    let changed = peers::revoke(db, &id)?;
    // The revoke removed the setup row; the hub's gate line written for it goes too.
    let authorized = setup_config(HqConfig::load().ok().as_ref(), Role::Machine).authorized_keys;
    let removed = hq_bridge::setup::prune_gate_lines(db, &authorized)?;
    if changed {
        println!("Revoked {id}. A machine serving it stops at its next request.");
    } else {
        println!("{id} is not an active peer.");
    }
    if removed > 0 {
        println!("Removed its key from {}.", authorized.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn net(relay: &str, edge_door: Option<&str>) -> NetArgs {
        NetArgs {
            relay: relay.into(),
            edge_door: edge_door.map(Into::into),
            bind: Vec::new(),
            key: None,
            name: None,
            no_addrs: false,
            role: None,
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
        assert!(bridge_services(Some(&config), None).unwrap().is_none());
        config.ws_port = 5678;
        config.bridge.services =
            BTreeMap::from([("web".to_string(), "127.0.0.1:5678".to_string())]);
        assert!(bridge_services(Some(&config), None).is_err());
        config.bridge.services =
            BTreeMap::from([("web".to_string(), "127.0.0.1:5690".to_string())]);
        assert!(bridge_services(Some(&config), None).unwrap().is_some());
        config.bridge.services = BTreeMap::from([("web".to_string(), "0.0.0.0:5690".to_string())]);
        assert!(bridge_services(Some(&config), None).is_err());
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

    fn targets(config: &HqConfig, role: Role) -> BTreeMap<String, hq_bridge::DoorTarget> {
        bridge_services(Some(config), Some(role))
            .unwrap()
            .map(|s| s.targets)
            .unwrap_or_default()
    }

    #[test]
    fn a_bridge_setup_machine_offers_a_loopback_gate_that_admits_loopback_only() {
        let mut config = HqConfig::default();
        assert!(
            targets(&config, Role::Machine).is_empty(),
            "no gate, nothing"
        );
        config.agent_host.gate_listen = Some("127.0.0.1:7747".into());
        assert!(
            targets(&config, Role::Machine).is_empty(),
            "loopback not admitted"
        );
        config.agent_host.gate_allowed_sources = vec!["127.0.0.1".into()];
        let offered = targets(&config, Role::Machine);
        assert!(
            matches!(offered.get(SVC_GATE), Some(hq_bridge::DoorTarget::Tcp(a)) if a.port() == 7747),
            "{offered:?}"
        );
        assert!(setup_config(Some(&config), Role::Machine).gate);
        assert!(
            targets(&config, Role::Hub).is_empty(),
            "a hub offers no gate"
        );
        // A gate on the tunnel stays where the owner put it, and is not offered.
        config.agent_host.gate_listen = Some("100.64.0.2:7747".into());
        config.agent_host.gate_allowed_sources = vec!["100.64.0.1".into()];
        assert!(targets(&config, Role::Machine).is_empty());
        assert!(!setup_config(Some(&config), Role::Machine).gate);
    }

    #[test]
    fn a_bridge_setup_hub_offers_its_door_and_the_config_wins() {
        let mut config = HqConfig::default();
        assert!(targets(&config, Role::Hub).is_empty());
        config.bridge.web_listen = Some("127.0.0.1:5690".into());
        assert!(matches!(
            targets(&config, Role::Hub).get(SVC_WEB),
            Some(hq_bridge::DoorTarget::Tcp(a)) if a.port() == 5690
        ));
        assert!(setup_config(Some(&config), Role::Hub).web);
        config.bridge.services =
            BTreeMap::from([("web".to_string(), "127.0.0.1:5691".to_string())]);
        assert!(matches!(
            targets(&config, Role::Hub).get(SVC_WEB),
            Some(hq_bridge::DoorTarget::Tcp(a)) if a.port() == 5691
        ));
    }

    #[test]
    fn bridge_setup_never_takes_a_configured_name_port_or_forward() {
        let mut config = HqConfig::default();
        config.agent_host.hosts.insert(
            "laptop".into(),
            serde_yaml::from_str("ssh: \"me@100.64.0.1\"").unwrap(),
        );
        let id = hq_bridge::key::load_or_create_key(&tempfile::tempdir().unwrap().path().join("k"))
            .unwrap()
            .public()
            .to_string();
        config.bridge.forwards = vec![hq_core::config::BridgeForward {
            peer: id.to_uppercase(),
            service: "web".into(),
            listen: "127.0.0.1:47480".into(),
        }];
        let setup = setup_config(Some(&config), Role::Machine);
        assert!(setup.reserved_names.contains(&"laptop".to_string()));
        assert_eq!(setup.reserved_ports, [47_480]);
        assert_eq!(setup.configured_forwards, [(id, "web".to_string())]);
        assert!(
            setup.authorized_keys.ends_with("gate_authorized_keys"),
            "{:?}",
            setup.authorized_keys
        );
    }
}
