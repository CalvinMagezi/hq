//! File finding, content search, and directory listing.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::ToolResult;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::tools::AgentTool;

use super::{FIND_FILE_LIMIT, MAX_OUTPUT_BYTES, text_result, truncate_output};
use std::path::Path;
use std::time::Duration;

/// Glob-based file finder.
pub struct FindTool;

#[async_trait]
impl AgentTool for FindTool {
    fn name(&self) -> &str {
        "find_files"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Matches paths and filenames by glob. Use grep when you are looking for what is inside a file. Scope to a subdirectory when you know one.",
        )
    }

    fn description(&self) -> &str {
        concat!(
            "Finds files matching a glob pattern. Returns up to 200 results sorted by modification time.\n\n",
            "Examples: \"**/*.rs\", \"src/**/*.ts\", \"crates/*/Cargo.toml\"\n",
            "Use path parameter to scope the search to a subdirectory.\n\n",
            "Prefer this over bash find or ls for file discovery.",
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern (e.g., \"**/*.rs\", \"src/**/*.ts\")"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (defaults to current directory)"
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: pattern"))?;
        let base_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        debug!(pattern = %pattern, path = %base_path, "finding files");

        // Build full glob pattern
        let full_pattern = if pattern.starts_with('/') {
            pattern.to_string()
        } else {
            format!("{}/{}", base_path, pattern)
        };

        // Run glob in a blocking task since it does filesystem I/O
        let results = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            let mut files = Vec::new();
            for entry in glob::glob(&full_pattern)? {
                match entry {
                    Ok(path) => {
                        files.push(path.display().to_string());
                        if files.len() >= FIND_FILE_LIMIT {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "glob entry error");
                    }
                }
            }
            files.sort();
            Ok(files)
        })
        .await??;

        if results.is_empty() {
            Ok(text_result("No files found matching the pattern."))
        } else {
            let truncated = if results.len() >= FIND_FILE_LIMIT {
                format!("\n\n(results limited to {} files)", FIND_FILE_LIMIT)
            } else {
                String::new()
            };
            Ok(text_result(format!(
                "{} file(s) found:\n{}{}",
                results.len(),
                results.join("\n"),
                truncated
            )))
        }
    }
}

// ─── GrepTool ───────────────────────────────────────────────────

/// Search file contents using ripgrep (`rg`).
pub struct GrepTool;

#[async_trait]
impl AgentTool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Search content, not filenames — use find_files for names. Anchor with word boundaries or a path filter before widening; an unanchored search across the repo returns thousands of lines you then pay to read.",
        )
    }

    fn description(&self) -> &str {
        concat!(
            "Searches file contents using ripgrep. Supports full regex syntax.\n\n",
            "Parameters:\n",
            "- pattern: regex to match (e.g., \"fn handle_\\\\w+\", \"class.*implements\")\n",
            "- path: directory to search (default: current)\n",
            "- file_type: filter by language (\"rs\", \"ts\", \"py\", etc.)\n",
            "- context: lines of context before/after each match\n",
            "- case_insensitive: true for case-insensitive search\n\n",
            "Prefer this over bash grep or rg for content search. Times out after 30 seconds.",
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in (defaults to current directory)"
                },
                "file_type": {
                    "type": "string",
                    "description": "File type filter (e.g., \"rs\", \"ts\", \"py\")"
                },
                "context": {
                    "type": "integer",
                    "description": "Number of context lines to show around matches"
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Case insensitive search (default: false)"
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: pattern"))?;
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let file_type = args.get("file_type").and_then(|v| v.as_str());
        let context = args.get("context").and_then(|v| v.as_u64());
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        debug!(pattern = %pattern, path = %path, "running grep");

        let mut cmd = tokio::process::Command::new("rg");
        cmd.arg("--no-heading")
            .arg("--line-number")
            .arg("--color=never");

        if case_insensitive {
            cmd.arg("-i");
        }
        if let Some(ft) = file_type {
            cmd.arg("--type").arg(ft);
        }
        if let Some(ctx) = context {
            cmd.arg("-C").arg(ctx.to_string());
        }

        cmd.arg(pattern).arg(path);

        let result = tokio::time::timeout(Duration::from_secs(30), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.is_empty() {
                    Ok(text_result("No matches found."))
                } else {
                    Ok(text_result(truncate_output(&stdout, MAX_OUTPUT_BYTES)))
                }
            }
            Ok(Err(e)) => {
                // rg not found — provide a helpful message
                Ok(text_result(format!(
                    "Error running ripgrep: {}. Is `rg` installed?",
                    e
                )))
            }
            Err(_) => Ok(text_result("Grep timed out after 30 seconds.")),
        }
    }
}

// ─── LsTool ─────────────────────────────────────────────────────

/// List directory contents with file sizes.
pub struct LsTool;

#[async_trait]
impl AgentTool for LsTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "Lists directory contents with human-readable file sizes. \
         Use find_files with a glob pattern for recursive searches."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the directory to list"
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let dir_path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: path"))?;

        debug!(path = %dir_path, "listing directory");

        let path = Path::new(dir_path);
        if !path.exists() {
            return Ok(text_result(format!("Directory not found: {}", dir_path)));
        }
        if !path.is_dir() {
            return Ok(text_result(format!("Not a directory: {}", dir_path)));
        }

        let path_owned = path.to_path_buf();
        let entries = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            let mut items = Vec::new();
            for entry in std::fs::read_dir(&path_owned)? {
                let entry = entry?;
                let metadata = entry.metadata()?;
                let name = entry.file_name().to_string_lossy().to_string();

                if metadata.is_dir() {
                    items.push(format!("  {}/ (dir)", name));
                } else {
                    let size = metadata.len();
                    let size_str = format_size(size);
                    items.push(format!("  {} ({})", name, size_str));
                }
            }
            items.sort();
            Ok(items)
        })
        .await??;

        if entries.is_empty() {
            Ok(text_result(format!("{} (empty directory)", dir_path)))
        } else {
            Ok(text_result(format!(
                "{}:\n{}",
                dir_path,
                entries.join("\n")
            )))
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
