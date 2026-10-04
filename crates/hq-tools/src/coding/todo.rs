//! Session-scoped todo/task tracking — a lightweight in-memory equivalent of
//! Claude Code's TodoWrite tool.
//!
//! Distinct from `hq plan`'s vault-persisted, multi-model, adversarially-reviewed
//! plan: that's coarser-grained (per-phase, survives across sessions/terminal
//! restarts) while this is per-step, in-memory only, and meant to track live
//! in-turn progress within a single running session.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use crate::registry::HqTool;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

/// Shared, cloneable handle to one session's todo list. Clone it (cheap —
/// wraps an `Arc`) into every tool/adapter that needs to read or replace the
/// list, mirroring `FileStateCache`/`FileHistory`'s sharing pattern.
#[derive(Clone, Default)]
pub struct TodoStore {
    inner: Arc<Mutex<Vec<TodoItem>>>,
}

impl TodoStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current todo list, most-recently-set order.
    pub fn snapshot(&self) -> Vec<TodoItem> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, items: Vec<TodoItem>) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *guard = items;
    }

    fn render(items: &[TodoItem]) -> String {
        if items.is_empty() {
            return "(no todos)".to_string();
        }
        items
            .iter()
            .map(|t| {
                let mark = match t.status {
                    TodoStatus::Completed => "[x]",
                    TodoStatus::InProgress => "[~]",
                    TodoStatus::Pending => "[ ]",
                };
                format!("{mark} {}", t.content)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub struct TodoWriteTool {
    store: TodoStore,
}

impl TodoWriteTool {
    pub fn new(store: TodoStore) -> Self {
        Self { store }
    }
}

#[async_trait]
impl HqTool for TodoWriteTool {
    fn name(&self) -> &str {
        "todo_write"
    }

    fn description(&self) -> &str {
        "Replace the current task's todo list. Pass the full list every call (not a diff) so \
         it always reflects true current state. Use for multi-step work where tracking what's \
         done, in progress, and pending helps you stay on track. Mark exactly one item \
         in_progress at a time; mark an item completed the moment it's actually done, not before."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["todos"],
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The full, current todo list — replaces whatever was there before",
                    "items": {
                        "type": "object",
                        "required": ["content", "status"],
                        "properties": {
                            "content": {
                                "type": "string",
                                "description": "One task, imperative form (e.g. 'Add the missing test case')"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            }
                        }
                    }
                }
            }
        })
    }

    fn category(&self) -> &str {
        "coding"
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let todos: Vec<TodoItem> =
            serde_json::from_value(args.get("todos").cloned().unwrap_or(Value::Array(vec![])))
                .map_err(|e| anyhow::anyhow!("invalid todos: {e}"))?;

        let in_progress_count = todos
            .iter()
            .filter(|t| t.status == TodoStatus::InProgress)
            .count();
        if in_progress_count > 1 {
            return Ok(json!({
                "error": format!(
                    "{in_progress_count} items marked in_progress — mark exactly one at a time \
                     so it's clear what you're actually working on right now"
                )
            }));
        }

        let rendered = TodoStore::render(&todos);
        let count = todos.len();
        self.store.set(todos);

        Ok(json!({
            "success": true,
            "count": count,
            "in_progress_count": in_progress_count,
            "rendered": rendered,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_replaces_the_whole_list_and_renders_it() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store.clone());

        let result = tool
            .execute(json!({
                "todos": [
                    {"content": "Write the design doc", "status": "completed"},
                    {"content": "Implement the change", "status": "in_progress"},
                    {"content": "Add tests", "status": "pending"},
                ]
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["count"], 3);
        assert_eq!(result["in_progress_count"], 1);
        assert_eq!(store.snapshot().len(), 3);
        assert!(
            result["rendered"]
                .as_str()
                .unwrap()
                .contains("[~] Implement the change")
        );
    }

    #[tokio::test]
    async fn write_rejects_more_than_one_in_progress_item() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store.clone());

        let result = tool
            .execute(json!({
                "todos": [
                    {"content": "A", "status": "in_progress"},
                    {"content": "B", "status": "in_progress"},
                ]
            }))
            .await
            .unwrap();

        assert!(result["error"].is_string());
        // The store must not have been mutated on rejection.
        assert!(store.snapshot().is_empty());
    }

    #[tokio::test]
    async fn a_second_write_fully_replaces_the_first() {
        let store = TodoStore::new();
        let tool = TodoWriteTool::new(store.clone());

        tool.execute(json!({"todos": [{"content": "A", "status": "pending"}]}))
            .await
            .unwrap();
        tool.execute(json!({"todos": [{"content": "B", "status": "completed"}]}))
            .await
            .unwrap();

        let snapshot = store.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].content, "B");
    }

    #[test]
    fn todo_write_tool_is_not_read_only() {
        let tool = TodoWriteTool::new(TodoStore::new());
        assert!(!tool.is_read_only());
    }
}
