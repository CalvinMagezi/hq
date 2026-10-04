use anyhow::Result;
use hq_core::config::HqConfig;
use hq_daemon::instance_lock::DaemonLock;

const DEFAULT_LOG_LINES: usize = 40;

/// The daemon runs inside `hq start all`, so stopping it stops the web server
/// and relays too, and a service manager may start it straight back up.
const SERVICE_NOTE: &str = "That process also runs the web server and relays. Under a \
service manager that restarts it, stop it through systemctl or launchctl instead.";

/// Daemon management: stop, status, logs. The running daemon is found through
/// the instance lock it holds in the vault, the only place its PID is written.
pub async fn run(config: &HqConfig, sub: &str, arg: Option<&str>) -> Result<()> {
    match sub {
        "status" | "" => match DaemonLock::holder_pid(&config.vault_path) {
            Some(pid) => println!("Daemon running (PID {pid}, uptime: {})", get_uptime(pid)),
            None => println!("Daemon not running"),
        },
        "stop" => match DaemonLock::holder_pid(&config.vault_path) {
            Some(pid) => {
                terminate(pid);
                println!("Sent SIGTERM to the hq process hosting the daemon (PID {pid}).");
                println!("{SERVICE_NOTE}");
            }
            None => println!("Daemon not running"),
        },
        "logs" => {
            let n = arg
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_LOG_LINES);
            super::logs::run("daemon", n, false, false)?;
        }
        _ => println!("Usage: hq daemon [stop|status|logs [N]]"),
    }
    Ok(())
}

fn terminate(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .output();
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

fn get_uptime(pid: u32) -> String {
    #[cfg(unix)]
    {
        std::process::Command::new("ps")
            .args(["-o", "etime=", "-p", &pid.to_string()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "?".to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        "?".to_string()
    }
}
