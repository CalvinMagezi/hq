use anyhow::Result;
use hq_core::config::HqConfig;
use hq_vault::VaultClient;
use std::path::PathBuf;

use super::mcp::{self, Scope};

/// List, inspect, link, and setup agent definitions and multi-agent CLI federation.
pub async fn run(config: &HqConfig, sub: &str, args: &[String]) -> Result<()> {
    let vault = VaultClient::new(config.vault_path.clone())?;

    match sub {
        "link" => {
            let path = args.first().map(PathBuf::from);
            let scope = Scope {
                target: Some("project".into()),
                path,
                ..Scope::default()
            };
            mcp::run_deprecated(config, "agents link", "install", scope).await?;
        }
        "install" => {
            let scope = Scope {
                global: true,
                ..Scope::default()
            };
            mcp::run_deprecated(config, "agents install", "install", scope).await?;
        }
        "setup" => {
            let scope = Scope {
                path: args.first().map(PathBuf::from),
                ..Scope::default()
            };
            mcp::run_deprecated(config, "agents setup", "install", scope).await?;
        }
        "list" | "ls" | "" => {
            println!("Agent Definitions");
            println!("=================\n");

            let agents_dirs = vec![("vault", config.vault_path.join("_agents"))];
            let mut found = 0;

            for (source, dir) in &agents_dirs {
                if !dir.exists() {
                    continue;
                }

                for entry in std::fs::read_dir(dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "md") {
                        let name = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();

                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let vertical = extract_fm_value(&content, "vertical")
                                .unwrap_or_else(|| "general".to_string());
                            let role = extract_fm_value(&content, "baseRole")
                                .unwrap_or_else(|| "agent".to_string());
                            let harness = extract_fm_value(&content, "preferredHarness")
                                .unwrap_or_else(|| "any".to_string());

                            println!(
                                "  {} ({}) — role: {}, harness: {} [{}]",
                                name, vertical, role, harness, source
                            );
                            found += 1;
                        }
                    }
                }
            }

            println!("\n  Built-in CLI Agent Federation Targets:");
            for target in &[
                "antigravity-cli (Google AGY)",
                "copilot-cli (GitHub Copilot)",
                "opencode (OpenCode AI)",
                "claude-code (Anthropic)",
            ] {
                println!("    - {}", target);
            }

            println!("\n  Built-in verticals:");
            for vertical in &["engineering", "qa", "research", "content", "ops"] {
                println!("    {}", vertical);
            }

            if found == 0 {
                println!("\n  No custom agent definitions found in vault.");
                println!(
                    "  Create agents in: {}",
                    config.vault_path.join("_agents").display()
                );
            }

            println!("\n  Total custom agents: {}", found);
        }
        "show" | "info" => {
            let name = args
                .first()
                .ok_or_else(|| anyhow::anyhow!("Usage: hq agents show <name>"))?;
            let path = format!("_agents/{}.md", name);

            if vault.note_exists(&path) {
                let note = vault.read_note(&path)?;
                println!("Agent: {}\n", note.title);
                println!("{}", note.content);
            } else {
                println!("Agent not found: {}", name);
                println!("Available agents: hq agents list");
            }
        }
        _ => {
            println!("Usage: hq agents <subcommand>");
            println!();
            println!("Subcommands:");
            println!("  list              List all agent definitions & CLI targets");
            println!("  show <name>       Show agent details");
            println!("  (MCP config moved to `hq mcp install`)");
        }
    }

    Ok(())
}

fn extract_fm_value(content: &str, key: &str) -> Option<String> {
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")?;
    let fm = &content[3..3 + end];
    for line in fm.lines() {
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() == 2 && parts[0].trim() == key {
            return Some(parts[1].trim().trim_matches('"').to_string());
        }
    }
    None
}
