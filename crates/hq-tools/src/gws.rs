//! Google Workspace tool — shells out to the `gws` CLI.

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;

use crate::registry::HqTool;

/// Executes Google Workspace operations via the `gws` CLI binary.
pub struct GoogleWorkspaceTool;

impl GoogleWorkspaceTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GoogleWorkspaceTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HqTool for GoogleWorkspaceTool {
    fn name(&self) -> &str {
        "google_workspace"
    }

    fn description(&self) -> &str {
        "Execute any Google Workspace operation (Drive, Sheets, Gmail, Calendar, Docs, Slides, Tasks, YouTube) via the gws CLI. Supports read and write for every enabled API, e.g. 'docs documents get --params {\"documentId\": \"...\"}' to read a Google Doc."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Full gws command arguments (e.g. 'drive files list --params {\"q\": \"...\"}')"
                }
            },
            "required": ["command"]
        })
    }

    fn category(&self) -> &str {
        "workspace"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        {
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            if command.is_empty() {
                bail!("command is required");
            }

            // Safety: reject obviously dangerous shell metacharacters
            if command.contains('`') || command.contains("$(") {
                bail!("shell injection not allowed in gws command");
            }

            let output = tokio::process::Command::new(hq_core::paths::resolve_gws_binary())
                .args(shell_words(command))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if output.status.success() {
                // Try to parse stdout as JSON; fall back to raw text
                let result = serde_json::from_str::<Value>(&stdout)
                    .unwrap_or_else(|_| json!({ "output": stdout.trim() }));
                Ok(result)
            } else {
                Ok(json!({
                    "error": true,
                    "exit_code": output.status.code(),
                    "stderr": stderr.trim(),
                    "stdout": stdout.trim(),
                }))
            }
        }
    }
}

/// Simple word-splitting for the command string. Respects single and double quotes.
fn shell_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;

    for ch in input.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escape = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            ' ' | '\t' if !in_single && !in_double => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}
