use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

/// The systemd unit a VPS install runs everything under.
const SYSTEMD_UNIT: &str = "hq.service";

/// Show or follow recent log lines: journald when the `hq` systemd unit
/// exists, the per-component log files otherwise.
pub fn run(target: &str, lines: usize, follow: bool, errors: bool) -> Result<()> {
    let mut cmd = if has_systemd_unit() {
        journal(lines, follow, errors)
    } else {
        let files: Vec<PathBuf> = resolve_targets(target)
            .iter()
            .map(|t| log_path_for(t, errors))
            .filter(|p| p.exists())
            .collect();
        if files.is_empty() {
            println!("No log files found for: {target}");
            return Ok(());
        }
        let mut cmd = Command::new("tail");
        cmd.arg("-n").arg(lines.to_string());
        if follow {
            cmd.arg("-f");
        }
        cmd.args(files);
        cmd
    };
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("log reader exited with {status}");
    }
    Ok(())
}

fn has_systemd_unit() -> bool {
    cfg!(target_os = "linux")
        && Command::new("systemctl")
            .args(["cat", SYSTEMD_UNIT])
            .output()
            .is_ok_and(|o| o.status.success())
}

fn journal(lines: usize, follow: bool, errors: bool) -> Command {
    let mut cmd = Command::new("journalctl");
    cmd.args(["-u", SYSTEMD_UNIT, "--no-pager", "-n"])
        .arg(lines.to_string());
    if follow {
        cmd.arg("-f");
    }
    // Everything the unit writes lands at one priority, so filter on the tracing level.
    if errors {
        cmd.args(["--grep", "ERROR"]);
    }
    cmd
}

fn resolve_targets(target: &str) -> Vec<&str> {
    match target {
        "all" => vec!["relay", "daemon"],
        other => vec![other],
    }
}

fn log_path_for(target: &str, errors: bool) -> PathBuf {
    let log_dir = if cfg!(target_os = "macos") {
        dirs::home_dir().unwrap_or_default().join("Library/Logs")
    } else {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".local/share/agent-hq/logs")
    };

    let stem = match target {
        "agent" => "hq-agent".to_string(),
        "relay" | "discord" => "discord-relay".to_string(),
        "daemon" => "hq-daemon".to_string(),
        "telegram" | "tg" => "agent-hq-telegram".to_string(),
        "relay-server" => "agent-hq-relay-server".to_string(),
        "vault-sync" => "agent-hq-vault-sync".to_string(),
        "pwa" => "agent-hq-pwa".to_string(),
        other => format!("hq-{other}"),
    };
    let suffix = if errors { "error.log" } else { "log" };
    log_dir.join(format!("{stem}.{suffix}"))
}
