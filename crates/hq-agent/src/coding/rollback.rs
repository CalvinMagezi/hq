//! Restoring files from the session's edit history.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::ToolResult;
use serde_json::{Value, json};
use tracing::debug;

use crate::tools::AgentTool;

use super::text_result;
use hq_tools::file_edit::{FileHistory, FileStateCache};
use std::path::Path;

/// Rollback a file to its state before the most recent edit.
pub struct RollbackTool {
    pub state_cache: FileStateCache,
    pub history: FileHistory,
}

impl RollbackTool {
    pub fn new(state_cache: FileStateCache, history: FileHistory) -> Self {
        Self {
            state_cache,
            history,
        }
    }
}

#[async_trait]
impl AgentTool for RollbackTool {
    fn name(&self) -> &str {
        "rollback_file"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Restores a file to its state before this session's edits. Reach for it as soon as an edit goes wrong rather than layering corrective edits on top of a broken file.",
        )
    }

    fn description(&self) -> &str {
        concat!(
            "Rollback a file to its state before the most recent edit or write. ",
            "Each call pops one snapshot from the history stack.\n\n",
            "Use this when an edit produced incorrect results and you want to undo it ",
            "without re-reading the entire file and manually reversing the change.",
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to rollback"
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;

        debug!(path = %file_path, "rolling back file");

        let path = Path::new(file_path);
        match self.history.rollback(path) {
            Some(content) => {
                tokio::fs::write(path, &content).await?;
                self.state_cache.record_write(path);
                Ok(text_result(format!(
                    "Rolled back {} ({} bytes restored)",
                    file_path,
                    content.len()
                )))
            }
            None => Ok(text_result(format!(
                "No rollback history available for {}. Only files modified via edit_file or write_file can be rolled back.",
                file_path
            ))),
        }
    }
}
