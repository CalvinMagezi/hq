//! Restart services — robust stop-then-start that works from any context
//! (CLI, slash commands, mid-conversation, nohup sessions).

use anyhow::Result;
use hq_core::config::HqConfig;

/// Restart HQ services. Stops all running instances then starts fresh.
///
/// When `background` is true, the new process is launched detached (for use
/// from slash commands or relay bots where the caller shouldn't block).
pub async fn run(config: &HqConfig, component: &str) -> Result<()> {
    println!("\n── Stopping all HQ instances ──\n");
    super::stop::run(config, component).await?;

    // Extra settle time to ensure ports are released
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    println!("\n── Starting fresh ──\n");

    // Find our own binary path
    let binary = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("hq"));

    // Launch as a detached background process so the caller doesn't block.
    // This is critical for slash-command restarts from Discord/Telegram.
    #[cfg(unix)]
    {
        use std::process::{Command, Stdio};

        let child = Command::new(&binary)
            .args(["start", component])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn();

        match child {
            Ok(c) => {
                println!(
                    "HQ started in background (PID {}). Component: {}",
                    c.id(),
                    component
                );
                println!("Use `hq status` to check, `hq stop` to stop.");
            }
            Err(e) => {
                println!("Failed to start HQ: {e}");
                println!("Binary: {}", binary.display());
                return Err(e.into());
            }
        }
    }

    #[cfg(not(unix))]
    {
        println!("Background restart not supported on this platform.");
        println!("Please run `hq start {component}` manually.");
    }

    Ok(())
}
