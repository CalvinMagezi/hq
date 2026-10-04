//! Force-kill ALL Agent HQ processes — nuclear option.

use anyhow::Result;
use hq_core::config::HqConfig;

/// Force-kill all Agent HQ processes including child harnesses.
pub async fn run(config: &HqConfig) -> Result<()> {
    println!("Killing all Agent HQ processes...\n");

    // First do a graceful stop
    super::stop::run(config, "all").await?;

    // Then force-kill any remaining patterns
    #[cfg(unix)]
    {
        use std::process::Command;

        let force_patterns = [
            // HQ binary
            "hq start",
            "hq-rs start",
        ];

        let mut killed = 0;
        let my_pid = std::process::id();

        for pattern in &force_patterns {
            let output = Command::new("pgrep").args(["-f", pattern]).output();
            if let Ok(o) = output
                && o.status.success()
            {
                let stdout = String::from_utf8_lossy(&o.stdout);
                for line in stdout.lines() {
                    if let Ok(pid) = line.trim().parse::<u32>()
                        && pid != my_pid
                    {
                        let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
                        killed += 1;
                    }
                }
            }
        }

        if killed > 0 {
            println!("Force-killed {} remaining process(es).", killed);
        } else {
            println!("No remaining processes to kill.");
        }
    }

    // Clean all PID files
    let pid_dir = if cfg!(target_os = "macos") {
        dirs::home_dir().unwrap_or_default().join("Library/Logs")
    } else {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".local/share/agent-hq/pids")
    };

    if pid_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&pid_dir)
    {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("com.agent-hq.") {
                let _ = std::fs::remove_file(entry.path());
            }
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

    println!("\nAll HQ processes terminated.");
    Ok(())
}
