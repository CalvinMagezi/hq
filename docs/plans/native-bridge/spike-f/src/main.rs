//! Spike F's command line. `show` is the machine that shows the QR code; `join` is the one that
//! scans it; `dial` reconnects by key; `stranger` tries a key nobody paired.
//!
//! ```text
//! pairspike show   --key K --paired P [--no-addrs] [--bind ip:port] [--relay none|default|<url>] [--advertise ip:port]... [--ttl S] [--attempts N] [--decision S] [--serve] [--auto-accept]
//! pairspike join   --ticket T --key K --store S [--relay ...] [--keepalive S]
//! pairspike dial   --key K --store S [--calls N] [--keep] [--idle S] [--relay ...] [--keepalive S]
//! pairspike stranger --ticket T [--relay ...]
//! ```

use anyhow::{Context, Result, anyhow, bail};
use iroh::RelayMode;
use pairspike::{
    net::{self, HostConfig, JoinConfig, JoinResult},
    pairing::{self, Ticket},
    qr,
};
use std::{net::SocketAddr, path::PathBuf, time::Duration};

const DEFAULT_BIND: &str = "0.0.0.0:0";

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn opts(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|w| w[0] == name)
        .map(|w| w[1].clone())
        .collect()
}

fn need(args: &[String], name: &str) -> Result<String> {
    opt(args, name).ok_or_else(|| anyhow!("missing {name}"))
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn relay_mode(args: &[String]) -> Result<RelayMode> {
    match opt(args, "--relay").as_deref() {
        None | Some("none") => Ok(RelayMode::Disabled),
        // n0's public relays are for development and testing; the spike uses them only to compare.
        Some("default") => Ok(RelayMode::Default),
        Some(url) => Ok(RelayMode::custom([url
            .parse()
            .context("--relay is none, default or a relay url")?])),
    }
}

fn secs(args: &[String], name: &str) -> Result<Option<Duration>> {
    opt(args, name)
        .map(|s| {
            s.parse::<u64>()
                .map(Duration::from_secs)
                .with_context(|| name.to_string())
        })
        .transpose()
}

fn bind(args: &[String]) -> Result<SocketAddr> {
    opt(args, "--bind")
        .unwrap_or_else(|| DEFAULT_BIND.into())
        .parse()
        .context("--bind")
}

fn join_config(args: &[String]) -> Result<JoinConfig> {
    Ok(JoinConfig {
        key: PathBuf::from(need(args, "--key")?),
        store: PathBuf::from(need(args, "--store")?),
        bind: bind(args)?,
        relay: relay_mode(args)?,
        keepalive: secs(args, "--keepalive")?,
    })
}

async fn show(args: &[String]) -> Result<()> {
    let cfg = HostConfig {
        key: PathBuf::from(need(args, "--key")?),
        paired: PathBuf::from(need(args, "--paired")?),
        bind: bind(args)?,
        relay: relay_mode(args)?,
        ttl: secs(args, "--ttl")?.unwrap_or(pairing::DEFAULT_TTL),
        attempts: opt(args, "--attempts")
            .map_or(Ok(pairing::DEFAULT_ATTEMPTS), |s| s.parse())
            .context("--attempts")?,
        advertise: opts(args, "--advertise")
            .iter()
            .map(|a| a.parse().context("--advertise"))
            .collect::<Result<_>>()?,
        with_addrs: !flag(args, "--no-addrs"),
        keepalive: secs(args, "--keepalive")?,
        hello_timeout: net::HELLO_TIMEOUT,
        decision_timeout: secs(args, "--decision")?.unwrap_or(net::DECISION_TIMEOUT),
    };
    let ttl = cfg.ttl;
    let mut host = net::Host::start(cfg).await?;
    let text = host.ticket.encode();
    println!("host id: {}", host.endpoint.id());
    println!(
        "\nScan this on the other machine, or paste the text below it ({} s):\n",
        ttl.as_secs()
    );
    if let Some(art) = qr::render(&text) {
        println!("{art}");
    }
    println!("{text}\n");
    let auto = flag(args, "--auto-accept");
    let deadline = tokio::time::sleep(ttl);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            request = host.requests.recv() => {
                let Some(request) = request else { break };
                println!("device {} wants to pair.", request.joiner);
                let accept = if auto { true } else { typed_digits_match(&request.sas).await };
                match request.decide.send(accept) {
                    Ok(()) if accept => println!("paired {}", request.joiner),
                    Ok(()) => println!("declined {}", request.joiner),
                    Err(_) => println!("too late: the other device gave up waiting, nothing was paired"),
                }
                if !flag(args, "--serve") {
                    break;
                }
            }
            _ = &mut deadline, if !flag(args, "--serve") => {
                println!("the ticket expired");
                break;
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    if flag(args, "--serve") {
        println!("serving paired devices; ctrl-c to stop");
        tokio::signal::ctrl_c().await?;
    }
    println!(
        "refused connections: {}",
        host.refused.load(std::sync::atomic::Ordering::SeqCst)
    );
    host.endpoint.close().await;
    Ok(())
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

async fn join(args: &[String]) -> Result<()> {
    let ticket = Ticket::parse(&need(args, "--ticket")?)?;
    let result = net::join(&ticket, join_config(args)?, |sas| {
        println!("digits: {sas}   (type them on the host; it pairs only if they match)");
    })
    .await?;
    match result {
        JoinResult::Paired { .. } => println!("paired with {}", ticket.id),
        JoinResult::Refused(why) => bail!("refused: {why}"),
        JoinResult::Declined { .. } => bail!("the host did not accept"),
    }
    Ok(())
}

fn percentile(sorted: &[Duration], p: usize) -> Duration {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

async fn dial(args: &[String]) -> Result<()> {
    let cfg = join_config(args)?;
    let calls: usize = opt(args, "--calls")
        .map_or(Ok(20), |s| s.parse())
        .context("--calls")?;
    let keep = flag(args, "--keep");
    loop {
        let session = match net::dial(&cfg).await {
            Ok(s) => s,
            Err(e) if keep => {
                println!("{} connect failed: {e:#}", stamp());
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            Err(e) => return Err(e),
        };
        let path = net::settle_path(&session.conn).await;
        println!(
            "{} connected in {:?} path={path} quic_rtt={:?}",
            stamp(),
            session.connect_time,
            net::path_rtt(&session.conn).unwrap_or_default()
        );
        if let Some(idle) = secs(args, "--idle")? {
            // Nothing is sent by either side: what moves is the transport's own keep-alive.
            let before = session.conn.stats();
            tokio::time::sleep(idle).await;
            let after = session.conn.stats();
            let sent = after.udp_tx.bytes - before.udp_tx.bytes;
            let received = after.udp_rx.bytes - before.udp_rx.bytes;
            let per_hour = (sent + received) as f64 * 3600.0 / idle.as_secs_f64();
            println!(
                "idle {}s: sent {sent} B, received {received} B, about {:.2} MB an hour, path={}",
                idle.as_secs(),
                per_hour / 1_000_000.0,
                net::path_kind(&session.conn)
            );
            return Ok(());
        }
        let mut times = Vec::new();
        loop {
            let t = std::time::Instant::now();
            match net::echo(&session.conn, b"ping").await {
                Ok(()) => times.push(t.elapsed()),
                Err(e) => {
                    println!("{} call failed: {e:#}", stamp());
                    break;
                }
            }
            if keep {
                println!(
                    "{} ok path={} {:?}",
                    stamp(),
                    net::path_kind(&session.conn),
                    t.elapsed()
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
            } else if times.len() >= calls {
                break;
            }
        }
        if !keep {
            times.sort();
            if times.is_empty() {
                bail!("no call succeeded");
            }
            println!(
                "calls={} p50={:?} p95={:?} max={:?}",
                times.len(),
                percentile(&times, 50),
                percentile(&times, 95),
                times[times.len() - 1]
            );
            return Ok(());
        }
    }
}

fn stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!(
        "[{:02}:{:02}:{:02}]",
        now / 3600 % 24,
        now / 60 % 60,
        now % 60
    )
}

async fn stranger(args: &[String]) -> Result<()> {
    let ticket = Ticket::parse(&need(args, "--ticket")?)?;
    let outcome = net::stranger(&ticket, bind(args)?, relay_mode(args)?).await?;
    println!("a key nobody paired got: {outcome}");
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {}", pairing::printable(&format!("{e:#}")));
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first() else {
        bail!("commands: show, join, dial, stranger (see the top of src/main.rs)");
    };
    let rest = &args[1..];
    match command.as_str() {
        "show" => show(rest).await,
        "join" => join(rest).await,
        "dial" => dial(rest).await,
        "stranger" => stranger(rest).await,
        other => bail!("unknown command {other}"),
    }
}
