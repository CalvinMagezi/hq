use anyhow::Result;
use hq_core::config::HqConfig;
use std::io::{self, BufRead, Write};
use std::process::Command;

/// Interactive onboarding walkthrough.
///
/// Guides users through 6 steps of progressive feature activation.
/// Progress is tracked in `_system/ONBOARD.md` and can be resumed.
pub async fn run(config: &HqConfig, step: Option<u8>, reset: bool) -> Result<()> {
    let vault_path = &config.vault_path;
    let onboard_path = vault_path.join("_system/ONBOARD.md");

    // Ensure vault is scaffolded
    if !vault_path.join("_system/SOUL.md").exists() {
        println!("\n  Vault not scaffolded. Run `hq install` first.\n");
        return Ok(());
    }

    // Reset if requested
    if reset {
        std::fs::write(&onboard_path, super::content::onboard_template())?;
        println!("\n  Onboarding progress reset. Run `hq onboard` to start fresh.\n");
        return Ok(());
    }

    // Load current progress
    let progress = load_progress(&onboard_path);

    println!();
    println!("  Agent-HQ Onboarding");
    println!("  ====================");
    println!();
    println!("  This walkthrough configures HQ's integrations step by step.");
    println!("  Skip any step with 's'. Your progress is saved automatically.");
    println!();

    // Determine starting step
    let start = match step {
        Some(s) if (1..=6).contains(&s) => s,
        Some(s) => {
            println!("  Invalid step {s}. Valid range: 1-6.");
            return Ok(());
        }
        None => {
            // Find first non-done step
            progress
                .iter()
                .enumerate()
                .find(|(_, status)| *status != "done")
                .map(|(i, _)| (i + 1) as u8)
                .unwrap_or(1)
        }
    };

    if start > 1 {
        println!("  Resuming from step {start}.");
        println!();
    }

    let mut updated_progress = progress.clone();

    for step_num in start..=6 {
        let result = match step_num {
            1 => step_api_keys(config).await,
            2 => step_agent_harnesses().await,
            3 => step_google_workspace().await,
            4 => step_mcp_server(config).await,
            5 => step_relay_setup(config).await,
            6 => step_personalization(vault_path).await,
            _ => unreachable!(),
        };

        match result {
            Ok(StepResult::Done) => {
                updated_progress[(step_num - 1) as usize] = "done".to_string();
                save_progress(&onboard_path, &updated_progress)?;
            }
            Ok(StepResult::Skipped) => {
                updated_progress[(step_num - 1) as usize] = "skipped".to_string();
                save_progress(&onboard_path, &updated_progress)?;
            }
            Ok(StepResult::Quit) => {
                save_progress(&onboard_path, &updated_progress)?;
                println!("\n  Progress saved. Run `hq onboard` to continue later.\n");
                return Ok(());
            }
            Err(e) => {
                println!("    Error: {e}");
                println!("    Continuing to next step...");
                println!();
            }
        }
    }

    // Final summary
    println!();
    println!("  ====================");
    println!("  Onboarding Complete!");
    println!("  ====================");
    println!();

    let done_count = updated_progress.iter().filter(|s| *s == "done").count();
    let skipped_count = updated_progress.iter().filter(|s| *s == "skipped").count();
    println!("  {done_count} steps completed, {skipped_count} skipped.");
    if skipped_count > 0 {
        println!("  Run `hq onboard --step N` to revisit skipped steps.");
    }
    println!();
    println!("  You're ready to go! Run `hq` to start chatting.");
    println!();

    Ok(())
}

// ── Step results ────────────────────────────────────────────────────────────

enum StepResult {
    Done,
    Skipped,
    Quit,
}

// ── Step 1: API Keys ────────────────────────────────────────────────────────

async fn step_api_keys(config: &HqConfig) -> Result<StepResult> {
    println!("  Step 1 of 6: API Keys");
    println!("  ---------------------");
    println!();

    use super::env::{ANTHROPIC, GOOGLE_AI, OPENROUTER, is_set, prompt_key, save_keys};
    let keys = [&OPENROUTER, &ANTHROPIC, &GOOGLE_AI];

    println!("    Current status:");
    for (name, key) in ["OpenRouter:  ", "Anthropic:   ", "Google AI:   "]
        .iter()
        .zip(keys)
    {
        let state = if is_set(key, config) {
            "configured"
        } else {
            "not set"
        };
        println!("    {name}{state}");
    }
    println!();

    if keys.iter().any(|k| is_set(k, config)) {
        println!("    At least one API key is configured.");
        match prompt_choice("    Reconfigure? [y/N/s(kip)/q(uit)]: ")? {
            'y' => {}
            'q' => return Ok(StepResult::Quit),
            _ => return Ok(StepResult::Done),
        }
    }

    println!();
    println!("    You need at least one LLM API key to use HQ.");
    println!("    OpenRouter is recommended (routes to any model).");
    println!();

    let mut typed = Vec::with_capacity(keys.len());
    for (i, key) in keys.into_iter().enumerate() {
        typed.push((key, prompt_key(i + 1, key, config, "    ")?));
    }
    let (updated, has_any_cloud) = save_keys(config, typed, None)?;

    println!();
    println!(
        "    Config updated: {}",
        HqConfig::config_file_path().display()
    );
    if has_any_cloud {
        println!(
            "    Cloud provider configured, local_only: false, default_model: {}",
            updated.default_model
        );
    }
    println!();

    Ok(StepResult::Done)
}

// ── Step 2: Agent Harnesses ─────────────────────────────────────────────────

async fn step_agent_harnesses() -> Result<StepResult> {
    println!("  Step 2 of 6: LLM Router");
    println!("  -----------------------");
    println!();
    println!("    HQ uses a built-in LLM router for all agent sessions.");
    println!("    External harnesses (claude, gemini, opencode, codex) have been retired.");
    println!();
    println!("    [OK] HQ built-in LLM router: ready");
    println!("         Supports OpenRouter, Anthropic, Google AI, and Ollama.");
    println!();

    match prompt_choice("    Continue? [y/s(kip)/q(uit)]: ")? {
        'q' => Ok(StepResult::Quit),
        's' => Ok(StepResult::Skipped),
        _ => Ok(StepResult::Done),
    }
}

// ── Step 3: Google Workspace ────────────────────────────────────────────────

async fn step_google_workspace() -> Result<StepResult> {
    println!("  Step 3 of 6: Google Workspace (gws CLI)");
    println!("  ----------------------------------------");
    println!();
    println!("    The gws CLI connects HQ to Google Drive, Gmail, Calendar, and Sheets.");
    println!("    This enables calendar-aware briefs, email triage, and document management.");
    println!();

    // Check if gws is installed
    let gws_installed = Command::new("gws")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !gws_installed {
        println!("    gws CLI is not installed.");
        println!();
        println!("    Install it with:");
        println!("      brew install nicholasgasior/tap/gws     # macOS");
        println!("      # or download from https://github.com/nicholasgasior/gws/releases");
        println!();
        println!("    Install gws now and press Enter when ready, or skip this step.");

        match prompt_choice("    [Enter when ready / s(kip) / q(uit)]: ")? {
            'q' => return Ok(StepResult::Quit),
            's' => return Ok(StepResult::Skipped),
            _ => {}
        }

        // Re-check
        let rechecked = Command::new("gws")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !rechecked {
            println!("    gws still not found. Skipping authentication step.");
            println!();
            return Ok(StepResult::Skipped);
        }
    }

    let gws_ver = Command::new("gws")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".into());

    println!("    [OK] gws CLI: {gws_ver}");
    println!();

    // Check authentication by trying a quick command
    println!("    Checking authentication...");
    let auth_ok = Command::new("gws")
        .args(["calendar", "+agenda"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if auth_ok {
        println!("    [OK] Google Workspace authenticated and working.");
        println!();
        return Ok(StepResult::Done);
    }

    println!("    Google Workspace is not authenticated.");
    println!();
    println!("    Running `gws auth login` will open your browser for Google OAuth.");
    println!("    Grant access to Drive, Gmail, Calendar, and Sheets when prompted.");
    println!();

    match prompt_choice("    Launch authentication? [y/s(kip)/q(uit)]: ")? {
        'q' => return Ok(StepResult::Quit),
        's' => return Ok(StepResult::Skipped),
        _ => {}
    }

    // Launch gws auth
    println!();
    println!("    Launching browser for Google OAuth...");
    let auth_result = Command::new("gws").args(["auth", "login"]).status();

    match auth_result {
        Ok(status) if status.success() => {
            println!("    [OK] Authentication successful!");
        }
        _ => {
            println!("    Authentication may not have completed.");
            println!("    You can retry later with: gws auth login");
        }
    }

    // Verify
    println!();
    println!("    Verifying...");
    let verify = Command::new("gws")
        .args(["calendar", "+agenda"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if verify {
        println!(
            "    [OK] Google Workspace is working. Calendar, Gmail, Drive, and Sheets are accessible."
        );
    } else {
        println!(
            "    [WARN] Verification failed. Authentication may need to be completed in the browser."
        );
        println!("    Try running `gws auth login` manually if issues persist.");
    }

    println!();
    Ok(StepResult::Done)
}

// ── Step 4: MCP Server ─────────────────────────────────────────────────────

async fn step_mcp_server(config: &HqConfig) -> Result<StepResult> {
    println!("  Step 4 of 6: MCP Server");
    println!("  -----------------------");
    println!();
    println!("    The MCP server connects HQ's 51 tools to Claude Desktop or VS Code.");
    println!("    After setup, Claude Desktop can access your vault, jobs, search, and more.");
    println!();

    // Check current status
    let claude_config = hq_core::paths::claude_desktop_config_path().unwrap_or_default();

    if claude_config.exists()
        && let Ok(content) = std::fs::read_to_string(&claude_config)
        && (content.contains("agent-hq") || content.contains("hq-rs"))
    {
        println!("    [OK] MCP server already configured for Claude Desktop.");
        match prompt_choice("    Reinstall? [y/N/s(kip)/q(uit)]: ")? {
            'y' => {}
            'q' => return Ok(StepResult::Quit),
            _ => return Ok(StepResult::Done),
        }
    }

    match prompt_choice("    Install MCP server? [y/s(kip)/q(uit)]: ")? {
        'q' => return Ok(StepResult::Quit),
        's' => return Ok(StepResult::Skipped),
        _ => {}
    }

    // Run hq mcp install
    println!();
    super::mcp::run(config, "install", super::mcp::Scope::default()).await?;
    println!();

    Ok(StepResult::Done)
}

// ── Step 5: Relay Setup ─────────────────────────────────────────────────────

async fn step_relay_setup(config: &HqConfig) -> Result<StepResult> {
    println!("  Step 5 of 6: Relay Setup (Discord / Telegram)");
    println!("  ----------------------------------------------");
    println!();
    println!("    Relays let you interact with HQ from Discord or Telegram.");
    println!("    This is optional and can be configured later.");
    println!();

    let has_discord = config
        .relay
        .discord_token
        .as_ref()
        .is_some_and(|t| !t.is_empty());
    let has_telegram = config
        .relay
        .telegram_token
        .as_ref()
        .is_some_and(|t| !t.is_empty());

    println!("    Current status:");
    println!(
        "    Discord:  {}",
        if has_discord {
            "configured"
        } else {
            "not configured"
        }
    );
    println!(
        "    Telegram: {}",
        if has_telegram {
            "configured"
        } else {
            "not configured"
        }
    );
    println!();

    match prompt_choice("    Configure relays? [y/s(kip)/q(uit)]: ")? {
        'q' => return Ok(StepResult::Quit),
        's' => return Ok(StepResult::Skipped),
        _ => {}
    }

    println!();

    // Discord
    println!("    Discord Bot Setup:");
    println!("    Create a bot at https://discord.com/developers/applications");
    let discord_token = prompt_secret("    Bot token (Enter to skip): ")?;

    // Telegram
    println!("    Telegram Bot Setup:");
    println!("    Create a bot via @BotFather on Telegram");
    let telegram_token = prompt_secret("    Bot token (Enter to skip): ")?;

    if !discord_token.is_empty() || !telegram_token.is_empty() {
        // Update config with relay tokens
        let config_path = HqConfig::config_file_path();
        let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
        let mut content = existing;

        if !content.contains("relay:") {
            content.push_str("\nrelay:\n");
        }
        if !discord_token.is_empty() && !content.contains("discord_token:") {
            content.push_str(&format!(
                "  discord_token: \"{}\"\n",
                yaml_escape(&discord_token)
            ));
        }
        if !telegram_token.is_empty() && !content.contains("telegram_token:") {
            content.push_str(&format!(
                "  telegram_token: \"{}\"\n",
                yaml_escape(&telegram_token)
            ));
        }

        hq_core::fs_private::write_private(&config_path, content)?;
        println!();
        println!("    Relay config saved. Start relays with: hq start relay");
        println!();
        println!("    A relay refuses every message until it has an owner. Either set");
        println!("    relay.telegram_authorized_chat_id / relay.discord_allowed_user_ids,");
        println!("    or pair from the chat app with a one-time code:");
        if !telegram_token.is_empty() {
            super::pair::issue(&config.vault_path, hq_core::pairing::PairPlatform::Telegram)?;
        }
        if !discord_token.is_empty() {
            super::pair::issue(&config.vault_path, hq_core::pairing::PairPlatform::Discord)?;
        }
    }

    println!();
    Ok(StepResult::Done)
}

// ── Step 6: Personalization ─────────────────────────────────────────────────

async fn step_personalization(vault_path: &std::path::Path) -> Result<StepResult> {
    println!("  Step 6 of 6: Personalization");
    println!("  ----------------------------");
    println!();
    println!("    Tell HQ about yourself so agents can personalize their responses.");
    println!();

    match prompt_choice("    Configure now? [y/s(kip)/q(uit)]: ")? {
        'q' => return Ok(StepResult::Quit),
        's' => return Ok(StepResult::Skipped),
        _ => {}
    }

    println!();

    // Name
    let name = prompt_line("    Your name: ")?;

    // Role
    let role = prompt_line("    Your role (e.g., software engineer, data scientist): ")?;

    // Primary use case
    println!();
    println!("    How will you primarily use HQ?");
    println!("      1. Knowledge management (notes, research, memory)");
    println!("      2. Coding assistance (code review, refactoring, generation)");
    println!("      3. Operations (email, calendar, task management)");
    println!("      4. All of the above");
    let use_case = prompt_line("    Choice [1-4]: ")?;

    let use_case_text = match use_case.trim() {
        "1" => "knowledge management",
        "2" => "coding assistance",
        "3" => "operations and productivity",
        _ => "general-purpose (knowledge, coding, and operations)",
    };

    // Update PREFERENCES.md
    let prefs_path = vault_path.join("_system/PREFERENCES.md");
    let prefs_content = format!(
        r#"---
noteType: system-file
fileName: preferences
version: 2
pinned: true
---
# User Preferences

## User Profile

- **Name**: {name}
- **Role**: {role}
- **Primary use case**: {use_case_text}

## Communication Style

- Concise responses preferred over verbose explanations
- Show reasoning when making non-obvious decisions
- Use structured output (tables, lists) for comparisons

## Workflow Defaults

- Local-first: all data stays on this machine
- Markdown vault for knowledge management
- Frontmatter on every note for metadata consistency

## Custom Instructions

- _Add any agent-specific instructions here. These are read at every session start._
"#,
        name = if name.is_empty() {
            "(not set)"
        } else {
            name.trim()
        },
        role = if role.is_empty() {
            "(not set)"
        } else {
            role.trim()
        },
        use_case_text = use_case_text,
    );
    std::fs::write(&prefs_path, prefs_content)?;

    // Seed MEMORY.md with initial facts
    if !name.is_empty() || !role.is_empty() {
        let memory_path = vault_path.join("_system/MEMORY.md");
        let mut facts = Vec::new();
        if !name.trim().is_empty() {
            facts.push(format!("- User's name is {}.", name.trim()));
        }
        if !role.trim().is_empty() {
            facts.push(format!("- User's role: {}.", role.trim()));
        }
        facts.push(format!("- Primary use case: {use_case_text}."));

        let memory_content = format!(
            r#"---
noteType: system-file
fileName: memory
version: 2
pinned: true
---
# Agent Memory

## Key Facts

{facts}

## Active Goals

_No active goals. Goals are set through conversation._

## Session History

_Recent session summaries will appear here after the first agent interaction._
"#,
            facts = facts.join("\n"),
        );
        std::fs::write(&memory_path, memory_content)?;
    }

    println!();
    println!("    Preferences and memory updated.");
    println!();

    Ok(StepResult::Done)
}

// ── Progress tracking ───────────────────────────────────────────────────────

const STEP_NAMES: [&str; 6] = [
    "API Keys",
    "LLM Router",
    "Google Workspace",
    "MCP Server",
    "Relay Setup",
    "Personalization",
];

fn load_progress(path: &std::path::Path) -> Vec<String> {
    let default = vec!["pending".to_string(); 6];

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return default,
    };

    let mut progress = Vec::new();
    for line in content.lines() {
        // Parse table rows like: | 1 | API Keys | done | 2026-03-22 |
        if line.starts_with('|') && !line.contains("Step") && !line.contains("---") {
            let cols: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if cols.len() >= 4 {
                progress.push(cols[3].to_string());
            }
        }
    }

    if progress.len() == 6 {
        progress
    } else {
        default
    }
}

fn save_progress(path: &std::path::Path, progress: &[String]) -> Result<()> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    let mut table = String::new();
    table.push_str("| Step | Name | Status | Completed |\n");
    table.push_str("|------|------|--------|----------|\n");

    for (i, status) in progress.iter().enumerate() {
        let completed = if status == "done" {
            today.clone()
        } else {
            "—".to_string()
        };
        table.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            i + 1,
            STEP_NAMES[i],
            status,
            completed,
        ));
    }

    let content = format!(
        r#"---
noteType: system-file
fileName: onboard
version: 1
---
# Onboarding Progress

Run `hq onboard` to walk through each step interactively.
Run `hq onboard --step N` to jump to a specific step.
Run `hq onboard --reset` to start fresh.

{table}"#,
        table = table,
    );

    std::fs::write(path, content)?;
    Ok(())
}

// ── Prompt helpers ──────────────────────────────────────────────────────────

/// Escape a string for safe inclusion in a YAML double-quoted value.
/// Handles double quotes, backslashes, and newlines.
fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn prompt_line(prompt: &str) -> Result<String> {
    let mut stdout = io::stdout();
    print!("{prompt}");
    stdout.flush()?;
    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn prompt_secret(prompt: &str) -> Result<String> {
    let mut stdout = io::stdout();
    print!("{prompt}");
    stdout.flush()?;
    let secret = rpassword::read_password()?;
    Ok(secret.trim().to_string())
}

fn prompt_choice(prompt: &str) -> Result<char> {
    let input = prompt_line(prompt)?;
    Ok(input.chars().next().unwrap_or('y').to_ascii_lowercase())
}
