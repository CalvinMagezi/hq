//! `hq claude`: add HQ to Claude Code and Claude Desktop on this computer, through the hub this
//! machine is paired with (`docs/PAIRING.md`, "What pairing sets up"). The work is
//! `hq_bridge::claude`, which the desktop app calls too.

use anyhow::{Result, anyhow, bail};
use clap::Subcommand;
use hq_bridge::claude::{self, Report, SystemRunner, Targets};
use hq_core::config::HqConfig;
use hq_db::Database;
use std::io::{BufRead, IsTerminal, Write};

const OFFER: &str = "Add HQ to Claude Code and Claude Desktop on this computer? [Y/n] ";

#[derive(Subcommand)]
pub enum ClaudeAction {
    /// Add the hub to Claude Code (user scope) and Claude Desktop
    Add {
        /// The hub's name, when this machine is paired with more than one
        hub: Option<String>,
    },
    /// Show what is in Claude Code and Claude Desktop for the hub
    Status { hub: Option<String> },
    /// Remove only the entries `hq claude add` wrote
    Remove { hub: Option<String> },
}

/// This computer's Claude, with this binary as the program Claude Desktop starts.
fn with_targets<T>(f: impl FnOnce(&Targets) -> T) -> Result<T> {
    let runner = SystemRunner::new();
    let hq_bin =
        std::env::current_exe().map_err(|e| anyhow!("cannot find this program's own path: {e}"))?;
    let targets = Targets {
        runner: &runner,
        desktop_config: claude::desktop_config_path(),
        hq_bin,
    };
    Ok(f(&targets))
}

fn print(report: &Report) -> bool {
    for line in report.lines() {
        println!("{line}");
    }
    [&report.code, &report.desktop]
        .iter()
        .any(|p| matches!(p, claude::Part::Failed(_)))
}

pub fn run(config: &HqConfig, action: ClaudeAction) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    let (ClaudeAction::Add { hub } | ClaudeAction::Status { hub } | ClaudeAction::Remove { hub }) =
        &action;
    let hub = claude::pick(&claude::hubs(&db)?, hub.as_deref())?;
    let report = with_targets(|t| match &action {
        ClaudeAction::Add { .. } => claude::add(t, &hub),
        ClaudeAction::Status { .. } => claude::status(t, &hub),
        ClaudeAction::Remove { .. } => claude::remove(t, &hub),
    })?;
    if print(&report) {
        bail!("something could not be done; see above");
    }
    Ok(())
}

/// Whether an answer to the offer is a yes. An empty line is the default, a closed input is not.
fn says_yes(bytes_read: usize, line: &str) -> bool {
    bytes_read > 0 && matches!(line.trim().to_ascii_lowercase().as_str(), "" | "y" | "yes")
}

/// After setup with a hub: asks on a terminal, and otherwise says the command. Never adds
/// without an answer, because these are the owner's files.
pub async fn offer(config: &HqConfig, hub_name: &str) {
    let command = format!("hq claude add {hub_name}");
    if !std::io::stdin().is_terminal() {
        println!("To add HQ to Claude Code and Claude Desktop on this computer, run: {command}");
        return;
    }
    print!("{OFFER}");
    let _ = std::io::stdout().flush();
    let (config, hub_name) = (config.clone(), hub_name.to_string());
    let done = tokio::task::spawn_blocking(move || {
        let mut line = String::new();
        let read = std::io::stdin().lock().read_line(&mut line).unwrap_or(0);
        if !says_yes(read, &line) {
            return Ok(false);
        }
        let db = Database::open(&config.db_path())?;
        let hub = claude::pick(&claude::hubs(&db)?, Some(&hub_name))?;
        with_targets(|t| print(&claude::add(t, &hub))).map(|_| true)
    })
    .await;
    match done {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => println!("Not added. To do it later, run: {command}"),
        Ok(Err(e)) => println!("Not added: {e:#}. To try again, run: {command}"),
        Err(e) => println!("Not added: {e}. To try again, run: {command}"),
    }
}

/// Before a revoke deletes the setup that names the hub: takes its entries out of Claude.
pub fn remove_for_revoke(db: &Database, peer: &str) {
    let removed = with_targets(|t| claude::remove_for_peer(db, t, peer));
    match removed {
        Ok(Some(report)) => {
            print(&report);
        }
        Ok(None) => {}
        Err(e) => eprintln!("warning: could not remove HQ from Claude: {e:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_yes_or_nothing_typed_says_yes_and_a_closed_input_says_no() {
        for (n, line, want) in [
            (1, "\n", true),
            (2, "y\n", true),
            (4, "Yes\n", true),
            (2, "n\n", false),
            (6, "maybe\n", false),
            (0, "", false),
        ] {
            assert_eq!(says_yes(n, line), want, "{line:?}");
        }
    }
}
