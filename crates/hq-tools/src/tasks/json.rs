//! Task JSON shapes, dependency and notification helpers, and argument parsing.

use anyhow::Result;
use hq_core::mailbox;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::path::PathBuf;

/// The one JSON shape for a task, shared by the MCP tools and `hq-web`.
pub fn task_json(task: &t::Task) -> Value {
    serde_json::to_value(task).unwrap_or(Value::Null)
}

/// `task_json` plus a `warnings` array when the task is being worked on or
/// finished while dependencies are still open. Dependencies are soft: the
/// write always goes through, the caller just gets told.
pub fn task_json_with_warnings(task: &t::Task) -> Value {
    let mut value = task_json(task);
    let started = task.status != t::STATUS_TO_DO && task.status != t::STATUS_BLOCKED;
    if started && !task.blocked_by.is_empty() {
        value["warnings"] = json!([format!(
            "{} is {} but still blocked by {}",
            task.display_id,
            task.status,
            task.blocked_by.join(", ")
        )]);
    }
    value
}

pub(super) fn task_summary(task: &t::Task) -> Value {
    json!({ "id": task.id, "display_id": task.display_id, "title": task.title, "status": task.status })
}

/// Applies requested dependency additions and removals to `task_id`.
pub fn apply_dependency_changes(
    conn: &rusqlite::Connection,
    task_id: &str,
    add: &[String],
    remove: &[String],
    created_by: &str,
) -> Result<()> {
    for blocker in remove {
        t::remove_dependency(conn, task_id, blocker)?;
    }
    for blocker in add {
        t::add_dependency(conn, task_id, blocker, created_by)?;
    }
    Ok(())
}

/// Tasks unblocked because `task` just moved into `complete`.
pub fn unblocked_by_transition(
    conn: &rusqlite::Connection,
    previous_status: Option<&str>,
    task: &t::Task,
) -> Result<Vec<t::Task>> {
    if task.status != t::STATUS_COMPLETE || previous_status == Some(t::STATUS_COMPLETE) {
        return Ok(Vec::new());
    }
    t::newly_unblocked(conn, &task.id)
}

/// Tags in `after` that `before` did not have: the only ones a write to an
/// existing task should notify, so an edit to a title or status stays quiet.
pub fn added_tags(before: &[String], after: &[String]) -> Vec<String> {
    after.iter().filter(|t| !before.contains(t)).cloned().collect()
}

/// Mails the agents behind `tags`, a subset of the task's own tags.
pub fn notify_tags(vault_path: &std::path::Path, task: &t::Task, tags: &[String]) {
    if tags.is_empty() {
        return;
    }
    let _ = mailbox::notify_tagged_agents(vault_path, &task.id, &task.display_id, &task.title, tags);
}

/// Tells each unblocked task's tagged agents that its last dependency is done.
pub fn notify_unblocked(vault_path: &std::path::Path, blocker: &t::Task, unblocked: &[t::Task]) {
    for task in unblocked.iter().filter(|t| !t.tags.is_empty()) {
        let content = format!(
            "Unblocked: {} ({} is complete)",
            task.title, blocker.display_id
        );
        let _ = mailbox::notify_tagged_agents_with(
            vault_path,
            &task.id,
            &task.display_id,
            &content,
            &task.tags,
        );
    }
}

/// `parent_id` wins over `top_level_only`; neither means no parent filter.
pub(super) fn parent_filter(args: &Value) -> Option<Option<String>> {
    if let Some(parent) = opt_str(args, "parent_id") {
        return Some(Some(parent));
    }
    args.get("top_level_only")
        .and_then(|v| v.as_bool())
        .filter(|b| *b)
        .map(|_| None)
}

pub(super) fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

pub(super) fn tags_from_args(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Best-effort: emits a `ValueItem` that surfaces on the web `/notifications`
/// page and its unread badge. Task updates stay in the web app: the value bus
/// never relays `task_ready_for_review` items to Telegram. Called by this
/// crate's `task_update` tool and `hq-web`'s REST update handler whenever a
/// task's status transitions into `ready_for_review`; the dedup key collapses
/// a repeat while one request is still outstanding. Never fails the caller.
pub async fn notify_ready_for_review(vault_path: PathBuf, task: t::Task) {
    let summary = format!("{}: {}", task.display_id, task.title);
    let item = hq_core::types::ValueItem::new(
        "task_ready_for_review",
        hq_core::types::ValueKind::ActionNeeded,
        summary,
        task.description.clone(),
    )
    .with_dedup_key(format!("task-review-{}", task.id));

    let db_path = vault_path.join("_data").join("vault.db");
    let emitted = hq_db::Database::open(&db_path)
        .and_then(|db| hq_db::value_items::emit(&db, &item).map(|_| ()));
    if let Err(e) = emitted {
        tracing::warn!(error = %e, task_id = %task.id, "task-notify: failed to emit ready-for-review value item");
    }
}
