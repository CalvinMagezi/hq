//! Coding tools — bash, file I/O, search tools for agent sessions.

use hq_core::types::{ToolResult, ToolResultContent};

mod bash;
mod file_io;
mod rollback;
mod search;

pub use bash::BashTool;
pub use file_io::{EditTool, ReadTool, WriteTool};
pub use rollback::RollbackTool;
pub use search::{FindTool, GrepTool, LsTool};

// ─── Constants ──────────────────────────────────────────────────

const BASH_TIMEOUT_SECS: u64 = 120;
const MAX_OUTPUT_BYTES: usize = 50 * 1024; // 50 KB
const FIND_FILE_LIMIT: usize = 200;

// ─── Helpers ────────────────────────────────────────────────────

pub(crate) fn text_result(text: impl Into<String>) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            r#type: "text".to_string(),
            text: text.into(),
        }],
        details: None,
        context_modifier: None,
    }
}

const TRUNCATION_HINT: &str = ". Use offset/limit parameters or pipe to a file to see full output.";

fn truncate_output(output: &str, max_bytes: usize) -> String {
    hq_core::text::truncate_output(output, max_bytes, TRUNCATION_HINT)
}

#[cfg(test)]
mod tests;
