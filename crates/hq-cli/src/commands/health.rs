use anyhow::Result;
use hq_core::config::HqConfig;
use crate::render as ansi;
use crate::render::Theme;
use hq_vault::VaultClient;
#[cfg(unix)]
use std::process::Command;

/// System health check — diagnose common issues.
pub async fn run(config: &HqConfig) -> Result<()> {
    let theme = Theme::dark();

    println!("\n{}", ansi::bold("Agent-HQ Health Check", &theme.primary));
    println!("{}\n", ansi::colored(&"=".repeat(21), &theme.border));

    let mut issues = 0;

    // 1. Vault exists & scaffolded
    let vault_path = &config.vault_path;
    if vault_path.join("_system/SOUL.md").exists() {
        ok(&format!("Vault scaffolded at {}", vault_path.display()));
    } else if vault_path.exists() {
        fail(&format!(
            "Vault exists at {} but not scaffolded — run: hq setup",
            vault_path.display()
        ));
        issues += 1;
    } else {
        fail(&format!(
            "No vault found at {} — run: hq setup",
            vault_path.display()
        ));
        issues += 1;
    }

    // 2. Config file
    let config_path = HqConfig::config_read_path();
    if config_path.exists() {
        ok(&format!("Config: {}", config_path.display()));
    } else {
        fail("Config file not found — run: hq setup");
        issues += 1;
    }

    // 3. API keys
    let has_openrouter = config
        .openrouter_api_key
        .as_ref()
        .is_some_and(|k| !k.is_empty());
    let has_anthropic = config
        .anthropic_api_key
        .as_ref()
        .is_some_and(|k| !k.is_empty());
    let has_google = config
        .google_ai_api_key
        .as_ref()
        .is_some_and(|k| !k.is_empty());

    let mut keys = Vec::new();
    if has_openrouter {
        keys.push("OpenRouter");
    }
    if has_anthropic {
        keys.push("Anthropic");
    }
    if has_google {
        keys.push("Google AI");
    }

    if !keys.is_empty() {
        ok(&format!("API keys configured ({})", keys.join(" + ")));
    } else if config.has_llm_key() {
        ok("API key found in the environment (not saved to the config file)");
    } else {
        warn("No LLM API keys set — configure in ~/.hq/config.yaml or set HQ_OPENROUTER_API_KEY");
    }

    // 4. Database
    let db_path = config.db_path();
    if db_path.exists() {
        let size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
        ok(&format!("Database: {:.1} MB", size as f64 / 1_048_576.0));
    } else {
        ok("Database: not created yet (HQ creates it on first start)");
    }

    // 5. Vault stats
    if vault_path.exists() {
        match VaultClient::new(vault_path.clone()) {
            Ok(vault) => {
                if let Ok((note_count, _db_size)) = vault.get_stats() {
                    ok(&format!("Notes: {}", note_count));
                }
            }
            Err(e) => {
                fail(&format!("Could not read vault: {}", e));
                issues += 1;
            }
        }
    }

    // 6. LLM router
    println!();
    ok("All agent sessions run through HQ's built-in LLM router");

    // 7. Key ports
    println!();
    for (port, label) in [(5678, "Agent WS + web UI"), (18900, "Relay Server")] {
        if is_port_in_use(port) {
            ok(&format!(
                "Port {} ({}) in use — service likely running",
                port, label
            ));
        } else {
            dim(&format!("Port {} ({}) available", port, label));
        }
    }

    // 8. MCP
    println!();
    let claude_config = hq_core::paths::claude_desktop_config_path().unwrap_or_default();
    if claude_config.exists()
        && let Ok(content) = std::fs::read_to_string(&claude_config)
    {
        if content.contains("agent-hq") {
            ok("MCP server configured for Claude Desktop");
        } else {
            warn("Claude Desktop config exists but agent-hq MCP not configured — run: hq mcp");
        }
    }

    // Summary
    println!();
    if issues == 0 {
        println!("All checks passed. Run `hq` to start chatting.\n");
    } else {
        println!(
            "{} issue(s) found. Fix the items above and re-run `hq health`.\n",
            issues
        );
    }

    Ok(())
}

fn is_port_in_use(port: u16) -> bool {
    #[cfg(unix)]
    {
        Command::new("lsof")
            .args(["-i", &format!(":{}", port), "-sTCP:LISTEN", "-t"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // A port nobody can bind is in use.
        std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
    }
}

fn ok(msg: &str) {
    let theme = Theme::dark();
    println!("{}", ansi::status_ok(msg, &theme.success));
}

fn fail(msg: &str) {
    let theme = Theme::dark();
    println!("{}", ansi::status_fail(msg, &theme.error));
}

fn warn(msg: &str) {
    let theme = Theme::dark();
    println!("{}", ansi::status_warn(msg, &theme.warning));
}

fn dim(msg: &str) {
    let theme = Theme::dark();
    println!("{}", ansi::status_dim(msg, &theme.text_muted));
}
