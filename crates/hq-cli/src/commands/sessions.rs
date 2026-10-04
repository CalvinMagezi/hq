//! `hq sessions` — manage long-lived harness sessions from the terminal.

use anyhow::Result;
use hq_core::config::HqConfig;
use std::sync::Arc;

fn open_db(config: &HqConfig) -> Result<Arc<hq_db::Database>> {
    Ok(Arc::new(hq_db::Database::open(&config.db_path())?))
}

fn print_json(value: &serde_json::Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_default()
    );
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    config: &HqConfig,
    sub: &str,
    arg1: Option<String>,
    prompt: Option<String>,
    cwd: Option<String>,
    label: Option<String>,
    host: Option<String>,
    lines: usize,
) -> Result<()> {
    let db = open_db(config)?;
    let vault = &config.vault_path;

    match sub {
        "list" => {
            let result = hq_tools::harness_session::list(&db, None)?;
            let empty = vec![];
            let sessions = result
                .get("sessions")
                .and_then(|s| s.as_array())
                .unwrap_or(&empty);
            if sessions.is_empty() {
                println!(
                    "No harness sessions. Start one: hq sessions spawn <harness> --prompt '...'"
                );
                return Ok(());
            }
            println!(
                "{:<24} {:<12} {:<9} {:<8} {:<8} LABEL",
                "SESSION", "HARNESS", "STATUS", "HOST", "AGENT"
            );
            for s in sessions {
                let row = &s["session"];
                println!(
                    "{:<24} {:<12} {:<9} {:<8} {:<8} {}",
                    row["id"].as_str().unwrap_or("-"),
                    row["harness"].as_str().unwrap_or("-"),
                    row["status"].as_str().unwrap_or("-"),
                    row["host"].as_str().unwrap_or("-"),
                    agent_column(s),
                    row["label"].as_str().unwrap_or(""),
                );
            }
        }
        "spawn" => {
            let harness = arg1.ok_or_else(|| {
                anyhow::anyhow!(
                    "usage: hq sessions spawn <harness> [--prompt ...] [--cwd ...] [--label ...]"
                )
            })?;
            let cwd = hq_tools::harness_session::require_cwd(cwd.as_deref())?;
            let result = hq_tools::harness_session::spawn_with(
                vault,
                &db,
                hq_tools::harness_session::SpawnRequest {
                    host: host.as_deref(),
                    harness: &harness,
                    prompt: prompt.as_deref(),
                    cwd: &cwd,
                    label: label.as_deref().unwrap_or(""),
                    mission_id: None,
                    watch: None,
                    goal: Default::default(),
                },
            )
            .await?;
            print_json(&result);
        }
        "status" => {
            let id =
                arg1.ok_or_else(|| anyhow::anyhow!("usage: hq sessions status <session-id>"))?;
            print_json(&hq_tools::harness_session::status(&db, &id)?);
        }
        "logs" => {
            let id = arg1.ok_or_else(|| anyhow::anyhow!("usage: hq sessions logs <session-id>"))?;
            let result = hq_tools::harness_session::tail_log(&db, &id, lines)?;
            if let Some(log_lines) = result.get("lines").and_then(|l| l.as_array()) {
                for line in log_lines {
                    println!("{}", line.as_str().unwrap_or(""));
                }
            }
        }
        "send" => {
            let id = arg1.ok_or_else(|| {
                anyhow::anyhow!("usage: hq sessions send <session-id> --prompt '...'")
            })?;
            let text = prompt.ok_or_else(|| anyhow::anyhow!("--prompt is required for send"))?;
            print_json(&hq_tools::harness_session::send(&db, &id, &text, None)?);
        }
        "stop" => {
            let id = arg1.ok_or_else(|| anyhow::anyhow!("usage: hq sessions stop <session-id>"))?;
            print_json(&hq_tools::harness_session::stop(&db, &id)?);
        }
        "resume" => {
            let id = arg1.ok_or_else(|| {
                anyhow::anyhow!("usage: hq sessions resume <session-id> [--prompt ...]")
            })?;
            let result =
                hq_tools::harness_session::resume(vault, &db, &id, prompt.as_deref()).await?;
            print_json(&result);
        }
        other => {
            anyhow::bail!(
                "unknown subcommand '{other}'. Use: list, spawn, status, logs, send, stop, resume"
            );
        }
    }
    Ok(())
}

/// Live Herdr status, or why there is none (`away` for an unreachable host).
fn agent_column(view: &serde_json::Value) -> &str {
    if view["reachable"] == false {
        return "away";
    }
    match view["agent_status"].as_str() {
        Some(status) => status,
        None if view["alive"] == true => "up",
        None => "-",
    }
}
