//! The MCP config files `hq mcp install` writes, per client and per project.

use anyhow::Result;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Resolve the canonical binary path for MCP configs.
pub fn stable_binary_path() -> PathBuf {
    let system_path = hq_core::config::HqConfig::bin_path();
    if system_path.exists() {
        return system_path;
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("hq"))
}

/// Build the agent-hq MCP server entry for a given vault path.
///
/// `agent_id` is how the harness on the other end of this config says who it is.
/// Without it every message that harness sends is attributed to the generic
/// "agent", nothing can be addressed *back* to it, and its MCP session cannot be
/// handed mail mid-turn because there is no address to watch. Writing it into
/// the config is what makes a harness a participant rather than a caller.
pub fn build_mcp_server_entry(vault_path: &Path, agent_id: &str) -> Value {
    json!({
        "command": stable_binary_path().to_string_lossy(),
        "args": ["mcp-serve"],
        "env": {
            "HQ_VAULT_PATH": vault_path.to_string_lossy(),
            "HQ_AGENT_ID": agent_id,
        }
    })
}

/// Identity used when HQ writes a config for a harness it cannot name — a
/// project-level file several harnesses may read.
const GENERIC_AGENT_ID: &str = "coding-agent";

fn read_json(path: &Path) -> Result<Value> {
    let content = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

/// Every per-user client `hq mcp install --target` accepts, besides `project`.
pub const GLOBAL_CLIENTS: [&str; 7] = [
    "claude-desktop",
    "claude-code",
    "vscode",
    "cursor",
    "antigravity",
    "copilot",
    "opencode",
];

/// One MCP config file HQ writes its `agent-hq` entry into.
pub struct Target {
    pub path: PathBuf,
    /// Object holding the servers; `None` when the file is the entry itself.
    key: Option<&'static str>,
    agent_id: &'static str,
    /// Editor configs are only touched when the editor is installed.
    needs_parent: bool,
}

fn target(path: PathBuf, key: &'static str, agent_id: &'static str) -> Target {
    Target {
        path,
        key: Some(key),
        agent_id,
        needs_parent: false,
    }
}

/// The per-user config files for one client.
pub fn global_targets(client: &str) -> Result<Vec<Target>> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let targets = match client {
        "claude-desktop" => hq_core::paths::claude_desktop_config_path()
            .map(|path| Target {
                needs_parent: true,
                ..target(path, "mcpServers", "claude-code")
            })
            .into_iter()
            .collect(),
        "claude-code" => vec![
            target(home.join(".claude.json"), "mcpServers", "claude-code"),
            target(
                home.join(".claude/settings.json"),
                "mcpServers",
                "claude-code",
            ),
        ],
        "vscode" => {
            let path = if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Code/User/settings.json")
            } else {
                home.join(".config/Code/User/settings.json")
            };
            vec![Target {
                needs_parent: true,
                ..target(path, "mcp.servers", "claude-code")
            }]
        }
        "cursor" => vec![target(
            home.join(".cursor/mcp.json"),
            "mcpServers",
            "cursor",
        )],
        "antigravity" => vec![Target {
            path: home.join(".gemini/antigravity-cli/mcp/agent-hq.json"),
            key: None,
            agent_id: "antigravity",
            needs_parent: false,
        }],
        "copilot" => vec![target(
            home.join(".copilot/mcp.json"),
            "mcpServers",
            "github-copilot",
        )],
        "opencode" => vec![target(
            home.join(".config/opencode/opencode.json"),
            "mcp",
            "opencode",
        )],
        other => anyhow::bail!(
            "unknown MCP client '{other}', expected one of: {}, project",
            GLOBAL_CLIENTS.join(", ")
        ),
    };
    Ok(targets)
}

/// The project-level config files in `project_dir`, each carrying the identity
/// of the harness that reads it so two harnesses in one repo never share a
/// mailbox. `.vscode/mcp.json` is read by several tools, so it stays generic.
pub fn project_targets(project_dir: &Path) -> Vec<Target> {
    vec![
        target(project_dir.join(".mcp.json"), "mcpServers", "claude-code"),
        target(project_dir.join(".cursor/mcp.json"), "mcpServers", "cursor"),
        target(
            project_dir.join(".opencode/mcp.json"),
            "mcpServers",
            "opencode",
        ),
        target(
            project_dir.join(".vscode/mcp.json"),
            "mcpServers",
            GENERIC_AGENT_ID,
        ),
        target(project_dir.join("opencode.json"), "mcp", "opencode"),
    ]
}

/// OpenCode accepts either key, so keep whichever one a file already uses.
fn servers_key(root: &Value, key: &'static str) -> &'static str {
    if key == "mcp" && root.get("mcp").is_none() && root.get("mcpServers").is_some() {
        "mcpServers"
    } else {
        key
    }
}

/// Merge the agent-hq entry into `t`, keeping every other server. Returns
/// `None` when the client is not installed. An unparseable file is an error,
/// never overwritten.
pub fn write_target<'a>(t: &'a Target, vault_path: &Path) -> Result<Option<&'a Path>> {
    if t.needs_parent && !t.path.parent().is_some_and(Path::exists) {
        return Ok(None);
    }
    let server = build_mcp_server_entry(vault_path, t.agent_id);
    let Some(key) = t.key else {
        write_json(&t.path, &server)?;
        return Ok(Some(&t.path));
    };
    let mut root = if t.path.exists() {
        read_json(&t.path)?
    } else {
        json!({})
    };
    let key = servers_key(&root, key);
    root.as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("config is not an object"))?
        .entry(key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("{key} is not an object"))?
        .insert("agent-hq".to_string(), server);
    write_json(&t.path, &root)?;
    Ok(Some(&t.path))
}

/// Remove the agent-hq entry from `t`. Returns whether anything was removed.
pub fn remove_target(t: &Target) -> Result<bool> {
    if !t.path.exists() {
        return Ok(false);
    }
    let Some(key) = t.key else {
        std::fs::remove_file(&t.path)?;
        return Ok(true);
    };
    let mut root = read_json(&t.path)?;
    let key = servers_key(&root, key);
    let removed = root
        .get_mut(key)
        .and_then(|s| s.as_object_mut())
        .is_some_and(|s| s.remove("agent-hq").is_some());
    if removed {
        write_json(&t.path, &root)?;
    }
    Ok(removed)
}

/// Validate an MCP config file has a working agent-hq entry.
pub fn check_mcp_file(path: &Path, label: &str) -> Vec<String> {
    let mut issues = Vec::new();
    if !path.exists() {
        issues.push(format!("{label}: NOT FOUND at {}", path.display()));
        return issues;
    }

    match std::fs::read_to_string(path) {
        Ok(content) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(&content) {
                let has_cmd = parsed
                    .get("mcpServers")
                    .and_then(|s| s.get("agent-hq"))
                    .and_then(|s| s.get("command"))
                    .or_else(|| {
                        parsed
                            .get("mcp")
                            .and_then(|s| s.get("agent-hq"))
                            .and_then(|s| s.get("command"))
                    })
                    .or_else(|| parsed.get("command"))
                    .and_then(|c| c.as_str());

                if let Some(cmd) = has_cmd {
                    let cmd_path = PathBuf::from(cmd);
                    if cmd_path.exists() {
                        if cmd.contains("target/release") || cmd.contains("/bin/hq-rs") {
                            issues.push(format!(
                                "{label}: points to unstable path ({cmd}). Run `hq mcp install`."
                            ));
                        }
                    } else {
                        issues.push(format!(
                            "{label}: broken binary path ({cmd}). Run `hq mcp install`."
                        ));
                    }
                } else {
                    issues.push(format!("{label}: no agent-hq entry. Run `hq mcp install`."));
                }
            } else {
                issues.push(format!("{label}: invalid JSON"));
            }
        }
        Err(e) => issues.push(format!("{label}: read error ({e})")),
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Identity is what lets a harness be addressed at all. Without it in the
    /// config, its messages come from "agent", nothing can reply to it, and its
    /// MCP session has no inbox to watch.
    #[test]
    fn every_config_carries_the_harness_identity() {
        let entry = build_mcp_server_entry(Path::new("/vault"), "claude-code");
        assert_eq!(entry["env"]["HQ_AGENT_ID"], "claude-code");
        assert_eq!(entry["env"]["HQ_VAULT_PATH"], "/vault");
    }

    /// Two harnesses sharing an id would read each other's mail.
    #[test]
    fn harnesses_do_not_share_an_identity() {
        let cursor = build_mcp_server_entry(Path::new("/vault"), "cursor");
        let claude = build_mcp_server_entry(Path::new("/vault"), "claude-code");
        assert_ne!(cursor["env"]["HQ_AGENT_ID"], claude["env"]["HQ_AGENT_ID"]);
    }

    /// The bug this fix closes: every project-level file used to get the same
    /// generic identity, so two project-scoped harnesses in one repo collided
    /// on a single mailbox and could not be told apart.
    #[test]
    fn project_configs_do_not_share_an_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut ids = std::collections::HashMap::new();
        for t in project_targets(dir.path()) {
            write_target(&t, Path::new("/vault")).unwrap();
            let content = read_json(&t.path).unwrap();
            let agent_id = content["mcpServers"]["agent-hq"]["env"]["HQ_AGENT_ID"]
                .as_str()
                .or_else(|| content["mcp"]["agent-hq"]["env"]["HQ_AGENT_ID"].as_str())
                .unwrap_or_else(|| panic!("no agent-hq identity written to {}", t.path.display()))
                .to_string();
            ids.insert(t.path, agent_id);
        }

        let cursor_id = &ids[&dir.path().join(".cursor/mcp.json")];
        let opencode_id = &ids[&dir.path().join(".opencode/mcp.json")];
        let claude_id = &ids[&dir.path().join(".mcp.json")];

        assert_eq!(cursor_id, "cursor");
        assert_eq!(opencode_id, "opencode");
        assert_eq!(claude_id, "claude-code");
    }

    /// Merging must not drop the identity or replace unrelated servers, and
    /// removing takes out only the agent-hq entry.
    #[test]
    fn merging_preserves_other_servers_and_the_identity() {
        let dir = tempfile::tempdir().unwrap();
        let t = target(dir.path().join("mcp.json"), "mcpServers", "opencode");
        write_json(
            &t.path,
            &json!({ "mcpServers": { "other": { "command": "x" } } }),
        )
        .unwrap();

        write_target(&t, Path::new("/vault")).unwrap();
        let merged = read_json(&t.path).unwrap();
        assert_eq!(merged["mcpServers"]["other"]["command"], "x");
        assert_eq!(
            merged["mcpServers"]["agent-hq"]["env"]["HQ_AGENT_ID"],
            "opencode"
        );

        assert!(remove_target(&t).unwrap());
        let removed = read_json(&t.path).unwrap();
        assert_eq!(removed["mcpServers"]["other"]["command"], "x");
        assert!(removed["mcpServers"].get("agent-hq").is_none());
    }

    /// A config HQ cannot parse (JSONC, a typo) belongs to the user.
    #[test]
    fn an_unparseable_config_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let t = target(
            dir.path().join("settings.json"),
            "mcp.servers",
            "claude-code",
        );
        std::fs::write(&t.path, "{ // comment\n}").unwrap();
        assert!(write_target(&t, Path::new("/vault")).is_err());
        assert_eq!(std::fs::read_to_string(&t.path).unwrap(), "{ // comment\n}");
    }
}
