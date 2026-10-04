//! File reads, writes, edits, and truncated tool output reads.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::ToolResult;
use serde_json::{Value, json};
use tracing::debug;

use crate::tools::AgentTool;

use super::text_result;
use hq_tools::file_edit::{FileHistory, FileStateCache};
use std::path::Path;

/// Read a file with line numbers (cat -n style).
pub struct ReadTool {
    pub state_cache: FileStateCache,
}

impl ReadTool {
    pub fn new(state_cache: FileStateCache) -> Self {
        Self { state_cache }
    }
}

#[async_trait]
impl AgentTool for ReadTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "For any source file over 300 lines, grep for what you need first and then read only those lines. Reading a large file whole to \"get oriented\" is the single most common way sessions run out of context.",
        )
    }

    fn description(&self) -> &str {
        concat!(
            "Reads a file from the local filesystem with line numbers (cat -n format).\n\n",
            "Usage:\n",
            "- file_path must be an absolute path, not a relative path.\n",
            "- By default reads up to 2000 lines from the beginning.\n",
            "- Use offset + limit to read specific sections of large files. This saves context.\n",
            "- Cannot read directories -- use list_dir for that.\n",
            "- You must read a file before calling edit_file on it.",
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-based)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (default: 2000)"
                }
            }
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;

        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;

        debug!(path = %file_path, offset = offset, limit = limit, "reading file");

        let path = Path::new(file_path);
        if !path.exists() {
            return Ok(text_result(format!("File not found: {}", file_path)));
        }

        let content = tokio::fs::read_to_string(path).await?;
        // Populates the same FileStateCache instance edit_file's staleness
        // check reads from (SessionBuilder::build shares one across both
        // tools) — without this, every edit_file call sees this file as
        // never-read.
        self.state_cache.record_read(path);
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        // offset is 1-based
        let start = if offset > 0 { offset - 1 } else { 0 };
        let end = (start + limit).min(total_lines);

        if start >= total_lines {
            return Ok(text_result(format!(
                "Offset {} exceeds file length ({} lines)",
                offset, total_lines
            )));
        }

        let mut output = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;
            // cat -n format: right-aligned line numbers with tab
            output.push_str(&format!("{:>6}\t{}\n", line_num, line));
        }

        if end < total_lines {
            output.push_str(&format!(
                "\n... ({} more lines, {} total)",
                total_lines - end,
                total_lines
            ));
        }

        Ok(text_result(output))
    }
}

// ─── WriteTool ──────────────────────────────────────────────────

/// Write content to a file, creating parent directories as needed.
pub struct WriteTool {
    pub state_cache: FileStateCache,
    pub history: FileHistory,
}

impl WriteTool {
    pub fn new(state_cache: FileStateCache, history: FileHistory) -> Self {
        Self {
            state_cache,
            history,
        }
    }
}

#[async_trait]
impl AgentTool for WriteTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Only for new files or a full rewrite you intend. Use edit_file for changes to an existing file — write_file silently discards anything you have not read. Read the file first if it already exists.",
        )
    }

    fn description(&self) -> &str {
        concat!(
            "Writes content to a file. Creates parent directories if needed. ",
            "Overwrites existing files completely.\n\n",
            "Use this only to create new files or completely rewrite a file. ",
            "For targeted changes to existing files, use edit_file instead.",
        )
    }

    fn is_destructive(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "content"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: content"))?;

        debug!(path = %file_path, bytes = content.len(), "writing file");

        let path = Path::new(file_path);

        // Snapshot existing file before overwrite (for rollback).
        if path.exists() {
            self.history.snapshot(path);
        }

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(path, content).await?;
        self.state_cache.record_write(path);
        Ok(text_result(format!(
            "Successfully wrote {} bytes to {}",
            content.len(),
            file_path
        )))
    }
}

// ─── EditTool ───────────────────────────────────────────────────

/// Exact string replacement in a file.
pub struct EditTool {
    pub state_cache: FileStateCache,
    pub history: FileHistory,
}

impl EditTool {
    pub fn new(state_cache: FileStateCache, history: FileHistory) -> Self {
        Self {
            state_cache,
            history,
        }
    }
}

#[async_trait]
impl AgentTool for EditTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "old_string must match the file byte for byte including indentation, and must be unique or the edit fails. Read the file first. Prefer several small edits over one sweeping replacement.",
        )
    }

    fn is_destructive(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        concat!(
            "Performs exact string replacement in a file.\n\n",
            "Usage:\n",
            "- You MUST call read_file at least once before editing. ",
            "This tool will error if the file hasn't been read.\n",
            "- Preserve exact indentation. Copy the content exactly as it appears after ",
            "the line number prefix in read_file output. Never include line numbers in ",
            "old_string or new_string.\n",
            "- old_string must be unique in the file. If it appears multiple times, add ",
            "more surrounding context to make it unique, or use replace_all: true.\n",
            "- Use minimal context in old_string -- typically 2-4 adjacent lines is enough ",
            "to uniquely identify the target. Avoid copying 10+ lines of context.\n",
            "- Use replace_all for renaming variables or symbols that appear throughout the file.\n",
            "- old_string and new_string must differ. No-op edits will error.\n",
            "- ALWAYS prefer editing existing files over creating new ones.",
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file_path", "old_string", "new_string"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)"
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: file_path"))?;
        let old_string = args
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: old_string"))?;
        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: new_string"))?;
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        debug!(path = %file_path, replace_all = replace_all, "editing file");

        let path = Path::new(file_path);
        if !path.exists() {
            return Ok(text_result(format!("File not found: {}", file_path)));
        }

        // Shared validation in hq_tools::file_edit: staleness
        // detection (must read_file before editing, and re-read if the file
        // changed on disk since), quote normalization (curly vs straight
        // quotes), and a no-op guard — this tool's own inline checks below
        // only covered "not found" and "found N times", not those three.
        let validation = hq_tools::file_edit::validate_file_edit(
            path,
            old_string,
            new_string,
            replace_all,
            &self.state_cache,
        );
        if let hq_core::types::ValidationResult::Err { message, .. } = validation {
            return Ok(text_result(format!("Error: {message}")));
        }

        let content = tokio::fs::read_to_string(path).await?;
        // The string actually present in the file, which may differ from
        // old_string only by quote style — apply_edit and the occurrence
        // count must operate on what's really there, or a quote-normalized
        // match reports "not found" here despite validate_file_edit having
        // just accepted it.
        let resolved_old =
            hq_tools::file_edit::find_actual_string(&content, old_string).unwrap_or(old_string);
        let occurrences = content.matches(resolved_old).count();
        let new_content =
            hq_tools::file_edit::apply_edit(&content, resolved_old, new_string, replace_all);

        // Snapshot before write (for rollback).
        self.history.snapshot(path);

        tokio::fs::write(path, &new_content).await?;
        self.state_cache.record_write(path);
        Ok(text_result(format!(
            "Successfully replaced {} occurrence(s) in {}",
            if replace_all { occurrences } else { 1 },
            file_path
        )))
    }
}
