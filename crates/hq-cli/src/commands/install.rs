use anyhow::Result;
use hq_core::config::HqConfig;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::content;

/// Vault directories every install creates.
const VAULT_DIRS: &[&str] = &[
    "_system",
    "_system/guides",
    "_threads/active",
    "_threads/archived",
    "_approvals/pending",
    "_approvals/resolved",
    "_logs",
    "_embeddings",
    "_agent-sessions",
    "_moc",
    "_templates",
    "_data",
    "_agents",
    "Notebooks/Memories",
    "Notebooks/Projects",
    "Notebooks/AI Intelligence",
    "Notebooks/Insights",
    "Notebooks/Diagrams",
    "Notebooks/Onboarding",
];

/// System files the user edits, so `--upgrade` leaves them alone.
const USER_EDITABLE: &[&str] = &["_system/MEMORY.md", "_system/PREFERENCES.md"];

/// Full HQ installation: scaffold vault, seed soul content, detect tools, write config.
///
/// Idempotent: running twice changes nothing. Existing files are never overwritten
/// unless `--upgrade` is passed (which updates system files but preserves user edits
/// to MEMORY.md and PREFERENCES.md).
pub async fn run(
    vault_override: Option<String>,
    non_interactive: bool,
    upgrade: bool,
    minimal: bool,
) -> Result<()> {
    println!();
    println!("  Agent-HQ Install");
    println!("  ================");
    println!();

    println!("  Step 1: Platform detection");
    let platform = Platform::detect();
    println!(
        "    OS: {}, Arch: {}, Shell: {}",
        platform.os, platform.arch, platform.shell
    );
    let vault_path = resolve_vault_path(vault_override);
    println!("    Vault: {}", vault_path.display());
    println!();

    scaffold_dirs(&vault_path)?;
    seed_system_files(&vault_path, upgrade)?;
    install_guides(&vault_path, upgrade, minimal)?;
    let keys = ApiKeys::from_env();
    let config_path = write_config(&vault_path, &keys, non_interactive)?;
    let tools = detect_and_record_tools(&vault_path, &platform, &keys)?;
    print_summary(&vault_path, &config_path, &keys, &tools);
    Ok(())
}

struct Platform {
    os: &'static str,
    arch: &'static str,
    shell: String,
}

impl Platform {
    fn detect() -> Self {
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            shell: std::env::var("SHELL").unwrap_or_else(|_| "unknown".into()),
        }
    }
}

/// Same resolution chain every other command uses (HqConfig::load(): defaults →
/// config file → HQ_VAULT_PATH env), so `hq install` never scaffolds a different
/// vault than the one `hq chat`/`hq start` will actually read from afterward.
fn resolve_vault_path(vault_override: Option<String>) -> PathBuf {
    match vault_override {
        Some(v) => PathBuf::from(v),
        None => HqConfig::load()
            .map(|c| c.vault_path)
            .unwrap_or_else(|_| HqConfig::default().vault_path),
    }
}

fn scaffold_dirs(vault_path: &Path) -> Result<()> {
    println!("  Step 2: Scaffolding vault");
    let mut dirs_created = 0;
    for dir in VAULT_DIRS {
        let full = vault_path.join(dir);
        if !full.exists() {
            std::fs::create_dir_all(&full)?;
            dirs_created += 1;
        }
    }
    println!(
        "    Created {dirs_created} directories ({} already existed)",
        VAULT_DIRS.len() - dirs_created
    );
    Ok(())
}

/// A file opts out of `--upgrade` by putting `managed: false` in its frontmatter,
/// so a per-install persona SOUL.md is not replaced by the shipped template.
fn is_unmanaged(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Some(rest) = text.strip_prefix("---\n") else {
        return false;
    };
    let frontmatter = rest.split("\n---").next().unwrap_or("");
    frontmatter.lines().any(|l| l.trim() == "managed: false")
}

fn seed_system_files(vault_path: &Path, upgrade: bool) -> Result<()> {
    println!("  Step 3: Seeding soul content");
    let mut seeded = 0;
    let mut upgraded = 0;
    for (path, file_content) in content::system_files() {
        let full = vault_path.join(path);
        if !full.exists() {
            std::fs::write(&full, file_content)?;
            seeded += 1;
        } else if upgrade && !USER_EDITABLE.contains(&path) && !is_unmanaged(&full) {
            backup_before_overwrite(&full);
            std::fs::write(&full, file_content)?;
            upgraded += 1;
        }
    }
    println!("    Seeded {seeded} system files, upgraded {upgraded}");
    Ok(())
}

fn install_guides(vault_path: &Path, upgrade: bool, minimal: bool) -> Result<()> {
    if minimal {
        println!("  Step 4: Skipped guides (--minimal)");
        return Ok(());
    }
    println!("  Step 4: Installing guide files");
    let mut guides_written = 0;
    for (path, file_content) in content::guide_files() {
        let full = vault_path.join(path);
        if full.exists() && !upgrade {
            continue;
        }
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        backup_before_overwrite(&full);
        std::fs::write(&full, file_content)?;
        guides_written += 1;
    }
    println!("    Wrote {guides_written} guide files to _system/guides/");
    Ok(())
}

/// LLM API keys found in the environment.
struct ApiKeys {
    openrouter: Option<String>,
    anthropic: Option<String>,
    google: Option<String>,
}

impl ApiKeys {
    fn from_env() -> Self {
        let first = |names: &[&str]| names.iter().find_map(|n| std::env::var(n).ok());
        Self {
            openrouter: first(&["OPENROUTER_API_KEY", "HQ_OPENROUTER_API_KEY"]),
            anthropic: first(&["ANTHROPIC_API_KEY", "HQ_ANTHROPIC_API_KEY"]),
            google: first(&[
                "GOOGLE_AI_API_KEY",
                "HQ_GOOGLE_AI_API_KEY",
                "GEMINI_API_KEY",
            ]),
        }
    }

    fn none_found(&self) -> bool {
        self.openrouter.is_none() && self.anthropic.is_none() && self.google.is_none()
    }
}

fn non_empty(key: &Option<String>) -> Option<&str> {
    key.as_deref().filter(|k| !k.is_empty())
}

fn key_status(key: &Option<String>) -> &'static str {
    if non_empty(key).is_some() {
        "configured"
    } else {
        "not configured"
    }
}

/// Step 5: write `~/.hq/config.yaml` when missing (or always with
/// `--non-interactive`). Returns the config path.
fn write_config(vault_path: &Path, keys: &ApiKeys, non_interactive: bool) -> Result<PathBuf> {
    println!("  Step 5: Writing config");
    hq_core::fs_private::create_private_dir_all(&HqConfig::hq_dir())?;
    let config_path = HqConfig::config_file_path();
    if config_path.exists() && !non_interactive {
        println!(
            "    Config exists: {} (use --non-interactive to overwrite)",
            config_path.display()
        );
        return Ok(config_path);
    }
    let mut config_content = format!(
        "vault_path: \"{}\"\ndefault_model: \"anthropic/claude-sonnet-4\"\nws_port: 5678\n",
        vault_path.display()
    );
    let entries = [
        ("openrouter_api_key", &keys.openrouter),
        ("anthropic_api_key", &keys.anthropic),
        ("google_ai_api_key", &keys.google),
    ];
    for (field, key) in entries {
        if let Some(key) = non_empty(key) {
            config_content.push_str(&format!("{field}: \"{key}\"\n"));
        }
    }
    backup_before_overwrite(&config_path);
    hq_core::fs_private::write_private(&config_path, config_content)?;
    println!("    Config: {}", config_path.display());
    Ok(config_path)
}

/// What step 6 found, for the summary.
struct DetectedTools {
    gws: Option<String>,
    mcp_status: String,
}

fn fmt_tool(version: &Option<String>, name: &str) -> String {
    match version {
        Some(ver) => ver.to_string(),
        None => format!("not installed (run `hq onboard` for {name} setup)"),
    }
}

/// Step 6: detect tools, then write CAPABILITIES.md (always, since it is
/// auto-detected) and ONBOARD.md (only when missing).
fn detect_and_record_tools(
    vault_path: &Path,
    platform: &Platform,
    keys: &ApiKeys,
) -> Result<DetectedTools> {
    println!("  Step 6: Detecting tools");
    let gws = detect_tool("gws");
    let drawit = detect_tool("drawit");
    println!("    LLM Router:  built-in (external harnesses retired)");
    println!("    gws CLI:     {}", fmt_tool(&gws, "gws"));
    println!("    DrawIt:      {}", fmt_tool(&drawit, "DrawIt"));

    let mcp_status = detect_mcp_status();
    let caps = content::capabilities_template(
        platform.os,
        platform.arch,
        &platform.shell,
        key_status(&keys.openrouter),
        key_status(&keys.anthropic),
        key_status(&keys.google),
        &fmt_tool(&gws, "gws"),
        &fmt_tool(&drawit, "DrawIt"),
        &mcp_status,
    );
    // `hq update` runs `install --upgrade` with a clean environment, which
    // would record every key and tool as missing; keep the existing file then.
    let capabilities_path = vault_path.join("_system/CAPABILITIES.md");
    let run_by_updater = std::env::var_os("HQ_UPDATE_HOOK").is_some();
    if !(run_by_updater && capabilities_path.exists()) {
        std::fs::write(&capabilities_path, caps)?;
    }

    let onboard_path = vault_path.join("_system/ONBOARD.md");
    if !onboard_path.exists() {
        std::fs::write(&onboard_path, content::onboard_template())?;
    }
    Ok(DetectedTools { gws, mcp_status })
}

fn print_summary(vault_path: &Path, config_path: &Path, keys: &ApiKeys, tools: &DetectedTools) {
    println!();
    println!("  =====================");
    println!("  Installation complete");
    println!("  =====================");
    println!();
    println!("  Vault: {}", vault_path.display());
    println!("  Config: {}", config_path.display());
    println!();

    let mut needs_attention = Vec::new();
    if keys.none_found() {
        needs_attention.push("No LLM API keys detected. Run `hq env` or `hq onboard`.");
    }
    if tools.gws.is_none() {
        needs_attention
            .push("gws CLI not found. Run `hq onboard` step 3 for Google Workspace setup.");
    }
    if tools.mcp_status.contains("not configured") {
        needs_attention.push("MCP server not configured. Run `hq mcp install` in each repo.");
    }

    if needs_attention.is_empty() {
        println!("  Everything looks good. Run `hq` to start chatting!");
    } else {
        println!("  Needs attention:");
        for item in &needs_attention {
            println!("    - {item}");
        }
        println!();
        println!("  Run `hq onboard` for an interactive walkthrough of remaining setup.");
    }
    println!();
}

/// How many rotated backups to keep per file: `.bak`, `.bak.1` .. `.bak.{MAX_BACKUPS}`.
/// Matches OpenClaw's `openclaw.json` backup convention (`.bak`..`.bak.4`, 5 generations).
const MAX_BACKUPS: u32 = 4;

/// Rotates `<path>.bak.{N}` -> `.bak.{N+1}` (oldest dropped past `MAX_BACKUPS`), then
/// copies the current file to `<path>.bak`, before an upgrade/overwrite replaces it.
/// A no-op if `path` doesn't exist yet (nothing to protect). Best-effort: a backup
/// failure is logged, not fatal — it must never block the upgrade it's protecting.
fn backup_before_overwrite(path: &std::path::Path) {
    if !path.exists() {
        return;
    }
    let bak_path = |n: u32| -> PathBuf {
        if n == 0 {
            PathBuf::from(format!("{}.bak", path.display()))
        } else {
            PathBuf::from(format!("{}.bak.{n}", path.display()))
        }
    };
    for n in (0..MAX_BACKUPS).rev() {
        let from = bak_path(n);
        if from.exists()
            && let Err(e) = std::fs::rename(&from, bak_path(n + 1))
        {
            tracing::warn!(path = %from.display(), error = %e, "install: backup rotation failed");
        }
    }
    if let Err(e) = hq_core::fs_private::copy_private(path, &bak_path(0)) {
        tracing::warn!(path = %path.display(), error = %e, "install: failed to back up before overwrite");
    }
}

/// Detect a CLI tool version, returning None if not installed.
fn detect_tool(cmd: &str) -> Option<String> {
    Command::new(cmd)
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let ver = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if ver.is_empty() { None } else { Some(ver) }
            } else {
                None
            }
        })
}

/// Check if MCP server is configured in Claude Desktop.
fn detect_mcp_status() -> String {
    let claude_config = hq_core::paths::claude_desktop_config_path().unwrap_or_default();

    if !claude_config.exists() {
        return "not configured (Claude Desktop config not found)".into();
    }

    match std::fs::read_to_string(&claude_config) {
        Ok(content) if content.contains("agent-hq") || content.contains("hq-rs") => {
            "configured for Claude Desktop".into()
        }
        Ok(_) => "not configured (Claude Desktop found, but agent-hq MCP not added)".into(),
        Err(_) => "not configured (could not read Claude Desktop config)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn backups_of_a_config_are_owner_only_even_from_a_loose_source() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.yaml");
        std::fs::write(&cfg, "openrouter_api_key: x").unwrap();
        std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o644)).unwrap();
        backup_before_overwrite(&cfg);
        backup_before_overwrite(&cfg);
        for name in ["config.yaml.bak", "config.yaml.bak.1"] {
            let mode = std::fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }

    #[test]
    fn managed_false_frontmatter_opts_out_of_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        let soul = dir.path().join("SOUL.md");

        std::fs::write(&soul, "---\nmanaged: false\n---\n# Assistant\n").unwrap();
        assert!(is_unmanaged(&soul));

        std::fs::write(&soul, "---\nversion: 3\n---\n# HQ\n").unwrap();
        assert!(!is_unmanaged(&soul));

        std::fs::write(&soul, "# HQ\nmanaged: false\n").unwrap();
        assert!(!is_unmanaged(&soul));
    }
}
