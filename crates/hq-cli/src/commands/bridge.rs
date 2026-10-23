//! `hq bridge`: pair this machine with another by QR code and reach it by key afterwards
//! (`docs/PAIRING.md`). Built only with the `bridge` feature, and nothing here runs at start-up:
//! a command opens a listener when it is run and closes it when it ends.

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use hq_bridge::{
    Bridge, BridgeConfig, EndpointId, JoinResult, RelayMode, Ticket, Tuning, code::printable, echo,
    echo_handler, qr,
};
use hq_core::config::HqConfig;
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

#[derive(Args, Clone)]
pub struct NetArgs {
    /// How the two machines find each other besides a direct path: none (the default), default
    /// (n0's public relays, meant for development and testing only) or a relay URL
    #[arg(long, default_value = "none")]
    relay: String,
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
    /// Keep this machine reachable by its paired peers until interrupted (echo service)
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

async fn start(net: &NetArgs, db: Database, serve: bool) -> Result<Bridge> {
    let bind: Vec<SocketAddr> = if net.bind.is_empty() {
        vec![DEFAULT_BIND.parse()?]
    } else {
        net.bind.clone()
    };
    Bridge::start(
        BridgeConfig {
            key: key_path(net)?,
            bind,
            relay: relay_mode(&net.relay)?,
            name: device_name(net),
            with_addrs: !net.no_addrs,
            advertise: Vec::new(),
        },
        db,
        serve.then(echo_handler),
    )
    .await
}

pub async fn run(config: &HqConfig, action: BridgeAction) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    match action {
        BridgeAction::Pair { net, ttl } => pair(&net, db, Duration::from_secs(ttl)).await,
        BridgeAction::Join { link, net } => join(&net, db, &read_link(link)?).await,
        BridgeAction::Serve { net } => serve(&net, db).await,
        BridgeAction::Run { net } => run_control(&net, db).await,
        BridgeAction::Ping { peer, count, net } => ping(&net, db, &peer, count).await,
        BridgeAction::Peers { all, json } => list(&db, all, json),
        BridgeAction::Revoke { id } => revoke(&db, &id),
    }
}

/// The person types the digits shown on the other device, so a habitual "y" cannot pair it.
async fn typed_digits_match(expected: &str) -> bool {
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

async fn serve(net: &NetArgs, db: Database) -> Result<()> {
    let bridge = start(net, db, true).await?;
    println!("Serving paired peers as {}; ctrl-c to stop.", bridge.id());
    tokio::signal::ctrl_c().await?;
    bridge.shutdown().await;
    Ok(())
}

async fn run_control(net: &NetArgs, db: Database) -> Result<()> {
    let bridge = start(net, db, true).await?;
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    super::bridge_control::serve(
        bridge,
        net.relay.clone(),
        device_name(net),
        input,
        tokio::io::stdout(),
    )
    .await
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
