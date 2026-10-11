//! `hq stack`: the Agentic Stack from a terminal. `list` shows each specialist
//! with its lane, what is running and the day's count; `run <name> <task>`
//! starts a run for one task now instead of waiting for the next sweep.

use anyhow::{Result, bail};
use hq_core::config::HqConfig;
use std::sync::Arc;

pub async fn run(config: &HqConfig, sub: &str, name: Option<String>, task: Option<String>, json: bool) -> Result<()> {
    let db = Arc::new(hq_db::Database::open(&config.db_path())?);
    match sub {
        "list" => {
            let rows = hq_tools::harness_session::stack::status(&db, config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
                return Ok(());
            }
            if rows.is_empty() {
                println!("No specialists. Add a `stack:` block to the config (see docs/plans/agentic-stack.md).");
                return Ok(());
            }
            println!("{:<14} {:<12} {:<26} {:>6} {:>8} RUNNING", "SPECIALIST", "HARNESS", "LANE", "QUEUE", "RUNS/24H");
            for r in &rows {
                let lane: Vec<&str> = r["purposes"].as_array().map(|a| a.iter().filter_map(|p| p.as_str()).collect()).unwrap_or_default();
                let running: Vec<&str> = r["running"].as_array().map(|a| a.iter().filter_map(|p| p.as_str()).collect()).unwrap_or_default();
                let state = match (r["enabled"].as_bool(), r["problem"].as_str()) {
                    (Some(false), _) => " (disabled)".to_string(),
                    (_, Some(problem)) => format!(" (problem: {problem})"),
                    _ => String::new(),
                };
                println!(
                    "{:<14} {:<12} {:<26} {:>6} {:>3}/{:<4} {}{}",
                    r["name"].as_str().unwrap_or("-"),
                    r["harness"].as_str().unwrap_or("-"),
                    lane.join(","),
                    r["queued"].as_i64().unwrap_or(0),
                    r["runs_last_24h"].as_i64().unwrap_or(0),
                    r["runs_per_day"].as_i64().unwrap_or(0),
                    if running.is_empty() { "-".to_string() } else { running.join(",") },
                    state
                );
            }
        }
        "run" => {
            let (Some(name), Some(task)) = (name, task) else {
                bail!("usage: hq stack run <specialist> <task id or display id>");
            };
            let resolve = |host: Option<&str>| hq_tools::agent_host::host(host);
            let report = hq_tools::harness_session::stack::run_for(&config.vault_path, &db, config, &name, &task, &resolve).await?;
            println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
        }
        other => bail!("unknown subcommand '{other}' (list, run <specialist> <task>)"),
    }
    Ok(())
}
