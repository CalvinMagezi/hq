//! Simplified "easy action" tool shortcuts.
//!
//! Each shortcut wraps one or more complex tools behind a natural-language
//! or single-parameter interface. All shortcuts carry `ToolPolicy::Weak`
//! so they appear even in weak sessions (relay bots, local models).

pub mod gateway;
pub mod vault;

use crate::registry::HqTool;
use hq_db::Database;
use std::path::PathBuf;
use std::sync::Arc;

/// Create all shortcut tools.
///
/// `vault_path` — path to the vault directory.
/// `db` — open database handle.
pub fn create_shortcut_tools(
    vault_path: PathBuf,
    db: Arc<Database>,
) -> Vec<Box<dyn HqTool>> {
    vec![
        // Vault shortcuts
        Box::new(vault::VaultFindShortcut::new(
            vault_path.clone(),
            db.clone(),
        )),
        Box::new(vault::VaultNoteShortcut::new(vault_path.clone())),
        Box::new(vault::VaultLogShortcut::new(vault_path.clone())),
        // Fuzzy gateway (resolves any tool by name)
        Box::new(gateway::HqGatewayTool::new(db, vault_path)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_shortcut_tools_returns_four_tools() {
        let tmp = std::path::PathBuf::from("/tmp");
        let db = Arc::new(hq_db::Database::open_memory().expect("in-memory db"));
        let tools = create_shortcut_tools(tmp, db);
        assert_eq!(tools.len(), 4, "expected 4 shortcut tools");
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"vault_find"));
        assert!(names.contains(&"vault_note"));
        assert!(names.contains(&"vault_log"));
        assert!(names.contains(&"hq"));
    }

    #[test]
    fn all_shortcuts_have_weak_policy() {
        let tmp = std::path::PathBuf::from("/tmp");
        let db = Arc::new(hq_db::Database::open_memory().expect("in-memory db"));
        let tools = create_shortcut_tools(tmp, db);
        for tool in &tools {
            assert_eq!(
                tool.tool_policy(),
                crate::registry::ToolPolicy::Weak,
                "tool '{}' should have Weak policy",
                tool.name()
            );
        }
    }
}
