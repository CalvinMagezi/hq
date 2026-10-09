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

/// How much of HQ a stdio MCP entry may reach. It becomes `hq mcp-serve --scope <x>`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum ServeScope {
    /// Every tool: for the owner's own harnesses.
    #[default]
    Full,
    /// Vault reads plus filing and updating tasks; no sessions, no code execution.
    Tasks,
    /// Vault reads only.
    Readonly,
}

impl ServeScope {
    /// The tools this scope may call; `None` is unrestricted.
    pub fn allowlist(self) -> Option<&'static [&'static str]> {
        match self {
            Self::Full => None,
            Self::Tasks => Some(hq_mcp::gateway::TASKS_ALLOWLIST),
            Self::Readonly => Some(hq_mcp::gateway::SPARK_READONLY_ALLOWLIST),
        }
    }

    fn arg(self) -> Option<&'static str> {
        match self {
            Self::Full => None,
            Self::Tasks => Some("tasks"),
            Self::Readonly => Some("readonly"),
        }
    }
}

/// What `hq mcp install` writes beyond the default full-access stdio entry.
#[derive(Clone, Debug, Default)]
pub struct EntryOptions {
    /// Scope the stdio entry's `mcp-serve` runs with.
    pub scope: ServeScope,
    /// Point at a remote HQ's `/mcp` instead of launching a local server. Only
    /// the VS Code configs take this form, and the key is asked for when VS Code
    /// starts the server, never written to the file.
    pub remote_url: Option<String>,
}

/// Id of the VS Code `inputs` entry that holds the remote key.
const VSCODE_KEY_INPUT: &str = "agent-hq-key";

/// Build the agent-hq MCP server entry for a given vault path.
///
/// `agent_id` is how the harness on the other end of this config says who it is.
/// Without it every message that harness sends is attributed to the generic
/// "agent", nothing can be addressed *back* to it, and its MCP session cannot be
/// handed mail mid-turn because there is no address to watch. Writing it into
/// the config is what makes a harness a participant rather than a caller.
pub fn build_mcp_server_entry(vault_path: &Path, agent_id: &str) -> Value {
    build_stdio_entry(vault_path, agent_id, ServeScope::Full, false)
}

fn build_stdio_entry(vault_path: &Path, agent_id: &str, scope: ServeScope, vscode: bool) -> Value {
    let mut args = vec!["mcp-serve".to_string()];
    if let Some(scope) = scope.arg() {
        args.push("--scope".into());
        args.push(scope.into());
    }
    let mut entry = json!({
        "command": stable_binary_path().to_string_lossy(),
        "args": args,
        "env": {
            "HQ_VAULT_PATH": vault_path.to_string_lossy(),
            "HQ_AGENT_ID": agent_id,
        }
    });
    if vscode {
        entry["type"] = json!("stdio");
    }
    entry
}

/// The VS Code form of a remote entry. The key is a `promptString` input, so
/// VS Code asks for it and keeps it out of the file.
fn build_remote_entry(url: &str) -> Value {
    json!({
        "type": "http",
        "url": url,
        "headers": { "Authorization": format!("Bearer ${{input:{VSCODE_KEY_INPUT}}}") },
    })
}

/// What to paste into a VS Code `mcp.json` for a remote HQ, when no file could be written.
pub fn remote_snippet(url: &str) -> Result<String> {
    let mut root = serde_json::Map::new();
    add_key_input(&mut root)?;
    root.insert(
        "servers".into(),
        json!({ "agent-hq": build_remote_entry(url) }),
    );
    Ok(serde_json::to_string_pretty(&Value::Object(root))?)
}

/// A remote HQ must be reached over TLS, except on this machine.
pub fn validate_remote_url(url: &str) -> Result<()> {
    if url.trim() != url || url.chars().any(char::is_whitespace) {
        anyhow::bail!("the URL must not contain whitespace");
    }
    let (secure, rest) = match url.split_once("://") {
        Some(("https", rest)) => (true, rest),
        Some(("http", rest)) => (false, rest),
        _ => anyhow::bail!("the URL must start with https:// (or http:// for this machine)"),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        anyhow::bail!("the URL has no host");
    }
    if authority.contains('@') {
        anyhow::bail!("the URL must not carry credentials; the key is asked for by VS Code");
    }
    if url.contains('?') || url.contains('#') {
        anyhow::bail!("the URL must not carry a query or fragment");
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    if !secure && !loopback {
        anyhow::bail!("http:// is only allowed for localhost; use https:// for a remote HQ");
    }
    Ok(())
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
    /// A VS Code `mcp.json`: entries carry a `type`, and a remote form is accepted.
    vscode: bool,
    /// An old location HQ no longer writes. `install` migrates its entry away,
    /// and `status` and `remove` still look there.
    pub legacy: bool,
}

fn target(path: PathBuf, key: &'static str, agent_id: &'static str) -> Target {
    Target {
        path,
        key: Some(key),
        agent_id,
        needs_parent: false,
        vscode: false,
        legacy: false,
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
            // `dirs::config_dir` is %APPDATA% on Windows, ~/Library/Application Support
            // on macOS and ~/.config elsewhere, which is where VS Code keeps its user data.
            let user = dirs::config_dir()
                .ok_or_else(|| anyhow::anyhow!("no config directory"))?
                .join("Code/User");
            vec![
                Target {
                    needs_parent: true,
                    vscode: true,
                    ..target(user.join("mcp.json"), "servers", "claude-code")
                },
                Target {
                    needs_parent: true,
                    legacy: true,
                    ..target(user.join("settings.json"), "mcp.servers", "claude-code")
                },
            ]
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
            vscode: false,
            legacy: false,
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
/// VS Code's own key there is `servers`; an older HQ wrote `mcpServers`, which it ignores.
pub fn project_targets(project_dir: &Path) -> Vec<Target> {
    vec![
        target(project_dir.join(".mcp.json"), "mcpServers", "claude-code"),
        target(project_dir.join(".cursor/mcp.json"), "mcpServers", "cursor"),
        target(
            project_dir.join(".opencode/mcp.json"),
            "mcpServers",
            "opencode",
        ),
        Target {
            vscode: true,
            ..target(
                project_dir.join(".vscode/mcp.json"),
                "servers",
                GENERIC_AGENT_ID,
            )
        },
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
/// `None` when the client is not installed, when `t` is a location HQ no longer
/// writes, or when a remote form was asked for and `t` cannot take it. An
/// unparseable file is an error, never overwritten.
pub fn write_target<'a>(
    t: &'a Target,
    vault_path: &Path,
    opts: &EntryOptions,
) -> Result<Option<&'a Path>> {
    if t.legacy || (t.needs_parent && !t.path.parent().is_some_and(Path::exists)) {
        return Ok(None);
    }
    let server = match &opts.remote_url {
        Some(_) if !t.vscode => return Ok(None),
        Some(url) => build_remote_entry(url),
        None => build_stdio_entry(vault_path, t.agent_id, opts.scope, t.vscode),
    };
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
    let obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("config is not an object"))?;
    if t.vscode {
        // An older HQ wrote this under `mcpServers`, which VS Code ignores.
        if let Some(old) = obj.get_mut("mcpServers").and_then(Value::as_object_mut) {
            old.remove("agent-hq");
        }
        if opts.remote_url.is_some() {
            add_key_input(obj)?;
        }
    }
    obj.entry(key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("{key} is not an object"))?
        .insert("agent-hq".to_string(), server);
    write_json(&t.path, &root)?;
    Ok(Some(&t.path))
}

/// Make sure the root `inputs` array has the prompt for the remote key.
fn add_key_input(root: &mut serde_json::Map<String, Value>) -> Result<()> {
    let inputs = root
        .entry("inputs")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("inputs is not an array"))?;
    if !inputs.iter().any(|i| i["id"] == VSCODE_KEY_INPUT) {
        inputs.push(json!({
            "id": VSCODE_KEY_INPUT,
            "type": "promptString",
            "description": "Agent HQ MCP key",
            "password": true,
        }));
    }
    Ok(())
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
                let command_in = |key: &str| {
                    parsed
                        .get(key)
                        .and_then(|s| s.get("agent-hq"))
                        .and_then(|s| s.get("command"))
                };
                let has_url = ["servers", "mcpServers", "mcp"].iter().any(|key| {
                    parsed
                        .get(*key)
                        .and_then(|s| s.get("agent-hq"))
                        .and_then(|s| s.get("url"))
                        .is_some()
                });
                if has_url {
                    // A remote entry has no local binary to check.
                    return issues;
                }
                let has_cmd = command_in("servers")
                    .or_else(|| command_in("mcpServers"))
                    .or_else(|| command_in("mcp"))
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
            write_target(&t, Path::new("/vault"), &EntryOptions::default()).unwrap();
            let content = read_json(&t.path).unwrap();
            let agent_id = content["mcpServers"]["agent-hq"]["env"]["HQ_AGENT_ID"]
                .as_str()
                .or_else(|| content["servers"]["agent-hq"]["env"]["HQ_AGENT_ID"].as_str())
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

        write_target(&t, Path::new("/vault"), &EntryOptions::default()).unwrap();
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
        assert!(write_target(&t, Path::new("/vault"), &EntryOptions::default()).is_err());
        assert_eq!(std::fs::read_to_string(&t.path).unwrap(), "{ // comment\n}");
    }
    fn vscode_target(path: PathBuf) -> Target {
        Target {
            vscode: true,
            ..target(path, "servers", GENERIC_AGENT_ID)
        }
    }

    #[test]
    fn a_scoped_stdio_entry_launches_mcp_serve_with_that_scope() {
        let full = build_stdio_entry(Path::new("/vault"), "x", ServeScope::Full, false);
        assert_eq!(full["args"], json!(["mcp-serve"]), "full access adds no flag");
        let tasks = build_stdio_entry(Path::new("/vault"), "x", ServeScope::Tasks, false);
        assert_eq!(tasks["args"], json!(["mcp-serve", "--scope", "tasks"]));
        let ro = build_stdio_entry(Path::new("/vault"), "x", ServeScope::Readonly, false);
        assert_eq!(ro["args"], json!(["mcp-serve", "--scope", "readonly"]));
        assert!(tasks.get("type").is_none());
        let vs = build_stdio_entry(Path::new("/vault"), "x", ServeScope::Tasks, true);
        assert_eq!(vs["type"], "stdio");
    }

    #[test]
    fn serve_scopes_map_to_the_gateway_allowlists() {
        assert!(ServeScope::Full.allowlist().is_none());
        assert_eq!(ServeScope::Tasks.allowlist(), Some(hq_mcp::gateway::TASKS_ALLOWLIST));
        assert_eq!(
            ServeScope::Readonly.allowlist(),
            Some(hq_mcp::gateway::SPARK_READONLY_ALLOWLIST)
        );
    }

    /// VS Code's workspace file keys its servers under `servers`; `mcpServers` is ignored.
    #[test]
    fn the_project_vscode_file_uses_the_servers_key_and_drops_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let t = project_targets(dir.path())
            .into_iter()
            .find(|t| t.path.ends_with(".vscode/mcp.json"))
            .unwrap();
        write_json(
            &t.path,
            &json!({ "mcpServers": { "agent-hq": { "command": "old" }, "keep": { "command": "k" } } }),
        )
        .unwrap();
        write_target(&t, Path::new("/vault"), &EntryOptions::default()).unwrap();
        let root = read_json(&t.path).unwrap();
        assert_eq!(root["servers"]["agent-hq"]["type"], "stdio");
        assert!(root["mcpServers"].get("agent-hq").is_none(), "the ignored old entry is removed");
        assert_eq!(root["mcpServers"]["keep"]["command"], "k", "other servers are untouched");
    }

    #[test]
    fn a_remote_entry_asks_vscode_for_the_key_and_never_writes_it() {
        let dir = tempfile::tempdir().unwrap();
        let t = vscode_target(dir.path().join("mcp.json"));
        write_json(
            &t.path,
            &json!({ "inputs": [{ "id": "other", "type": "promptString" }], "servers": { "x": { "command": "y" } } }),
        )
        .unwrap();
        let opts = EntryOptions {
            remote_url: Some("https://hq.example.com/mcp".into()),
            ..EntryOptions::default()
        };
        write_target(&t, Path::new("/vault"), &opts).unwrap();
        write_target(&t, Path::new("/vault"), &opts).unwrap();
        let root = read_json(&t.path).unwrap();
        let entry = &root["servers"]["agent-hq"];
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["url"], "https://hq.example.com/mcp");
        assert_eq!(entry["headers"]["Authorization"], "Bearer ${input:agent-hq-key}");
        assert_eq!(root["servers"]["x"]["command"], "y");
        let inputs = root["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 2, "the existing input stays and ours is added once");
        let ours = inputs.iter().find(|i| i["id"] == "agent-hq-key").unwrap();
        assert_eq!(ours["password"], true);
        assert!(!std::fs::read_to_string(&t.path).unwrap().contains("Bearer h"));
    }

    #[test]
    fn a_remote_entry_is_only_written_to_vscode_files() {
        let dir = tempfile::tempdir().unwrap();
        let t = target(dir.path().join("mcp.json"), "mcpServers", "cursor");
        let opts = EntryOptions {
            remote_url: Some("https://hq.example.com/mcp".into()),
            ..EntryOptions::default()
        };
        assert!(write_target(&t, Path::new("/vault"), &opts).unwrap().is_none());
        assert!(!t.path.exists());
    }

    #[test]
    fn a_legacy_target_is_never_written_but_can_be_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let t = Target {
            legacy: true,
            ..target(dir.path().join("settings.json"), "mcp.servers", "claude-code")
        };
        write_json(
            &t.path,
            &json!({ "editor.fontSize": 14, "mcp.servers": { "agent-hq": { "command": "old" } } }),
        )
        .unwrap();
        assert!(write_target(&t, Path::new("/vault"), &EntryOptions::default()).unwrap().is_none());
        assert!(remove_target(&t).unwrap());
        let root = read_json(&t.path).unwrap();
        assert_eq!(root["editor.fontSize"], 14);
        assert!(root["mcp.servers"].get("agent-hq").is_none());
    }

    #[test]
    fn the_remote_snippet_is_a_complete_vscode_file() {
        let snippet: Value =
            serde_json::from_str(&remote_snippet("https://hq.example.com/mcp").unwrap()).unwrap();
        assert_eq!(snippet["servers"]["agent-hq"]["url"], "https://hq.example.com/mcp");
        assert_eq!(snippet["inputs"][0]["id"], "agent-hq-key");
    }

    #[test]
    fn remote_urls_need_tls_except_on_this_machine_and_carry_no_secrets() {
        for ok in [
            "https://hq.example.com/mcp",
            "https://hq.example.com:8443/mcp",
            "http://localhost:5678/mcp",
            "http://127.0.0.1:5679/mcp",
            "http://[::1]:5678/mcp",
        ] {
            assert!(validate_remote_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://hq.example.com/mcp",
            "http://localhost.evil.example/mcp",
            "https://user:pw@hq.example.com/mcp",
            "https://hq.example.com/mcp?key=abc",
            "https://hq.example.com/mcp#frag",
            "https://",
            "ftp://hq.example.com/mcp",
            "hq.example.com/mcp",
            "https://hq.example.com/mcp ",
            "https://hq .example.com/mcp",
        ] {
            assert!(validate_remote_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_checker_accepts_a_remote_entry_and_the_servers_key() {
        let dir = tempfile::tempdir().unwrap();
        let remote = dir.path().join("remote.json");
        write_json(
            &remote,
            &json!({ "servers": { "agent-hq": { "type": "http", "url": "https://hq.example.com/mcp" } } }),
        )
        .unwrap();
        assert!(check_mcp_file(&remote, "remote").is_empty());
        let local = dir.path().join("local.json");
        let bin = std::env::current_exe().unwrap();
        write_json(
            &local,
            &json!({ "servers": { "agent-hq": { "command": bin.to_string_lossy() } } }),
        )
        .unwrap();
        assert!(check_mcp_file(&local, "local").is_empty());
    }
}
