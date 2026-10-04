use anyhow::Result;
use hq_core::config::HqConfig;

/// Start the HQ web dashboard (PWA).
pub async fn run(config: &HqConfig, port: u16) -> Result<()> {
    println!("\n── HQ Control Center ──\n");
    println!("Opening web dashboard at http://localhost:{}...", port);
    println!("Vault: {}\n", config.vault_path.display());

    let url = format!("http://localhost:{port}");
    let health_url = format!("{url}/health");

    let available = std::process::Command::new("curl")
        .args(["-sf", "--max-time", "2", &health_url])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);

    if available {
        println!("Dashboard server is running. Opening {url}");
    } else {
        println!("Dashboard server is not responding on {url}.");
        println!("Start it with one of:");
        println!("  hq start all");
        println!("  ./scripts/install-launchagent.sh");
        println!("\nIf HQ is already running, check:");
        println!("  curl -sf {health_url}");
    }

    // Try opening browser
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }

    Ok(())
}
