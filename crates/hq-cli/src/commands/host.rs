//! `hq host`: run or inspect the built-in agent host.

use anyhow::{Context, Result, bail};
use hq_host::{BurstLimit, Client, Host, Server, socket_path};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Most agents a served host runs or holds at once. Several times what one
/// person's sessions have needed, low enough that a runaway is bounded.
const DEFAULT_MAX_AGENTS: usize = 16;
/// Most agents started within `BURST_WINDOW` before the host locks itself.
const DEFAULT_MAX_STARTS: usize = 10;
const BURST_WINDOW: Duration = Duration::from_secs(10 * 60);

fn default_dir() -> PathBuf {
    hq_core::config::native_host_dir()
}

pub struct HostArgs {
    pub sub: String,
    pub arg: Option<String>,
    pub addr: Option<String>,
    pub dir: Option<PathBuf>,
    pub allow_unsandboxed: bool,
    pub max_agents: Option<usize>,
    pub max_starts: Option<usize>,
    pub key: Option<String>,
    pub from: Option<String>,
    pub port: Option<u16>,
    pub unix: Option<PathBuf>,
    pub rest: Vec<String>,
}

pub async fn run(args: HostArgs) -> Result<()> {
    let HostArgs { sub, arg, addr, dir, allow_unsandboxed, max_agents, max_starts, key, from, port, unix, rest } = args;
    let sub = sub.as_str();
    let dir_arg = dir.clone();
    let dir = dir.unwrap_or_else(default_dir);
    match sub {
        "serve" => {
            let caps = Caps {
                max_agents: max_agents.unwrap_or(DEFAULT_MAX_AGENTS),
                max_starts: max_starts.unwrap_or(DEFAULT_MAX_STARTS),
            };
            serve(dir, allow_unsandboxed, caps).await
        }
        "status" => status(&dir),
        "stop" => stop(&dir),
        "lockdown" => lockdown(&dir, arg.as_deref()),
        "unlock" => unlock(&dir),
        "report" => report(dir_arg),
        "gate" => gate(&dir),
        "relay" => relay(port, unix),
        "sandbox-init" => sandbox_init(port, unix, &rest),
        "install" => super::host_install::install(),
        "join" => super::host_install::join(arg.as_deref(), addr.as_deref()),
        "add" => super::host_install::add(arg.as_deref(), from.as_deref()),
        "check" => super::host_install::check(arg.as_deref()),
        "authorize" => super::host_install::authorize(key.as_deref(), from.as_deref()),
        other => bail!("unknown subcommand '{other}': use serve, status, stop, lockdown, unlock, install, authorize, report or gate"),
    }
}

struct Caps {
    max_agents: usize,
    max_starts: usize,
}

async fn serve(dir: PathBuf, allow_unsandboxed: bool, caps: Caps) -> Result<()> {
    if caps.max_agents == 0 || caps.max_starts == 0 {
        bail!("--max-agents and --max-starts must be at least 1");
    }
    let host = Arc::new(
        Host::new()
            .with_state_dir(&dir)
            .with_require_sandbox(!allow_unsandboxed)
            .with_max_agents(caps.max_agents)
            .with_burst_limit(BurstLimit { max: caps.max_starts, window: BURST_WINDOW }),
    );
    if let Some(held) = host.lockdown_status() {
        eprintln!("hq host is LOCKED DOWN ({}); no agent will start until `hq host unlock`", held.reason);
    }
    let server = Server::bind(&dir, host.clone())
        .with_context(|| format!("starting the host in {}", dir.display()))?;
    match hq_host::ensure_workspace() {
        Ok(root) => println!("hq host workspace {}", root.display()),
        Err(e) => eprintln!("hq host could not prepare the HQ folder: {e}"),
    }
    let report = host.restore();
    for name in &report.restored {
        println!("hq host restored {name}");
    }
    for (name, why) in &report.skipped {
        eprintln!("hq host could not restore {name}: {why}");
    }
    let stop = server.stop_handle();
    println!("hq host serving {}", socket_path(&dir).display());
    let serving = tokio::task::spawn_blocking(move || server.serve());

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut serving = serving;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => stop.stop(),
        _ = term.recv() => stop.stop(),
        done = &mut serving => return Ok(done?),
    }
    Ok(serving.await?)
}

fn status(dir: &std::path::Path) -> Result<()> {
    let mut client = Client::connect(dir)
        .map_err(|e| anyhow::anyhow!("host not running in {}: {e}", dir.display()))?;
    let status = client.call("host.status", json!({}))?;
    println!("{}", serde_json::to_string_pretty(&status)?);
    let agents = client.call("agent.list", json!({}))?;
    for a in agents["agents"].as_array().into_iter().flatten() {
        let text = |k: &str| a[k].as_str().unwrap_or("-").to_string();
        println!(
            "{:<28} {:<8} {:<9} sandbox={:<8} idle {}s, up {}s",
            text("name"),
            text("agent"),
            text("state"),
            text("sandbox"),
            a["quiet_ms"].as_u64().unwrap_or(0) / 1000,
            a["age_ms"].as_u64().unwrap_or(0) / 1000,
        );
    }
    Ok(())
}

/// `hq host lockdown`: the kill switch. The lockdown file goes down first, so new
/// starts are refused even when the host is wedged or not running; then the host
/// is asked to stop every agent it runs.
fn lockdown(dir: &std::path::Path, reason: Option<&str>) -> Result<()> {
    let wanted = hq_host::Lockdown::new(reason.unwrap_or("locked from the command line"), "cli");
    let held = hq_host::engage_lockdown(dir, &wanted)
        .with_context(|| format!("writing {}", hq_host::lockdown_path(dir).display()))?;
    println!("hq host locked down: {}", held.reason);
    let stopped = Client::connect(dir).and_then(|mut c| c.call("host.lockdown", json!({ "reason": held.reason })));
    match stopped {
        Ok(_) => println!("every agent the host ran has been stopped; nothing starts until `hq host unlock`"),
        Err(e) => {
            println!("the host could not be asked to stop its agents ({e}); no new agent will start, but check for running ones");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// `hq host unlock`: lifts a lockdown by deleting the file, which only someone on
/// this machine can do. Nothing restarts by itself: HQ resumes what it needs.
fn unlock(dir: &std::path::Path) -> Result<()> {
    let previous = hq_host::read_lockdown(dir);
    let cleared = hq_host::clear_lockdown(dir)
        .with_context(|| format!("removing {}", hq_host::lockdown_path(dir).display()))?;
    match (cleared, previous) {
        (true, Some(held)) => println!("hq host unlocked (it was locked by {}: {})", held.source, held.reason),
        (true, None) => println!("hq host unlocked"),
        (false, _) => println!("hq host was not locked down"),
    }
    Ok(())
}

fn stop(dir: &std::path::Path) -> Result<()> {
    let mut client = Client::connect(dir)
        .map_err(|e| anyhow::anyhow!("host not running in {}: {e}", dir.display()))?;
    client.call("host.stop", json!({}))?;
    println!("hq host stopping");
    Ok(())
}

/// Longest hook payload read from stdin.
const MAX_HOOK_INPUT: u64 = 64 * 1024;
/// A hook must not hold up the agent, so the host gets this long to answer.
const REPORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// `hq host report`: forwards a Claude Code hook event to the host. It runs
/// inside an agent's pane with the pane's own token, and it never fails or
/// prints, so a missing host cannot disturb the agent.
fn report(dir: Option<PathBuf>) -> Result<()> {
    use std::io::Read;
    let token = std::env::var(hq_host::PANE_TOKEN_ENV).ok();
    let dir = dir.or_else(|| std::env::var_os(hq_host::RUN_DIR_ENV).map(PathBuf::from));
    let (Some(token), Some(dir)) = (token, dir) else {
        return Ok(());
    };
    let mut input = String::new();
    let _ = std::io::stdin().take(MAX_HOOK_INPUT).read_to_string(&mut input);
    let Some(params) = hq_host::report_params(&input) else {
        return Ok(());
    };
    let sent = Client::connect_pane(&dir, &token).and_then(|mut c| {
        c.set_timeout(Some(REPORT_TIMEOUT));
        c.call("agent.report", params)
    });
    drop(sent);
    Ok(())
}

/// `hq host gate`: the command a remote machine's `authorized_keys` pins an ssh
/// key to. It reads one request from stdin, refuses anything outside the
/// allowlist, and prints the host's reply as one JSON line.
fn gate(dir: &std::path::Path) -> Result<()> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin()
        .take(hq_host::MAX_GATE_INPUT as u64 + 1)
        .read_to_string(&mut input)
        .context("reading the request from stdin")?;
    match hq_host::gate_parse_request(&input) {
        Ok((method, params)) => {
            println!("{}", hq_host::gate_forward(dir, &method, params));
            Ok(())
        }
        Err(hq_host::Denied(message)) => {
            eprintln!("{}", json!({ "error": { "code": "gate_denied", "message": message } }));
            std::process::exit(hq_host::GATE_DENIED_EXIT);
        }
    }
}

/// Runs inside a Linux sandbox: forwards loopback `--port` to the unix socket.
fn relay(port: Option<u16>, unix: Option<PathBuf>) -> Result<()> {
    let (Some(port), Some(unix)) = (port, unix) else {
        bail!("usage: hq host relay --port <port> --unix <socket>");
    };
    hq_host::run_relay(port, &unix).context("running the egress relay")
}

/// Runs inside a Linux sandbox: starts the relay, then replaces itself with the agent.
fn sandbox_init(port: Option<u16>, unix: Option<PathBuf>, rest: &[String]) -> Result<()> {
    let (Some(port), Some(unix)) = (port, unix) else {
        bail!("usage: hq host sandbox-init --port <port> --unix <socket> -- <command>");
    };
    Err(hq_host::sandbox_init(port, &unix, rest)).context("starting the agent")
}
