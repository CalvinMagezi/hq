use anyhow::Result;
use hq_core::config::HqConfig;

/// Remove stale locks and orphaned processes.
pub async fn run(_config: &HqConfig) -> Result<()> {
    println!("\n── Cleaning stale state ──\n");

    let mut cleaned = 0;

    // Clean PID files for dead processes
    let pid_dirs = if cfg!(target_os = "macos") {
        vec![dirs::home_dir().unwrap_or_default().join("Library/Logs")]
    } else {
        vec![
            dirs::home_dir()
                .unwrap_or_default()
                .join(".local/share/agent-hq/pids"),
        ]
    };

    for pid_dir in &pid_dirs {
        if !pid_dir.exists() {
            continue;
        }

        for entry in std::fs::read_dir(pid_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "pid")
                && let Ok(pid_str) = std::fs::read_to_string(&path)
                && let Ok(pid) = pid_str.trim().parse::<u32>()
                && !is_alive(pid)
            {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                std::fs::remove_file(&path)?;
                println!("  Removed stale PID file: {} (PID {})", name, pid);
                cleaned += 1;
            }
        }
    }

    // Clean lock files
    let lock_files = [
        dirs::home_dir()
            .unwrap_or_default()
            .join(".discord-relay/bot.lock"),
        _config.vault_path.join("_sync/sync.lock"),
    ];

    for lock_file in &lock_files {
        if lock_file.exists()
            && let Ok(content) = std::fs::read_to_string(lock_file)
        {
            let pid: u32 = content.trim().parse().unwrap_or(0);
            if pid == 0 || !is_alive(pid) {
                let name = lock_file.file_name().unwrap_or_default().to_string_lossy();
                std::fs::remove_file(lock_file)?;
                println!("  Removed stale lock: {}", name);
                cleaned += 1;
            } else {
                let name = lock_file.file_name().unwrap_or_default().to_string_lossy();
                println!("  Lock held by active PID {}: {}", pid, name);
            }
        }
    }

    if cleaned == 0 {
        println!("  No stale state found.");
    }

    println!("\n  Clean complete.\n");
    Ok(())
}

fn is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        hq_core::heartbeat::is_pid_alive(pid)
    }
}
