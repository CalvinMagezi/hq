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
    macos::install(target).await?;

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
    macos::uninstall(target)?;

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

    #[cfg(target_os = "macos")]
    {
        let _ = target;
        macos::status().await;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = target;
        println!("  Service status needs systemd (Linux) or launchd (macOS).");
    }

    Ok(())
}

/// The daemon as a LaunchAgent: one job running `hq start all`, under the label and log names
/// `scripts/install-hq.sh` and `scripts/install-launchagent.sh` already use.
#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;
    use std::time::Duration;

    use anyhow::{Result, bail};

    use crate::commands::host_install::{LaunchAgent, user_domain};

    const LABEL: &str = "com.agent-hq.hq-all";
    /// Plists older releases named per component.
    const LEGACY_TARGETS: &[&str] = &["agent", "daemon", "relay"];
    const HEALTH_WAIT: Duration = Duration::from_secs(30);
    const HEALTH_POLL: Duration = Duration::from_secs(1);
    /// How long a daemon gets to shut down cleanly before it is killed.
    const STOP_WAIT: Duration = Duration::from_secs(10);

    fn logs() -> PathBuf {
        dirs::home_dir().unwrap_or_default().join("Library/Logs")
    }

    fn agent() -> LaunchAgent<'static> {
        LaunchAgent {
            label: LABEL,
            args: &["start", "all"],
            stdout: logs().join("hq-agent.log"),
            stderr: logs().join("hq-agent.error.log"),
        }
    }

    fn health_url() -> String {
        let port = hq_core::config::HqConfig::load()
            .map(|c| c.ws_port)
            .unwrap_or(5678);
        format!("http://127.0.0.1:{port}/health")
    }

    async fn healthy() -> bool {
        reqwest::get(health_url())
            .await
            .is_ok_and(|r| r.status().is_success())
    }

    pub(super) async fn install(target: &str) -> Result<()> {
        if !matches!(target, "all" | "daemon") {
            bail!(
                "on macOS the daemon and relays run as one LaunchAgent (`hq start all`); run `hq service install`"
            );
        }
        stop_unmanaged_daemons().await;
        let plist = agent().install()?;
        println!("  Installed: {}", plist.display());
        let deadline = std::time::Instant::now() + HEALTH_WAIT;
        while std::time::Instant::now() < deadline {
            if healthy().await {
                println!("  Daemon up at {}", health_url());
                println!("  Logs: {}", logs().join("hq-agent.log").display());
                return Ok(());
            }
            tokio::time::sleep(HEALTH_POLL).await;
        }
        println!(
            "  The LaunchAgent is loaded but {} did not answer within {}s; see `hq service status` and {}",
            health_url(),
            HEALTH_WAIT.as_secs(),
            logs().join("hq-agent.error.log").display()
        );
        Ok(())
    }

    /// Stop every `hq start all` launchd does not own, so the job is not left fighting a
    /// hand-started daemon for the port.
    async fn stop_unmanaged_daemons() {
        let managed = launchd_state().and_then(|(_, pid)| pid);
        let pids: Vec<String> = std::process::Command::new("pgrep")
            .args(["-f", "hq start all"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let others: Vec<String> = pids
            .into_iter()
            .filter(|p| Some(p) != managed.as_ref())
            .collect();
        if others.is_empty() {
            return;
        }
        println!(
            "  Stopping hq started outside launchd (pid {})",
            others.join(", ")
        );
        let signal = |sig: &str| {
            for pid in &others {
                let _ = std::process::Command::new("kill").args([sig, pid]).status();
            }
        };
        signal("-TERM");
        let alive = || {
            others.iter().any(|pid| {
                std::process::Command::new("kill")
                    .args(["-0", pid])
                    .status()
                    .is_ok_and(|s| s.success())
            })
        };
        let deadline = std::time::Instant::now() + STOP_WAIT;
        while alive() && std::time::Instant::now() < deadline {
            tokio::time::sleep(HEALTH_POLL).await;
        }
        if alive() {
            signal("-KILL");
        }
    }

    pub(super) fn uninstall(target: &str) -> Result<()> {
        let _ = target;
        if agent().uninstall()? {
            println!(
                "  Removed: {}",
                agent()
                    .plist_path(&dirs::home_dir().unwrap_or_default())
                    .display()
            );
        }
        for legacy in LEGACY_TARGETS {
            let label = format!("com.agent-hq.{legacy}");
            let old = LaunchAgent {
                label: &label,
                args: &[],
                stdout: PathBuf::new(),
                stderr: PathBuf::new(),
            };
            if old.uninstall()? {
                println!("  Removed: {label}.plist");
            }
        }
        Ok(())
    }

    /// `state` and `pid` from `launchctl print`, which has no stable machine-readable form.
    fn launchd_state() -> Option<(String, Option<String>)> {
        let out = std::process::Command::new("launchctl")
            .args(["print", &format!("{}/{LABEL}", user_domain().ok()?)])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |name: &str| {
            text.lines().map(str::trim).find_map(|l| {
                l.strip_prefix(name)?
                    .strip_prefix(" = ")
                    .map(str::to_string)
            })
        };
        Some((field("state")?, field("pid")))
    }

    pub(super) async fn status() {
        match launchd_state() {
            Some((state, pid)) => println!(
                "  {LABEL}: {state}{}",
                pid.map(|p| format!(" (pid {p})")).unwrap_or_default()
            ),
            None => println!("  {LABEL}: not installed (run `hq service install`)"),
        }
        let health = if healthy().await {
            "answering"
        } else {
            "not answering"
        };
        println!("  {}: {health}", health_url());
    }
}
