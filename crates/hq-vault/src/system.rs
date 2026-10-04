//! System context — SOUL.md, MEMORY.md, PREFERENCES.md, HEARTBEAT.md
//!
//! The `_system/` directory in the vault holds identity and state files
//! that define agent personality, memory, preferences, and heartbeat.

use anyhow::{Context, Result};
use hq_core::types::{Note, PinnedScanReport, SystemContext};
use std::collections::HashMap;
use std::path::Path;
use tracing::debug;

const SYSTEM_DIR: &str = "_system";

const SOUL_FILE: &str = "SOUL.md";
const MEMORY_FILE: &str = "MEMORY.md";
const PREFERENCES_FILE: &str = "PREFERENCES.md";
const HEARTBEAT_FILE: &str = "HEARTBEAT.md";
const CONFIG_FILE: &str = "CONFIG.md";
const USER_FILE: &str = "USER.md";
const PURPOSE_FILE: &str = "PURPOSE.md";

/// Operators personalize their instance through `SOUL.md`; this generic line
/// only covers a vault that has none yet.
const DEFAULT_ASSISTANT_IDENTITY: &str = "You are HQ, a local-first AI agent hub";

/// Read a system file by name (e.g. "SOUL.md") from `_system/`.
/// Returns the raw content string or an empty string if the file doesn't exist.
pub(crate) fn read_system_file(vault_path: &Path, name: &str) -> Result<String> {
    let path = vault_path.join(SYSTEM_DIR).join(name);
    if !path.exists() {
        debug!(file = name, "system file does not exist, returning empty");
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).with_context(|| format!("reading system file: {}", name))
}

/// A `_system/` file's body with frontmatter stripped and trimmed; empty if absent or unreadable.
fn read_stripped(vault_path: &Path, name: &str) -> String {
    let raw = read_system_file(vault_path, name).unwrap_or_default();
    hq_core::frontmatter_utils::strip_frontmatter(raw.trim()).trim().to_string()
}

/// Canonical SOUL loader — single source of truth for HQ identity.
///
/// Reads `_system/SOUL.md` from the vault, strips YAML frontmatter, and
/// returns the body.
///
/// Every interface (CLI, relay, web) should call this function to get the
/// HQ soul. Interface-specific context (environment, model, tool instructions)
/// should be appended by the caller, not baked into the soul.
pub fn load_soul(vault_path: &Path) -> String {
    let body = read_stripped(vault_path, SOUL_FILE);
    if !body.is_empty() {
        return body;
    }
    // Keeps the agent knowing it is HQ before the vault has been initialized.
    format!(
        "# HQ Identity (fallback)\n\n{DEFAULT_ASSISTANT_IDENTITY}. You are an independent operator: you manage your own vault, your own source code, and the coding agents you launch. You are one continuous identity across Telegram, Discord, Web, and CLI. You are NOT a generic language model. You have local filesystem access and a markdown vault. Never claim you lack tools or file access."
    )
}

/// Load the user model from `_system/USER.md`.
///
/// Returns the body with YAML frontmatter stripped, or an empty string if
/// the file doesn't exist.
pub fn get_user_model(vault_path: &Path) -> String {
    read_stripped(vault_path, USER_FILE)
}

/// Read curated objectives (PURPOSE.md), frontmatter stripped. Empty if absent.
pub(crate) fn read_purpose(vault_path: &Path) -> String {
    read_stripped(vault_path, PURPOSE_FILE)
}

/// The one voice: who HQ is and what we're working toward. SELF.md is not
/// read: the task that generated it is retired, so its runtime facts only age.
pub fn identity_preamble(vault_path: &Path) -> String {
    const SOUL_CAP: usize = 2000;
    const PURPOSE_CAP: usize = 1000;
    let cap = |text: &str, n: usize| text.trim().chars().take(n).collect::<String>();
    let soul = cap(&load_soul(vault_path), SOUL_CAP);
    let purpose = cap(&read_purpose(vault_path), PURPOSE_CAP);
    let mut s = String::new();
    s.push_str("# Who I am\n");
    s.push_str(&soul);
    if !purpose.is_empty() {
        s.push_str("\n\n# What we're working toward\n");
        s.push_str(&purpose);
    }
    s
}

/// Compact identity block for injection into subagents and daemon tasks.
///
/// Combines the first ~3 lines of SOUL.md with key facts from USER.md.
/// Target: ~150-200 tokens — enough to ground identity without budget pressure.
pub fn get_soul_summary(vault_path: &Path) -> String {
    let soul = load_soul(vault_path);
    let user = get_user_model(vault_path);

    // Prefer the SOUL's `## Essence` section (the canonical compact identity);
    // fall back to the first 3 non-empty lines for souls without one.
    let soul_excerpt: String = extract_section(&soul, "## Essence").unwrap_or_else(|| {
        soul.lines()
            .filter(|l| !l.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .join("\n")
    });

    // Prefer the "# Inferred Notes" section from the user model if present.
    let user_excerpt = if let Some(idx) = user.find("# Inferred Notes") {
        let notes = user[idx..].lines().take(5).collect::<Vec<_>>().join("\n");
        format!("\n\n## About the User\n{notes}")
    } else if !user.is_empty() {
        "\n\n## User Model: see _system/USER.md".to_string()
    } else {
        String::new()
    };

    format!("## HQ Identity (Summary)\n{soul_excerpt}{user_excerpt}")
}

/// Extract the body of a markdown section by its exact heading line, up to the
/// next heading of the same or higher level. Returns None if absent or empty.
fn extract_section(text: &str, heading: &str) -> Option<String> {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let mut lines = text.lines();
    lines.find(|l| l.trim() == heading)?;
    let body: Vec<&str> = lines
        .take_while(|l| {
            let hashes = l.chars().take_while(|c| *c == '#').count();
            !(hashes > 0 && hashes <= level && l.trim_start().starts_with('#'))
        })
        .collect();
    let body = body.join("\n").trim().to_string();
    if body.is_empty() { None } else { Some(body) }
}

/// Write a system file by name to `_system/`.
pub(crate) fn write_system_file(vault_path: &Path, name: &str, content: &str) -> Result<()> {
    let dir = vault_path.join(SYSTEM_DIR);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    std::fs::write(&path, content)?;
    debug!(file = name, "wrote system file");
    Ok(())
}

/// Read all system files and pinned notes into a `SystemContext`.
pub fn get_system_context(vault_path: &Path) -> Result<SystemContext> {
    let soul = read_system_file(vault_path, SOUL_FILE)?;
    let memory = read_system_file(vault_path, MEMORY_FILE)?;
    let preferences = read_system_file(vault_path, PREFERENCES_FILE)?;
    let heartbeat = read_system_file(vault_path, HEARTBEAT_FILE)?;

    // Parse config key-value table from CONFIG.md
    let config_raw = read_system_file(vault_path, CONFIG_FILE)?;
    let config = parse_config_table(&config_raw);

    // Scan for pinned notes
    let (pinned_notes, pinned_scan) = get_pinned_notes_report(vault_path)?;

    Ok(SystemContext {
        soul,
        memory,
        preferences,
        heartbeat,
        config,
        pinned_notes,
        pinned_scan,
    })
}

/// Directories scanned for pinned notes, relative to vault root. `_system/`
/// is HQ's own natural home for a self-maintained pinned note (see
/// FEATURE-REQUESTS.md FR-002), so it must be scanned like any other.
const PINNED_SCAN_DIRS: &[&str] = &["Notebooks", "_system", "_plans", "_moc"];

/// Scan [`PINNED_SCAN_DIRS`] recursively for notes with `pinned: true`, at
/// most 10 in total, and report which directories existed vs. were missing
/// so a caller can surface the scan instead of silently returning nothing.
pub(crate) fn get_pinned_notes_report(vault_path: &Path) -> Result<(Vec<Note>, PinnedScanReport)> {
    let mut pinned = Vec::new();
    let mut report = PinnedScanReport::default();

    for dir_name in PINNED_SCAN_DIRS {
        // Once the cap is hit, stop entirely rather than keep marking later
        // directories "scanned" without ever reading them — a directory this
        // loop never opened must not be reported as scanned, since that is
        // exactly the silent failure FR-002 exists to make visible.
        if pinned.len() >= 10 {
            break;
        }
        let dir = vault_path.join(dir_name);
        if !dir.exists() {
            report.skipped.push((*dir_name).to_string());
            continue;
        }
        report.scanned.push((*dir_name).to_string());
        collect_pinned(&dir, vault_path, &mut pinned, 10)?;
    }

    Ok((pinned, report))
}

fn collect_pinned(dir: &Path, vault_root: &Path, out: &mut Vec<Note>, limit: usize) -> Result<()> {
    if out.len() >= limit {
        return Ok(());
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        if out.len() >= limit {
            break;
        }
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with('.') {
                collect_pinned(&path, vault_root, out, limit)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            // Quick check for pinned frontmatter without full parse
            if let Ok(content) = std::fs::read_to_string(&path)
                && (content.contains("pinned: true") || content.contains("pinned: yes"))
                && let Ok(rel) = path.strip_prefix(vault_root)
            {
                let rel_str = rel.to_string_lossy().to_string();
                if let Ok(note) = crate::notes::read_note(vault_root, &rel_str) {
                    out.push(note);
                }
            }
        }
    }

    Ok(())
}

/// Parse a markdown table (key-value) from CONFIG.md.
fn parse_config_table(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    let body = hq_core::frontmatter_utils::strip_frontmatter(raw);

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            let cells: Vec<&str> = trimmed
                .trim_matches('|')
                .split('|')
                .map(|s| s.trim())
                .collect();
            if cells.len() >= 2 {
                let key = cells[0].trim();
                let value = cells[1].trim();
                // Skip header row and separator
                if !key.is_empty()
                    && !value.is_empty()
                    && key != "Key"
                    && !key.starts_with("---")
                    && !key.starts_with('-')
                {
                    map.insert(key.to_string(), value.to_string());
                }
            }
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_note_in_system_dir_loads() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("_system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("FEATURE-REQUESTS.md"),
            "---\npinned: true\n---\n\n# Feature Requests\n",
        )
        .unwrap();

        let (pinned, report) = get_pinned_notes_report(dir.path()).unwrap();
        assert_eq!(pinned.len(), 1);
        assert!(report.scanned.contains(&"_system".to_string()));
    }

    #[test]
    fn directories_past_the_cap_are_neither_scanned_nor_skipped() {
        // Regression: once 10 pinned notes are found, the loop must stop
        // instead of marking every remaining directory "scanned" without
        // ever opening it.
        let dir = tempfile::tempdir().unwrap();
        let notebooks = dir.path().join("Notebooks");
        std::fs::create_dir_all(&notebooks).unwrap();
        for i in 0..10 {
            std::fs::write(
                notebooks.join(format!("n{i}.md")),
                "---\npinned: true\n---\n\nnote",
            )
            .unwrap();
        }
        std::fs::create_dir_all(dir.path().join("_system")).unwrap();

        let (pinned, report) = get_pinned_notes_report(dir.path()).unwrap();
        assert_eq!(pinned.len(), 10);
        assert!(report.scanned.contains(&"Notebooks".to_string()));
        assert!(
            !report.scanned.contains(&"_system".to_string()),
            "must not claim a directory was scanned when the cap stopped the loop first: {report:?}"
        );
        assert!(
            !report.skipped.contains(&"_system".to_string()),
            "must not claim a directory was skipped (missing) when it exists but was never reached: {report:?}"
        );
    }

    #[test]
    fn soul_summary_prefers_essence_section() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("_system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("SOUL.md"),
            "# HQ\n\nPreamble line.\n\n## Essence\n\nSovereign digital assistant.\nOne identity across interfaces.\n\n## Operating Principles\n\nAct then report.",
        )
        .unwrap();
        let summary = get_soul_summary(dir.path());
        assert!(summary.contains("Sovereign digital assistant."));
        assert!(summary.contains("One identity across interfaces."));
        assert!(!summary.contains("Act then report."));
    }

    #[test]
    fn soul_summary_falls_back_to_first_lines_without_essence() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("_system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("SOUL.md"),
            "# HQ\nLine one.\nLine two.\nLine three.",
        )
        .unwrap();
        let summary = get_soul_summary(dir.path());
        assert!(summary.contains("Line one."));
        assert!(!summary.contains("Line three."));
    }

    #[test]
    fn identity_preamble_includes_soul_and_purpose_but_not_self() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("_system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(sys.join("SOUL.md"), "# Soul\nI am HQ.").unwrap();
        std::fs::write(sys.join("PURPOSE.md"), "# Purpose\n## Active\n### Ship SP3").unwrap();
        std::fs::write(sys.join("SELF.md"), "# Self\nCan: edit vault.").unwrap();
        let p = identity_preamble(dir.path());
        assert!(p.contains("I am HQ"));
        assert!(p.contains("Ship SP3"));
        assert!(!p.contains("Can: edit vault"));
    }

    #[test]
    fn test_parse_config_table() {
        let raw = r#"---
noteType: system-file
---
# Configuration

| Key | Value |
|-----|-------|
| DEFAULT_MODEL | gemini-2.5-flash |
| orchestration_mode | internal |
"#;
        let config = parse_config_table(raw);
        assert_eq!(config.get("DEFAULT_MODEL").unwrap(), "gemini-2.5-flash");
        assert_eq!(config.get("orchestration_mode").unwrap(), "internal");
    }

    #[test]
    fn test_get_soul_summary_fallback() {
        // With an empty temp dir (no vault files), the fallback identity kicks in
        // and get_soul_summary must still return a non-empty, well-formed string.
        let dir = tempfile::tempdir().expect("tempdir");
        let summary = get_soul_summary(dir.path());
        assert!(!summary.is_empty(), "summary must not be empty");
        assert!(
            summary.starts_with("## HQ Identity (Summary)"),
            "summary must start with the expected header"
        );
        // Fallback soul line should be present.
        assert!(
            summary.contains("HQ") || summary.contains("Alex"),
            "summary must reference HQ identity"
        );
    }
}
