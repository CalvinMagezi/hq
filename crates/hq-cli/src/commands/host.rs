//! `hq host`: run or inspect the built-in agent host.

use anyhow::{Context, Result, bail};
use hq_host::{Client, Host, Server, socket_path};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn default_dir() -> PathBuf {
    hq_core::config::native_host_dir()
}

pub async fn run(sub: &str, dir: Option<PathBuf>) -> Result<()> {
    let dir_arg = dir.clone();
    let dir = dir.unwrap_or_else(default_dir);
    match sub {
        "serve" => serve(dir).await,
        "status" => status(&dir),
        "stop" => stop(&dir),
        "report" => report(dir_arg),
        other => bail!("unknown subcommand '{other}': use serve, status, stop or report"),
    }
}

async fn serve(dir: PathBuf) -> Result<()> {
    let host = Arc::new(Host::new().with_state_dir(&dir));
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
    match Client::connect(dir).and_then(|mut c| c.call("host.status", json!({}))) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(())
        }
        Err(e) => bail!("host not running in {}: {e}", dir.display()),
    }
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
    let sent = Client::connect_with_token(&dir, &token).and_then(|mut c| {
        c.set_timeout(Some(REPORT_TIMEOUT));
        c.call("agent.report", params)
    });
    drop(sent);
    Ok(())
}
