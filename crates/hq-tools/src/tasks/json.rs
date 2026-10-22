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
    after
        .iter()
        .filter(|t| !before.contains(t))
        .cloned()
        .collect()
}

/// Mails whoever a write newly gave a task to. Assignees are always routed to; tags are too
/// while `route_tags` is on (the default), so routing by tag keeps working until it is
/// replaced by assignees. `previous` is the task before the write, `None` for a new task,
/// and only what is new since then is mailed. A name in both lists is mailed once.
pub fn notify_recipients(
    vault_path: &std::path::Path,
    task: &t::Task,
    previous: Option<&t::Task>,
    route_tags: bool,
) {
    let (old_tags, old_assignees): (&[String], &[String]) = match previous {
        Some(p) => (&p.tags, &p.assignees),
        None => (&[], &[]),
    };
    let mut recipients = added_tags(old_assignees, &task.assignees);
    if route_tags {
        for tag in added_tags(old_tags, &task.tags) {
            if !recipients.contains(&tag) {
                recipients.push(tag);
            }
        }
    }
    notify_tags(vault_path, task, &recipients);
}

/// Mails the agents behind `tags`, a subset of the task's own tags.
pub fn notify_tags(vault_path: &std::path::Path, task: &t::Task, tags: &[String]) {
    if tags.is_empty() {
        return;
    }
    let _ =
        mailbox::notify_tagged_agents(vault_path, &task.id, &task.display_id, &task.title, tags);
}

/// Tells each unblocked task's tagged agents that its last dependency is done.
pub fn notify_unblocked(
    vault_path: &std::path::Path,
    blocker: &t::Task,
    unblocked: &[t::Task],
    route_tags: bool,
) {
    for task in unblocked {
        let recipients = recipients_of(task, route_tags);
        if recipients.is_empty() {
            continue;
        }
        let content = format!(
            "Unblocked: {} ({} is complete)",
            task.title, blocker.display_id
        );
        let _ = mailbox::notify_tagged_agents_with(
            vault_path,
            &task.id,
            &task.display_id,
            &content,
            &recipients,
        );
    }
}

/// Everyone a task is for: its assignees, and its tags too while `route_tags` is on. A name is listed once.
fn recipients_of(task: &t::Task, route_tags: bool) -> Vec<String> {
    let mut recipients = task.assignees.clone();
    if route_tags {
        for tag in &task.tags {
            if !recipients.contains(tag) {
                recipients.push(tag.clone());
            }
        }
    }
    recipients
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
///
/// A task whose completion HQ verifies itself (`tasks.verification` enforced for its tier) is
/// not the owner's to review, so it raises nothing: the owner hears only of exceptions
/// ([`notify_owner_exception`]).
pub async fn notify_ready_for_review(vault_path: PathBuf, task: t::Task) {
    let summary = format!("{}: {}", task.display_id, task.title);
    let item = hq_core::types::ValueItem::new(
        TASK_REVIEW_SOURCE,
        hq_core::types::ValueKind::ActionNeeded,
        summary,
        task.description.clone(),
    )
    .with_dedup_key(format!("task-review-{}", task.id));

    let db_path = vault_path.join("_data").join("vault.db");
    let emitted = hq_db::Database::open(&db_path).and_then(|db| {
        // A policy that cannot be read is off, so the owner is told as before.
        if db
            .with_conn(|c| t::completion_requires_verification(c, &task))
            .unwrap_or(false)
        {
            return Ok(());
        }
        hq_db::value_items::emit(&db, &item).map(|_| ())
    });
    if let Err(e) = emitted {
        tracing::warn!(error = %e, task_id = %task.id, "task-notify: failed to emit ready-for-review value item");
    }
}

/// The value source of a review request.
pub const TASK_REVIEW_SOURCE: &str = "task_ready_for_review";
/// The value source of a task verification could not settle and only the owner can.
pub const TASK_EXCEPTION_SOURCE: &str = "task_needs_owner";
/// The value source of a tier whose auto-completion was held back by what the audits and canaries
/// show.
pub const TASK_AUTONOMY_SOURCE: &str = "task_autonomy";
/// The value source of a tier 2 task inside its veto window.
pub const TASK_VETO_SOURCE: &str = "task_veto_window";
/// The value source of the daily briefing.
pub const TASK_BRIEFING_SOURCE: &str = "task_briefing";
/// The value source of the one daily alert about the verification queue.
pub const TASK_QUEUE_ALERT_SOURCE: &str = "task_queue_alert";
/// The value source of the weekly audit digest.
pub const TASK_AUDIT_SOURCE: &str = "task_audit_digest";

/// Best-effort: tells the owner, in the web inbox, about one task that verification returned to its
/// worker as often as it allows. This is the "exception" of the exception-first model; it is
/// web-only and deduplicated per task.
pub async fn notify_owner_exception(vault_path: PathBuf, task: t::Task, findings: String) {
    let item = hq_core::types::ValueItem::new(
        TASK_EXCEPTION_SOURCE,
        hq_core::types::ValueKind::ActionNeeded,
        format!("{} needs a decision: {}", task.display_id, task.title),
        format!(
            "{}\n\nThe last findings:\n{findings}",
            task.blocked_reason.clone().unwrap_or_default()
        ),
    )
    .with_dedup_key(format!("task-exception-{}", task.id));
    let db_path = vault_path.join("_data").join("vault.db");
    let emitted = hq_db::Database::open(&db_path)
        .and_then(|db| hq_db::value_items::emit(&db, &item).map(|_| ()));
    if let Err(e) = emitted {
        tracing::warn!(error = %e, task_id = %task.id, "task-notify: failed to emit the owner exception");
    }
}

/// Best-effort: tells the owner, once per tier, that auto-completion is held back and why. The
/// alert says which numbers did it, because the fix is to read them, not to dismiss the alert.
pub async fn notify_autonomy_withdrawn(vault_path: PathBuf, tier: u8, why: String) {
    let item = hq_core::types::ValueItem::new(
        TASK_AUTONOMY_SOURCE,
        hq_core::types::ValueKind::ActionNeeded,
        format!("HQ stopped completing tier {tier} tasks on its own"),
        why,
    )
    .with_dedup_key(format!("task-autonomy-tier-{tier}"));
    let db_path = vault_path.join("_data").join("vault.db");
    let emitted = hq_db::Database::open(&db_path)
        .and_then(|db| hq_db::value_items::emit(&db, &item).map(|_| ()));
    if let Err(e) = emitted {
        tracing::warn!(error = %e, tier, "task-notify: failed to emit the autonomy alert");
    }
}

/// Best-effort: one weekly digest saying how many completed tasks are waiting for the owner's
/// agree or not.
pub async fn notify_audit_digest(vault_path: PathBuf, selected: usize, open: usize) {
    let item = hq_core::types::ValueItem::new(
        TASK_AUDIT_SOURCE,
        hq_core::types::ValueKind::Fyi,
        format!("{selected} completed tasks to check this week"),
        format!(
            "HQ completed these on its own. Open the audit view and answer agree or not for each; \
             {open} are waiting in all. A disagreement reopens the task and teaches HQ how far to trust itself."
        ),
    )
    .with_dedup_key(format!("task-audit-digest-{}", chrono::Utc::now().format("%G-W%V")));
    let db_path = vault_path.join("_data").join("vault.db");
    let emitted = hq_db::Database::open(&db_path)
        .and_then(|db| hq_db::value_items::emit(&db, &item).map(|_| ()));
    if let Err(e) = emitted {
        tracing::warn!(error = %e, "task-notify: failed to emit the audit digest");
    }
}

/// Best-effort: tells the owner a tier 2 task passed verification and completes at `release` (UTC)
/// unless vetoed. One item per task, web-only; it is the owner's chance to say no, not a request
/// to review.
pub async fn notify_veto_window(vault_path: PathBuf, task: t::Task, release: String) {
    let item = hq_core::types::ValueItem::new(
        TASK_VETO_SOURCE,
        hq_core::types::ValueKind::ActionNeeded,
        format!("{} completes at {release} UTC unless you veto it", task.display_id),
        format!(
            "{}\n\nHQ verified this tier 2 task. If it should not be done, run `hq verify veto {}` before then.",
            task.title, task.display_id
        ),
    )
    .with_dedup_key(format!("task-veto-{}", task.id));
    let db_path = vault_path.join("_data").join("vault.db");
    let emitted = hq_db::Database::open(&db_path)
        .and_then(|db| hq_db::value_items::emit(&db, &item).map(|_| ()));
    if let Err(e) = emitted {
        tracing::warn!(error = %e, "task-notify: failed to emit the veto window item");
    }
}

/// Best-effort: the briefing as one summary item, once a day. It never lists a pass by itself.
pub async fn notify_briefing(vault_path: PathBuf, text: String) {
    let item = hq_core::types::ValueItem::new(
        TASK_BRIEFING_SOURCE,
        hq_core::types::ValueKind::Fyi,
        "While you were away".to_string(),
        text,
    )
    .with_dedup_key(format!(
        "task-briefing-{}",
        chrono::Utc::now().format("%Y-%m-%d")
    ));
    emit_value(vault_path, item, "the briefing");
}

/// Best-effort: one alert a day when the verification queue needs the owner's attention.
pub async fn notify_queue_alert(vault_path: PathBuf, alerts: &[String]) {
    if alerts.is_empty() {
        return;
    }
    let item = hq_core::types::ValueItem::new(
        TASK_QUEUE_ALERT_SOURCE,
        hq_core::types::ValueKind::ActionNeeded,
        "The verification queue needs attention".to_string(),
        alerts.join("\n"),
    )
    .with_dedup_key(format!(
        "task-queue-alert-{}",
        chrono::Utc::now().format("%Y-%m-%d")
    ));
    emit_value(vault_path, item, "the queue alert");
}

fn emit_value(vault_path: PathBuf, item: hq_core::types::ValueItem, what: &str) {
    let db_path = vault_path.join("_data").join("vault.db");
    let emitted = hq_db::Database::open(&db_path)
        .and_then(|db| hq_db::value_items::emit(&db, &item).map(|_| ()));
    if let Err(e) = emitted {
        tracing::warn!(error = %e, "task-notify: failed to emit {what}");
    }
}
