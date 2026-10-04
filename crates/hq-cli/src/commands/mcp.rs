use anyhow::Result;
use hq_core::config::HqConfig;
use std::path::PathBuf;

use super::cursor_mcp_config::{self, Target};

/// Which config files an `hq mcp` subcommand touches.
#[derive(Default)]
pub struct Scope {
    /// One client from `GLOBAL_CLIENTS`, or `project`; every client when unset.
    pub target: Option<String>,
    /// Skip the project-level files.
    pub global: bool,
    /// Project directory for the project-level files (default: cwd).
    pub path: Option<PathBuf>,
}

/// Install, check, or remove HQ MCP server configuration.
pub async fn run(config: &HqConfig, sub: &str, scope: Scope) -> Result<()> {
    match sub {
        "install" | "" => install(config, &scope),
        "status" => status(config, &scope),
        "remove" => remove(&scope),
        "doctor" => doctor(config).await,
        _ => {
            println!(
                "Usage: hq mcp [install|status|remove|doctor] [path] [--target <client>] [--global]"
            );
            Ok(())
        }
    }
}

/// Entry for the old MCP-config commands so scripts that call them keep working.
pub async fn run_deprecated(config: &HqConfig, old: &str, sub: &str, scope: Scope) -> Result<()> {
    eprintln!("note: `hq {old}` is deprecated, running `hq mcp {sub}` instead");
    run(config, sub, scope).await
}

fn project_dir(scope: &Scope) -> Result<PathBuf> {
    let project = match &scope.path {
        Some(p) => p.clone(),
        None => std::env::current_dir()?,
    };
    if !project.is_dir() {
        anyhow::bail!("not a directory: {}", project.display());
    }
    Ok(project)
}

fn selected_targets(scope: &Scope) -> Result<Vec<Target>> {
    match scope.target.as_deref() {
        Some("project") => Ok(cursor_mcp_config::project_targets(&project_dir(scope)?)),
        Some(client) => cursor_mcp_config::global_targets(client),
        None => {
            let mut targets = Vec::new();
            for client in cursor_mcp_config::GLOBAL_CLIENTS {
                targets.extend(cursor_mcp_config::global_targets(client)?);
            }
            if !scope.global {
                targets.extend(cursor_mcp_config::project_targets(&project_dir(scope)?));
            }
            Ok(targets)
        }
    }
}

fn install(config: &HqConfig, scope: &Scope) -> Result<()> {
    println!("Installing HQ MCP server configuration...\n");
    let mut installed = 0;
    for t in selected_targets(scope)? {
        match cursor_mcp_config::write_target(&t, &config.vault_path) {
            Ok(Some(path)) => {
                println!("  Installed to: {}", path.display());
                installed += 1;
            }
            Ok(None) => {}
            Err(e) => eprintln!("  Warning: Could not update {}: {e}", t.path.display()),
        }
    }

    if installed == 0 {
        let entry = cursor_mcp_config::build_mcp_server_entry(&config.vault_path, "claude-code");
        println!("No supported AI editor configs found.");
        println!("Manually add the MCP server to your editor config:\n");
        println!("{}", serde_json::to_string_pretty(&entry)?);
    } else {
        println!("\nMCP server configured for {installed} target(s).");
    }
    Ok(())
}

fn status(config: &HqConfig, scope: &Scope) -> Result<()> {
    println!("MCP Server Status");
    println!("=================\n");
    for t in selected_targets(scope)? {
        let state = match std::fs::read_to_string(&t.path) {
            Ok(content) if content.contains("agent-hq") => "installed",
            Ok(_) => "present, agent-hq not configured",
            Err(_) => "not found",
        };
        println!("  {}: {state}", t.path.display());
    }
    println!("  Vault: {}", config.vault_path.display());
    Ok(())
}

/// Files the retired `hq link` injected a marked context block into.
const LEGACY_CONTEXT_FILES: &[&str] = &[
    "AGENTS.md",
    ".agent/AGENTS.md",
    "CLAUDE.md",
    ".claude/rules.md",
    ".github/copilot-instructions.md",
    ".opencode/instructions.md",
];
const LEGACY_MARKER_START: &str = "<!-- agent-hq:start -->";
const LEGACY_MARKER_END: &str = "<!-- agent-hq:end -->";

fn remove(scope: &Scope) -> Result<()> {
    // Without a path or target, only per-user configs: a repo's tracked project files change only on request.
    let touches_project = scope.path.is_some() || scope.target.as_deref() == Some("project");
    let scope = Scope {
        target: scope.target.clone(),
        global: scope.global || !touches_project,
        path: scope.path.clone(),
    };
    println!("Removing HQ MCP server from configs...\n");
    if touches_project {
        strip_legacy_context(&project_dir(&scope)?);
    }
    for t in selected_targets(&scope)? {
        match cursor_mcp_config::remove_target(&t) {
            Ok(true) => println!("  Removed from: {}", t.path.display()),
            Ok(false) => {}
            Err(e) => eprintln!("  Warning: Could not update {}: {e}", t.path.display()),
        }
    }
    println!("\nDone.");
    Ok(())
}

/// Removes the context blocks and Cursor rule the retired `hq link` wrote into a repo.
fn strip_legacy_context(project: &std::path::Path) {
    for rel in LEGACY_CONTEXT_FILES {
        let path = project.join(rel);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (Some(start), Some(end)) =
            (text.find(LEGACY_MARKER_START), text.find(LEGACY_MARKER_END))
        else {
            continue;
        };
        if end < start {
            continue;
        }
        let before = text[..start].trim_end();
        let after = text[end + LEGACY_MARKER_END.len()..].trim_start();
        let rest = match (before.is_empty(), after.is_empty()) {
            (true, true) => String::new(),
            (true, false) => after.to_string(),
            (false, true) => format!("{before}\n"),
            (false, false) => format!("{before}\n\n{after}"),
        };
        let result = if rest.is_empty() {
            std::fs::remove_file(&path)
        } else {
            std::fs::write(&path, rest)
        };
        match result {
            Ok(()) => println!("  Removed old HQ context from: {}", path.display()),
            Err(e) => eprintln!("  Warning: Could not update {}: {e}", path.display()),
        }
    }
    let rule = project.join(".cursor/rules/agent-hq.mdc");
    if rule.exists() && std::fs::remove_file(&rule).is_ok() {
        println!("  Removed: {}", rule.display());
    }
}

/// Diagnose MCP connection issues.
async fn doctor(config: &HqConfig) -> Result<()> {
    println!("HQ MCP Doctor");
    println!("=============\n");

    let mut issues: Vec<String> = Vec::new();

    let system_bin = HqConfig::bin_path();
    let bin_label = system_bin.display();
    if system_bin.exists() {
        let output = std::process::Command::new(&system_bin)
            .arg("version")
            .output();
        match output {
            Ok(o) if o.status.success() => {
                let ver = String::from_utf8_lossy(&o.stdout).trim().to_string();
                println!("  {bin_label}: OK ({ver})");
            }
            Ok(o) => {
                let code = o.status.code().unwrap_or(-1);
                println!("  {bin_label}: FAIL (exit code {code})");
                issues.push(format!(
                    "Binary at {bin_label} is broken. Rebuild and reinstall."
                ));
            }
            Err(e) => {
                println!("  {bin_label}: ERROR ({e})");
                issues.push(format!("Cannot execute {bin_label}."));
            }
        }
    } else {
        println!("  {bin_label}: NOT FOUND");
        issues.push(format!(
            "No binary at {bin_label}. Run: sudo cp target/release/hq {bin_label} (or set HQ_BIN_PATH)"
        ));
    }

    if config.vault_path.exists() {
        println!("  Vault path: OK ({})", config.vault_path.display());
    } else {
        println!("  Vault path: MISSING ({})", config.vault_path.display());
        issues.push("Vault path in ~/.hq/config.yaml does not exist.".into());
    }

    let db_path = config.db_path();
    if db_path.exists() {
        println!("  Database: OK ({})", db_path.display());
    } else {
        println!("  Database: will be created on first run");
    }

    let cwd = std::env::current_dir().unwrap_or_default();
    for (label, path) in [
        (".mcp.json", cwd.join(".mcp.json")),
        (".cursor/mcp.json", cwd.join(".cursor/mcp.json")),
    ] {
        let file_issues = cursor_mcp_config::check_mcp_file(&path, label);
        if file_issues.is_empty() {
            println!("  {label}: OK");
        } else {
            for issue in &file_issues {
                println!("  {issue}");
            }
            issues.extend(file_issues);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let global = home.join(".cursor/mcp.json");
        let global_issues = cursor_mcp_config::check_mcp_file(&global, "~/.cursor/mcp.json");
        if global_issues.is_empty() && global.exists() {
            println!("  ~/.cursor/mcp.json: OK");
        } else if !global.exists() {
            println!(
                "  ~/.cursor/mcp.json: not configured (optional; use `hq mcp install --target cursor`)"
            );
        } else {
            for issue in &global_issues {
                println!("  {issue}");
            }
            issues.extend(global_issues);
        }
    }

    println!("\n  Testing MCP handshake...");
    let binary = if system_bin.exists() {
        system_bin
    } else {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("hq"))
    };

    let child = std::process::Command::new(&binary)
        .arg("mcp-serve")
        .env(
            "HQ_VAULT_PATH",
            config.vault_path.to_string_lossy().as_ref(),
        )
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    match child {
        Ok(mut proc) => {
            use std::io::Write;
            let init_msg = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"doctor","version":"1.0"}}}"#;

            if let Some(stdin) = proc.stdin.as_mut() {
                let framed = format!("Content-Length: {}\r\n\r\n{}", init_msg.len(), init_msg);
                let _ = stdin.write_all(framed.as_bytes());
                let _ = stdin.flush();
                drop(proc.stdin.take());
            }

            std::thread::sleep(std::time::Duration::from_secs(2));
            let _ = proc.kill();
            let output = proc.wait_with_output();

            match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    let combined = format!("{stdout}{stderr}");
                    if combined.contains("agent-hq") || combined.contains("protocolVersion") {
                        println!("  MCP handshake: OK (server responds correctly)");
                    } else if stdout.is_empty() && stderr.is_empty() {
                        println!("  MCP handshake: NO RESPONSE");
                        issues.push(
                            "MCP server produced no output. Check vault path and database.".into(),
                        );
                    } else {
                        println!("  MCP handshake: UNEXPECTED RESPONSE");
                        issues.push("MCP server responded but not with expected protocol.".into());
                    }
                }
                Err(e) => {
                    println!("  MCP handshake: ERROR ({e})");
                    issues.push("Could not read MCP server output.".into());
                }
            }
        }
        Err(e) => {
            println!("  MCP handshake: SPAWN FAILED ({e})");
            issues.push("Could not start MCP server process.".into());
        }
    }

    println!();
    if issues.is_empty() {
        println!("  All checks passed. Restart Cursor to pick up MCP changes.");
    } else {
        println!("  Found {} issue(s):", issues.len());
        for issue in &issues {
            println!("    - {issue}");
        }
    }
    println!();

    Ok(())
}
