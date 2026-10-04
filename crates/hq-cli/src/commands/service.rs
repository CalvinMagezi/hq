use anyhow::Result;
use hq_core::config::HqConfig;

/// Install/uninstall/status for launchd (macOS) or systemd (Linux) service daemons.
pub async fn run(_config: &HqConfig, sub: &str, target: &str) -> Result<()> {
    match sub {
        "install" => install(target).await,
        "uninstall" => uninstall(target).await,
        "status" => status(target).await,
        _ => {
            println!("Usage: hq service <install|uninstall|status> [target]");
            println!("  Targets: all, relay, daemon");
            Ok(())
        }
    }
}

async fn install(target: &str) -> Result<()> {
    // The standalone agent worker is retired; uninstall still accepts it to clean up old units.
    if target == "agent" {
        anyhow::bail!(
            "the agent component is retired; use `hq service install all` (daemon and relay)"
        );
    }
    println!("Installing service daemons for: {target}\n");

    #[cfg(target_os = "macos")]
    {
        println!("macOS: launchd plist installation");
        println!("  The Rust binary does not yet generate plist files.");
        println!("  Use the hq binary directly as a long-running process:");
        println!("    hq start {target}");
    }

    #[cfg(target_os = "linux")]
    {
        let systemd_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".config/systemd/user");
        std::fs::create_dir_all(&systemd_dir)?;

        let hq_binary = std::env::current_exe()?;
        let targets = match target {
            "all" => vec!["daemon", "relay"],
            other => vec![other],
        };

        for t in &targets {
            let unit_name = format!("agent-hq-{}.service", t);
            let unit_path = systemd_dir.join(&unit_name);

            let unit_content = format!(
                "[Unit]\nDescription=Agent HQ - {t}\nAfter=network.target\n\n[Service]\nType=simple\nRestart=on-failure\nRestartSec=5\nExecStart={binary} start {t}\n\n[Install]\nWantedBy=default.target\n",
                t = t,
                binary = hq_binary.display(),
            );

            std::fs::write(&unit_path, unit_content)?;

            let _ = std::process::Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .output();
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "enable", "--now", &unit_name])
                .output();

            println!("  Installed: {}", unit_path.display());
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        println!("Service installation not supported on this platform.");
        println!("Run services manually with: hq start {target}");
    }

    Ok(())
}

async fn uninstall(target: &str) -> Result<()> {
    println!("Uninstalling service daemons for: {target}\n");

    #[cfg(target_os = "linux")]
    {
        let systemd_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".config/systemd/user");

        let targets = match target {
            "all" => vec!["agent", "daemon", "relay"],
            other => vec![other],
        };

        for t in &targets {
            let unit_name = format!("agent-hq-{}.service", t);
            let unit_path = systemd_dir.join(&unit_name);

            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", &unit_name])
                .output();

            if unit_path.exists() {
                std::fs::remove_file(&unit_path)?;
                println!("  Removed: {}", unit_path.display());
            }
        }

        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
    }

    #[cfg(target_os = "macos")]
    {
        let launch_agents = dirs::home_dir()
            .unwrap_or_default()
            .join("Library/LaunchAgents");

        let targets = match target {
            "all" => vec!["agent", "daemon", "relay"],
            other => vec![other],
        };

        for t in &targets {
            let plist = launch_agents.join(format!("com.agent-hq.{t}.plist"));
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist.to_string_lossy()])
                .output();

            if plist.exists() {
                std::fs::remove_file(&plist)?;
                println!("  Removed: {}", plist.display());
            }
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = target;
        println!("Service uninstall not supported on this platform.");
    }

    Ok(())
}

async fn status(target: &str) -> Result<()> {
    println!("Service status for: {target}\n");

    #[cfg(target_os = "linux")]
    {
        let targets = match target {
            "all" => vec!["daemon", "relay"],
            other => vec![other],
        };

        for t in &targets {
            let unit_name = format!("agent-hq-{}.service", t);
            match std::process::Command::new("systemctl")
                .args(["--user", "is-active", &unit_name])
                .output()
            {
                Ok(output) => {
                    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    println!("  {unit_name}: {state}");
                }
                Err(_) => println!("  {unit_name}: unknown"),
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = target;
        println!("  Service status requires systemd (Linux).");
        println!("  On macOS, use: hq ps");
    }

    Ok(())
}
