use anyhow::Result;
use hq_core::config::HqConfig;
use std::process::Command;

/// Show all managed processes.
pub async fn run(_config: &HqConfig) -> Result<()> {
    println!("\n── Agent HQ Processes ──\n");

    let services = [
        ("agent", "com.agent-hq.agent", "HQ Agent"),
        ("relay", "com.agent-hq.discord-relay", "Discord Relay"),
        ("relay-server", "com.agent-hq.relay-server", "Relay Server"),
        ("telegram", "com.agent-hq.telegram", "Telegram"),
        ("vault-sync", "com.agent-hq.vault-sync", "Vault Sync"),
    ];

    for (_id, daemon, label) in &services {
        let pid = get_daemon_pid(daemon);
        let padded = format!("{:<14}", label);
        match pid {
            Some(p) => {
                let uptime = get_uptime(p);
                println!("  {} PID {} (uptime: {})", padded, p, uptime);
            }
            None => {
                println!("  {} not running", padded);
            }
        }
    }

    println!();
    Ok(())
}

fn get_daemon_pid(daemon: &str) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("launchctl").args(["list"]).output().ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains(daemon) {
                let pid_str = line.split_whitespace().next()?;
                if pid_str != "-" {
                    return pid_str.parse().ok();
                }
            }
        }
    }

    // Also check PID files
    let pid_dir = if cfg!(target_os = "macos") {
        dirs::home_dir()?.join("Library/Logs")
    } else {
        dirs::home_dir()?.join(".local/share/agent-hq/pids")
    };

    let pid_file = pid_dir.join(format!("{}.pid", daemon));
    if pid_file.exists() {
        let pid_str = std::fs::read_to_string(&pid_file).ok()?;
        let pid: u32 = pid_str.trim().parse().ok()?;
        if is_alive(pid) {
            return Some(pid);
        }
    }

    None
}

fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn get_uptime(pid: u32) -> String {
    #[cfg(unix)]
    {
        Command::new("ps")
            .args(["-o", "etime=", "-p", &pid.to_string()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "?".to_string())
    }
    #[cfg(not(unix))]
    {
        "?".to_string()
    }
}
