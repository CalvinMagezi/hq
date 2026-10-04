//! Custom terminal slash commands — user- or agent-authored `/command`s that
//! live in the vault so they survive restarts and sync across machines like
//! everything else under `.vault/`.
//!
//! A command is a markdown file at `<vault>/_commands/<name>.md` with a
//! `description` frontmatter field and a body that is a prompt template: the
//! literal token `{{args}}` is replaced with whatever the user typed after
//! the command name, and when the template has no such token the args are
//! appended instead, so a bare `/deploy` template still works standalone.
//!
//! This is the mechanism behind "hq, set up a slash command that does X" —
//! `SlashCommandManageTool` lets the agent write these files mid-conversation
//! and the terminal picks them up on the very next command, no restart.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::registry::HqTool;

/// The placeholder a command template substitutes with the user's arguments.
const ARGS_PLACEHOLDER: &str = "{{args}}";

/// A parsed custom command, ready to be rendered against a user's arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCommand {
    pub name: String,
    pub description: String,
    pub template: String,
}

/// Directory custom commands are read from and written to.
pub fn commands_dir(vault_path: &Path) -> PathBuf {
    vault_path.join("_commands")
}

/// A command name must be safe as a bare filename and as a `/name` token:
/// lowercase letters, digits, hyphens, and underscores only.
fn validate_command_name(name: &str) -> Result<()> {
    let name = name.strip_prefix('/').unwrap_or(name);
    if name.is_empty() {
        bail!("command name must not be empty");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        bail!(
            "invalid command name '{name}': use lowercase letters, digits, hyphens, and underscores only"
        );
    }
    Ok(())
}

fn command_path(vault_path: &Path, name: &str) -> PathBuf {
    let name = name.strip_prefix('/').unwrap_or(name);
    commands_dir(vault_path).join(format!("{name}.md"))
}

/// Parse a single command file. `name` is taken from the filename, not from
/// frontmatter, so a renamed file can never disagree with its own lookup key.
fn parse_command_file(path: &Path, name: &str) -> Option<CustomCommand> {
    let raw = std::fs::read_to_string(path).ok()?;
    let matter = gray_matter::Matter::<gray_matter::engine::YAML>::new();
    let result = matter.parse(&raw);

    let description = result
        .data
        .as_ref()
        .and_then(|d| match d {
            gray_matter::Pod::Hash(map) => map.get("description").and_then(|v| match v {
                gray_matter::Pod::String(s) => Some(s.clone()),
                _ => None,
            }),
            _ => None,
        })
        .unwrap_or_else(|| format!("Custom command: /{name}"));

    Some(CustomCommand {
        name: name.to_string(),
        description,
        template: result.content.trim().to_string(),
    })
}

/// Load every custom command defined in the vault. Missing directory is not
/// an error — it just means no custom commands have been created yet.
pub fn load_custom_commands(vault_path: &Path) -> Vec<CustomCommand> {
    let dir = commands_dir(vault_path);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut commands: Vec<CustomCommand> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
        .filter_map(|e| {
            let stem = e.path().file_stem()?.to_str()?.to_string();
            parse_command_file(&e.path(), &stem)
        })
        .collect();
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    commands
}

/// Look up one custom command by name (with or without a leading `/`).
pub fn find_custom_command(vault_path: &Path, name: &str) -> Option<CustomCommand> {
    let name = name.strip_prefix('/').unwrap_or(name);
    let path = command_path(vault_path, name);
    parse_command_file(&path, name)
}

/// Render a command's template against the user's typed arguments.
pub fn render_custom_command(cmd: &CustomCommand, args: &str) -> String {
    if cmd.template.contains(ARGS_PLACEHOLDER) {
        return cmd.template.replace(ARGS_PLACEHOLDER, args.trim());
    }
    if args.trim().is_empty() {
        cmd.template.clone()
    } else {
        format!("{}\n\n{}", cmd.template, args.trim())
    }
}

/// Write (or overwrite) a custom command definition.
pub fn write_custom_command(
    vault_path: &Path,
    name: &str,
    description: &str,
    template: &str,
) -> Result<PathBuf> {
    validate_command_name(name)?;
    if template.trim().is_empty() {
        bail!("template must not be empty");
    }
    let name = name.strip_prefix('/').unwrap_or(name);
    let dir = commands_dir(vault_path);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.md"));
    let description = if description.trim().is_empty() {
        format!("Custom command: /{name}")
    } else {
        description.trim().to_string()
    };
    let body = format!(
        "---\ndescription: \"{}\"\n---\n{}\n",
        description.replace('"', "'"),
        template.trim()
    );
    std::fs::write(&path, body)?;
    Ok(path)
}

/// Delete a custom command definition. Errors if it does not exist.
pub fn delete_custom_command(vault_path: &Path, name: &str) -> Result<()> {
    validate_command_name(name)?;
    let path = command_path(vault_path, name);
    if !path.exists() {
        bail!(
            "no such command: /{}",
            name.strip_prefix('/').unwrap_or(name)
        );
    }
    std::fs::remove_file(&path)?;
    Ok(())
}

/// Agent-callable tool: create, update, list, or delete custom terminal
/// slash commands. This is what turns "set up a `/deploy` command for me"
/// into an actual file the terminal will recognize on the next prompt.
pub struct SlashCommandManageTool {
    vault_path: PathBuf,
}

impl SlashCommandManageTool {
    pub fn new(vault_path: PathBuf) -> Self {
        Self { vault_path }
    }
}

#[async_trait]
impl HqTool for SlashCommandManageTool {
    fn name(&self) -> &str {
        "slash_command_manage"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Use when the user asks you to create, change, or remove a custom `/command` for \
             the hq terminal. The command becomes available immediately in `hq chat` — no \
             restart needed. Write the template as the exact prompt you'd want run; use \
             {{args}} where the user's typed arguments should be inserted, or omit it to have \
             them appended to the end automatically.",
        )
    }

    fn description(&self) -> &str {
        "Create, update, list, or delete custom terminal slash commands stored in the vault \
         under _commands/. Each command is a prompt template: {{args}} is replaced with \
         whatever the user types after the command name."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "delete"],
                    "description": "Operation to perform. 'create' also updates an existing command."
                },
                "name": {
                    "type": "string",
                    "description": "Command name without the leading slash, e.g. 'deploy' for /deploy. Lowercase letters, digits, hyphens, underscores only."
                },
                "description": {
                    "type": "string",
                    "description": "One-line description shown in /help (required for create)"
                },
                "template": {
                    "type": "string",
                    "description": "The prompt template to run when the command is invoked. Use {{args}} to place the user's arguments; if omitted, arguments are appended (required for create)"
                }
            },
            "required": ["action"]
        })
    }

    fn category(&self) -> &str {
        "terminal"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        match action {
            "create" => {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("name is required"))?;
                let description = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let template = args
                    .get("template")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("template is required"))?;
                let path = write_custom_command(&self.vault_path, name, description, template)?;
                Ok(json!({
                    "status": "created",
                    "path": path.to_string_lossy(),
                    "invoke_as": format!("/{}", name.strip_prefix('/').unwrap_or(name)),
                }))
            }
            "list" => {
                let commands = load_custom_commands(&self.vault_path);
                Ok(json!({
                    "commands": commands.iter().map(|c| json!({
                        "name": format!("/{}", c.name),
                        "description": c.description,
                    })).collect::<Vec<_>>()
                }))
            }
            "delete" => {
                let name = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("name is required"))?;
                delete_custom_command(&self.vault_path, name)?;
                Ok(json!({"status": "deleted"}))
            }
            _ => bail!("invalid action: {}", action),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_loads_a_command() {
        let tmp = tempfile::tempdir().unwrap();
        write_custom_command(tmp.path(), "deploy", "Ship it", "Deploy {{args}} to prod").unwrap();

        let cmd = find_custom_command(tmp.path(), "/deploy").unwrap();
        assert_eq!(cmd.name, "deploy");
        assert_eq!(cmd.description, "Ship it");
        assert_eq!(
            render_custom_command(&cmd, "staging"),
            "Deploy staging to prod"
        );
    }

    #[test]
    fn appends_args_when_no_placeholder() {
        let tmp = tempfile::tempdir().unwrap();
        write_custom_command(
            tmp.path(),
            "summarize",
            "Summarize",
            "Summarize the discussion.",
        )
        .unwrap();
        let cmd = find_custom_command(tmp.path(), "summarize").unwrap();
        assert_eq!(
            render_custom_command(&cmd, "focus on risks"),
            "Summarize the discussion.\n\nfocus on risks"
        );
        assert_eq!(render_custom_command(&cmd, ""), "Summarize the discussion.");
    }

    #[test]
    fn rejects_invalid_names() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(write_custom_command(tmp.path(), "Deploy!", "x", "y").is_err());
        assert!(write_custom_command(tmp.path(), "has/slash", "x", "y").is_err());
    }

    #[test]
    fn lists_and_deletes() {
        let tmp = tempfile::tempdir().unwrap();
        write_custom_command(tmp.path(), "one", "First", "do one").unwrap();
        write_custom_command(tmp.path(), "two", "Second", "do two").unwrap();

        let all = load_custom_commands(tmp.path());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "one");

        delete_custom_command(tmp.path(), "/one").unwrap();
        let remaining = load_custom_commands(tmp.path());
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "two");
    }

    #[tokio::test]
    async fn tool_create_list_delete_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = SlashCommandManageTool::new(tmp.path().to_path_buf());

        let created = tool
            .execute(json!({
                "action": "create",
                "name": "codereview2",
                "description": "Second opinion",
                "template": "Review {{args}}"
            }))
            .await
            .unwrap();
        assert_eq!(created["status"], "created");

        let listed = tool.execute(json!({"action": "list"})).await.unwrap();
        assert_eq!(listed["commands"].as_array().unwrap().len(), 1);

        let deleted = tool
            .execute(json!({"action": "delete", "name": "codereview2"}))
            .await
            .unwrap();
        assert_eq!(deleted["status"], "deleted");
    }
}
