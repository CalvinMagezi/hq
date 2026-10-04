//! Coding tools in the MCP registry: batch edit, directory listing, todos, git,
//! session listing and config. External clients bring their own file and
//! shell tools; HQ's own session registers its native ones in hq-agent.

pub mod config;
pub mod edit;
pub mod file_ops;
pub mod git;
pub mod session;
pub mod todo;

pub use config::ConfigTool;
pub use edit::BatchEditTool;
pub use file_ops::ListDirTool;
pub use git::{GitCommitTool, GitDiffTool, GitLogTool, GitPrTool, GitStatusTool};
pub use session::SessionListTool;
pub use todo::{TodoItem, TodoStatus, TodoStore, TodoWriteTool};

use crate::file_edit::{FileHistory, FileStateCache};
use crate::registry::HqTool;

pub(super) const MAX_OUTPUT_BYTES: usize = 50 * 1024;

pub(super) fn truncate_output(output: &str, max_bytes: usize) -> String {
    hq_core::text::truncate_output(output, max_bytes, "")
}

/// Create all coding tools for MCP registration.
pub fn create_coding_tools() -> Vec<Box<dyn HqTool>> {
    let state_cache = FileStateCache::new();
    let history = FileHistory::new(50, 10 * 1024 * 1024); // 50 files, 10MB total

    vec![
        Box::new(BatchEditTool::new(state_cache, history)),
        Box::new(ListDirTool),
        Box::new(TodoWriteTool::new(TodoStore::new())),
        // Git operations
        Box::new(GitCommitTool),
        Box::new(GitDiffTool),
        Box::new(GitStatusTool),
        Box::new(GitLogTool),
        Box::new(GitPrTool),
        Box::new(SessionListTool),
        Box::new(ConfigTool),
    ]
}

#[cfg(test)]
mod read_only_tests {
    use super::*;
    use crate::file_edit::{FileHistory, FileStateCache};

    #[test]
    fn read_tools_are_marked_read_only() {
        assert!(GitStatusTool.is_read_only());
        assert!(GitDiffTool.is_read_only());
        assert!(GitLogTool.is_read_only());
        assert!(ListDirTool.is_read_only());
    }

    #[test]
    fn write_tools_are_not_read_only() {
        let state_cache = FileStateCache::new();
        let history = FileHistory::new(10, 1024 * 1024);
        assert!(!BatchEditTool::new(state_cache, history).is_read_only());
        assert!(!GitCommitTool.is_read_only());
    }
}
