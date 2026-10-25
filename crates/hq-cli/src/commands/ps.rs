use anyhow::Result;
use hq_core::config::HqConfig;
#[cfg(unix)]
use std::process::Command;

/// The long-running parts of HQ, each found by the command line it runs as.
#[cfg(unix)]
const COMPONENTS: [(&str, &str); 3] = [
    ("Daemon", "hq start"),
    ("Host", "hq host serve"),
    ("Web", "hq web"),
];

/// `hq web` subcommands that exit at once; only the server itself is a running component.
#[cfg(unix)]
const SHORT_LIVED_WEB: [&str; 3] = ["hq web status", "hq web stop", "hq web link"];

/// Show the HQ processes running on this machine and whether the daemon answers.
pub async fn run(config: &HqConfig) -> Result<()> {
    println!("\n── Agent HQ Processes ──\n");

    #[cfg(unix)]
    {
        for (label, pattern) in COMPONENTS {
            let pids = processes(pattern);
            let padded = format!("{label:<8}");
            if pids.is_empty() {
                println!("  {padded} not running");
            }
            for pid in pids {
                println!("  {padded} PID {pid} (uptime: {})", uptime(pid));
            }
        }
    }
    #[cfg(not(unix))]
    println!(
        "  Process listing needs a Unix system; use `hq web status` and the health line below."
    );

    let url = format!("http://127.0.0.1:{}/health", config.ws_port);
    let answering = reqwest::get(&url)
        .await
        .is_ok_and(|r| r.status().is_success());
    let state = if answering {
        "answering"
    } else {
        "not answering"
    };
    println!("\n  {url}: {state}\n");
    Ok(())
}

/// Pids whose command line runs `pattern`, never this process.
#[cfg(unix)]
fn processes(pattern: &str) -> Vec<u32> {
    let Ok(out) = Command::new("pgrep").args(["-fl", pattern]).output() else {
        return Vec::new();
    };
    let me = std::process::id();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| !SHORT_LIVED_WEB.iter().any(|s| line.contains(s)))
        .filter_map(|line| line.split_whitespace().next()?.parse::<u32>().ok())
        .filter(|pid| *pid != me)
        .collect()
}

#[cfg(unix)]
fn uptime(pid: u32) -> String {
    Command::new("ps")
        .args(["-o", "etime=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "?".to_string())
}
