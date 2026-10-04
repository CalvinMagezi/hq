//! Stop running HQ components — finds and kills processes by multiple strategies.
//!
//! Strategy 1: PID files (legacy — com.agent-hq.*.pid)
//! Strategy 2: Process name matching via `pgrep` (catches nohup-launched instances)
//! Strategy 3: Port scan for known service ports

use anyhow::Result;
use hq_core::config::HqConfig;
use std::path::PathBuf;
use std::process::Command;

/// Stop running HQ components.
pub async fn run(config: &HqConfig, component: &str) -> Result<()> {
    let targets = resolve_targets(component);
    let mut stopped = 0;

    // Strategy 1: PID files
    for target in &targets {
        let pid_file = pid_path_for(config, target);
        if pid_file.exists() {
            let pid_str = std::fs::read_to_string(&pid_file)?.trim().to_string();
            if let Ok(pid) = pid_str.parse::<u32>() {
                if is_alive(pid) {
                    kill_tree(pid);
                    println!("Stopped {} (PID {}) via PID file", target, pid);
                    stopped += 1;
                } else {
                    println!("{} was not running (stale PID file)", target);
                }
            }
            let _ = std::fs::remove_file(&pid_file);
        }
    }

    // Strategy 2: Process name matching — catches nohup/background launches
    if component == "all" {
        let patterns = [
            "hq start all",
            "hq-rs start all",
            "hq start daemon",
            "hq start agent",
            "hq start telegram",
            "hq start relay",
        ];

        for pattern in &patterns {
            let killed = pkill_pattern(pattern);
            if killed > 0 {
                println!("Stopped {} process(es) matching '{}'", killed, pattern);
                stopped += killed;
            }
        }
    } else {
        let pattern = format!("hq start {component}");
        let killed = pkill_pattern(&pattern);
        if killed > 0 {
            println!("Stopped {} process(es) matching '{}'", killed, pattern);
            stopped += killed;
        }
        // Also try hq-rs variant
        let pattern2 = format!("hq-rs start {component}");
        let killed2 = pkill_pattern(&pattern2);
        if killed2 > 0 {
            println!("Stopped {} process(es) matching '{}'", killed2, pattern2);
            stopped += killed2;
        }
    }

    // Strategy 3: Free known ports
    if component == "all" || component == "ws" {
        let ws_port = config.ws_port;
        let freed = free_port(ws_port);
        if freed {
            println!("Freed port {ws_port} (WebSocket server)");
            stopped += 1;
        }
    }

    // Clean relay lock
    let relay_lock = dirs::home_dir()
        .unwrap_or_default()
        .join(".discord-relay/bot.lock");
    if relay_lock.exists() {
        let _ = std::fs::remove_file(&relay_lock);
        println!("Removed relay lock file");
    }

    if stopped == 0 {
        println!("No HQ processes were running.");
    }

    // Brief settle time for ports to release
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    Ok(())
}

fn resolve_targets(component: &str) -> Vec<String> {
    match component {
        "all" => vec![
            "agent".into(),
            "relay".into(),
            "daemon".into(),
            "relay-server".into(),
            "telegram".into(),
            "discord".into(),
        ],
        other => vec![other.to_string()],
    }
}

fn pid_path_for(_config: &HqConfig, target: &str) -> PathBuf {
    let pid_dir = if cfg!(target_os = "macos") {
        dirs::home_dir().unwrap_or_default().join("Library/Logs")
    } else {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".local/share/agent-hq/pids")
    };
    pid_dir.join(format!("com.agent-hq.{}.pid", target))
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
        let _ = pid;
        false
    }
}

/// Kill a process and all its children.
fn kill_tree(pid: u32) {
    #[cfg(unix)]
    {
        // First try graceful SIGTERM to the process group
        let _ = Command::new("kill")
            .args(["--", &format!("-{pid}")])
            .output();

        // Then SIGTERM to the specific PID
        let _ = Command::new("kill").arg(pid.to_string()).output();

        // Give it a moment to die gracefully
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Force kill if still alive
        if is_alive(pid) {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

/// Kill processes matching a pattern. Returns count killed.
fn pkill_pattern(pattern: &str) -> u32 {
    #[cfg(unix)]
    {
        // Get our own PID to exclude ourselves
        let my_pid = std::process::id();

        // Use pgrep to find matching PIDs
        let output = Command::new("pgrep").args(["-f", pattern]).output();

        match output {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let mut killed = 0;
                for line in stdout.lines() {
                    if let Ok(pid) = line.trim().parse::<u32>() {
                        // Don't kill ourselves or our parent
                        if pid == my_pid {
                            continue;
                        }
                        kill_tree(pid);
                        killed += 1;
                    }
                }
                killed
            }
            _ => 0,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pattern;
        0
    }
}

/// Kill whatever is listening on a port. Returns true if something was freed.
fn free_port(port: u16) -> bool {
    #[cfg(unix)]
    {
        let output = Command::new("lsof")
            .args(["-ti", &format!(":{port}")])
            .output();

        match output {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let mut freed = false;
                for line in stdout.lines() {
                    if let Ok(pid) = line.trim().parse::<u32>() {
                        let _ = Command::new("kill").arg(pid.to_string()).output();
                        freed = true;
                    }
                }
                freed
            }
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = port;
        false
    }
}
