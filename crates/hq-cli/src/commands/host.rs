//! `hq host`: run or inspect the built-in agent host.

use anyhow::{Context, Result, bail};
use hq_core::config::HqConfig;
use hq_host::{Client, Host, Server, socket_path};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

fn default_dir() -> PathBuf {
    HqConfig::hq_dir().join("run").join("host")
}

pub async fn run(sub: &str, dir: Option<PathBuf>) -> Result<()> {
    let dir = dir.unwrap_or_else(default_dir);
    match sub {
        "serve" => serve(dir).await,
        "status" => status(&dir),
        "stop" => stop(&dir),
        other => bail!("unknown subcommand '{other}': use serve, status or stop"),
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
