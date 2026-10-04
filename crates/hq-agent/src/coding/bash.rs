//! Shell command execution.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::ToolResult;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::tools::AgentTool;

use super::{BASH_TIMEOUT_SECS, MAX_OUTPUT_BYTES, text_result, truncate_output};
use crate::bash_policy::sanitized_env;
use crate::bash_sandbox::{BashSettings, Launch, plan_launch};
use std::sync::Arc;
use std::time::Duration;

/// Execute a shell command via `bash -c`, with an allowlisted environment
/// and, when available, inside the OS sandbox from `bash_sandbox`.
#[derive(Debug, Clone)]
pub struct BashTool {
    settings: Arc<BashSettings>,
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new(BashSettings::default())
    }
}

impl BashTool {
    pub fn new(settings: BashSettings) -> Self {
        crate::bash_sandbox::hide_process_environment();
        crate::bash_sandbox::warn_if_unprotected_once(&settings);
        Self {
            settings: Arc::new(settings),
        }
    }

    /// The process to spawn for `command`, or the refusal text when the
    /// configured sandbox is required but missing.
    fn build_process(&self, command: &str) -> std::result::Result<tokio::process::Command, String> {
        let mut process = match plan_launch(&self.settings, command) {
            Launch::Refused(reason) => return Err(reason),
            Launch::Direct => {
                let mut p = tokio::process::Command::new("bash");
                p.arg("-c").arg(command);
                p
            }
            Launch::Wrapped { program, args } => {
                let mut p = tokio::process::Command::new(program);
                p.args(args);
                p
            }
        };
        process
            .env_clear()
            .envs(sanitized_env(&self.settings.env_passthrough))
            .kill_on_drop(true);
        Ok(process)
    }
}

#[async_trait]
impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Prefer read_file, edit_file, grep, and find_files over cat/sed/grep/find — they are cheaper and give line numbers. The working directory does NOT persist between calls, so chain with && rather than a separate cd. Quote paths containing spaces.",
        )
    }

    fn description(&self) -> &str {
        concat!(
            "Executes a bash command and returns its output. ",
            "The working directory persists between commands but shell state does not.\n\n",
            "IMPORTANT: Avoid using this tool when a dedicated tool exists:\n",
            "- File search: use find_files (NOT bash find or ls)\n",
            "- Content search: use grep (NOT bash grep or rg)\n",
            "- Read files: use read_file (NOT bash cat/head/tail)\n",
            "- Edit files: use edit_file (NOT bash sed/awk)\n\n",
            "Instructions:\n",
            "- Always use absolute paths. Quote paths that contain spaces.\n",
            "- For independent commands, make parallel tool calls. For dependent commands, chain with &&.\n",
            "- Do not use newlines to separate commands -- use && or ;.\n",
            "- Do not use sleep loops. If a command is long-running, use run_in_background.\n",
            "- If creating directories or files, verify the parent path exists first.\n\n",
            "Git safety:\n",
            "- Prefer new commits over amending existing ones.\n",
            "- Never skip hooks (--no-verify) unless explicitly asked.\n",
            "- Before destructive ops (reset --hard, push --force, branch -D), confirm with user.\n",
            "- Never force push to main/master.\n\n",
            "Commands time out after 120 seconds. Max output: 50KB.",
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": ["integer", "string"],
                    "description": "Optional timeout in seconds (default 120)"
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: command"))?;

        let timeout_secs = args
            .get("timeout")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(BASH_TIMEOUT_SECS);

        if let Err(e) = crate::bash_policy::validate_command(command) {
            warn!(command = %hq_core::redact::loggable_command(command), error = %e, "bash command blocked by policy");
            return Ok(text_result(format!("Command rejected: {}", e)));
        }

        let mut process = match self.build_process(command) {
            Ok(p) => p,
            Err(reason) => {
                warn!(command = %hq_core::redact::loggable_command(command), "bash command refused: sandbox required but unavailable");
                return Ok(text_result(reason));
            }
        };
        debug!(command = %hq_core::redact::loggable_command(command), timeout = timeout_secs, "executing bash command");

        let result =
            tokio::time::timeout(Duration::from_secs(timeout_secs), process.output()).await;
        match result {
            Ok(Ok(output)) => Ok(format_output(&output, command)),
            Ok(Err(e)) => Ok(text_result(format!("Error executing command: {}", e))),
            Err(_) => Ok(text_result(format!(
                "Command timed out after {}s",
                timeout_secs
            ))),
        }
    }
}

/// Merge stdout, stderr and a non-zero exit code into one tool result.
fn format_output(output: &std::process::Output, command: &str) -> ToolResult {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);

    let mut text = String::new();
    if !stdout.is_empty() {
        text.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("STDERR:\n");
        text.push_str(&stderr);
    }
    if text.is_empty() {
        text = format!("(no output, exit code {})", exit_code);
    } else if exit_code != 0 {
        text.push_str(&format!("\n(exit code {})", exit_code));
    }

    let mut result = text_result(truncate_output(&text, MAX_OUTPUT_BYTES));

    // A context modifier lets the session loop inject a compact annotation
    // instead of a full message when the command may have moved the cwd.
    if command.trim().starts_with("cd ") || command.contains("&& cd ") || command.contains("; cd ")
    {
        result.context_modifier = Some(format!(
            "Working directory may have changed. Command was: {}",
            command
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cr3tValueThatMustNeverBeLogged";

    #[tokio::test]
    async fn logs_never_carry_command_arguments_that_hold_secrets() {
        let (logs, _guard) = hq_core::test_util::capture_logs();
        let tool = BashTool::default();
        let padding = "x".repeat(80);
        // Secrets at the very start of the arguments, in several shapes.
        let allowed = format!(
            "echo -u admin:{SECRET} -H 'X-Api-Key: {SECRET}' https://u:{SECRET}@h/ {padding}"
        );
        let blocked = format!("rm -rf / -p{SECRET} {padding}");

        tool.execute("t1", json!({"command": allowed}))
            .await
            .unwrap();
        tool.execute("t2", json!({"command": blocked}))
            .await
            .unwrap();
        tool.execute(
            "t3",
            json!({"command": "PGPASSWORD=hunter2hunter2 MYSQL_PWD=zzTopSecret99 true"}),
        )
        .await
        .unwrap();

        let out = logs.contents();
        assert!(out.contains("executing bash command"), "{out}");
        assert!(out.contains("bash command blocked by policy"), "{out}");
        assert!(
            out.contains("echo (len "),
            "program name and length are logged: {out}"
        );
        assert!(
            out.contains("true (len "),
            "env assignments are skipped for the program name: {out}"
        );
        assert!(!out.contains(SECRET), "a secret reached the log: {out}");
        assert!(
            !out.contains("hunter2hunter2") && !out.contains("zzTopSecret99"),
            "{out}"
        );
    }
}
