//! `hq host`: run or inspect the built-in agent host.

use anyhow::{Context, Result, bail};
use hq_host::{Client, Host, Server, socket_path};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn default_dir() -> PathBuf {
    hq_core::config::native_host_dir()
}

pub struct HostArgs {
    pub sub: String,
    pub dir: Option<PathBuf>,
    pub allow_unsandboxed: bool,
    pub key: Option<String>,
    pub from: Option<String>,
}

pub async fn run(args: HostArgs) -> Result<()> {
    let HostArgs { sub, dir, allow_unsandboxed, key, from } = args;
    let sub = sub.as_str();
    let dir_arg = dir.clone();
    let dir = dir.unwrap_or_else(default_dir);
    match sub {
        "serve" => serve(dir, allow_unsandboxed).await,
        "status" => status(&dir),
        "stop" => stop(&dir),
        "report" => report(dir_arg),
        "gate" => gate(&dir),
        "install" => super::host_install::install(),
        "authorize" => super::host_install::authorize(key.as_deref(), from.as_deref()),
        other => bail!("unknown subcommand '{other}': use serve, status, stop, install, authorize, report or gate"),
    }
}

async fn serve(dir: PathBuf, allow_unsandboxed: bool) -> Result<()> {
    let host = Arc::new(
        Host::new()
            .with_state_dir(&dir)
            .with_require_sandbox(!allow_unsandboxed),
    );
    let server = Server::bind(&dir, host.clone())
        .with_context(|| format!("starting the host in {}", dir.display()))?;
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
