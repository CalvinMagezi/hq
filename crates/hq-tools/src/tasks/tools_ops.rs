//! Read-only views for keeping a long backlog honest: which in-progress tasks look
//! abandoned and what to do about each, and how an initiative is going.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::config::TasksConfig;
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::sync::Arc;

use super::tools_lease::ttl_secs;
use crate::registry::HqTool;
use crate::util::arg_str;

/// Most stale tasks one reply lists.
const MAX_STALE_LISTED: usize = 100;
/// An in-progress task idle this long is probably obsolete, not merely paused.
const OBSOLETE_AFTER_HOURS: i64 = 30 * 24;

/// What to do about one stale task, with the reason. A proposal for a person or an
/// agent to act on; HQ applies none of them.
pub fn propose(stale: &t::StaleTask, has_open_subtasks: bool, has_checkpoint: bool) -> Value {
    let days = stale.idle_hours / 24;
    let (action, why) = if has_open_subtasks {
        ("review_subtasks", format!("{days} days idle, and it has open sub-tasks: decide those first"))
    } else if stale.idle_hours >= OBSOLETE_AFTER_HOURS {
        ("close_or_release", format!("untouched for {days} days: close it if it no longer matters, otherwise move it back to to_do"))
    } else if has_checkpoint {
        ("resume", format!("{days} days idle with a checkpoint to resume from: claim it with task_claim"))
    } else {
        ("release", format!("{days} days with nobody on it: move it back to to_do, or claim it with task_claim"))
    };
    json!({
        "task_id": stale.task_id,
        "display_id": stale.display_id,
        "title": stale.title,
        "idle_hours": stale.idle_hours,
        "last_activity_at": stale.last_activity_at,
        "suggested_action": action,
        "why": why,
    })
}

pub(super) struct TaskStaleTool {
    pub(super) settings: TasksConfig,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskStaleTool {
    fn name(&self) -> &str {
        "task_stale"
    }
    fn description(&self) -> &str {
        "In-progress tasks that look abandoned: nobody holds a lease and nothing has been written, \
         commented or heartbeated for the stale window (tasks.stale_after_hours, default 72). Each comes \
         with a suggested action and why. Nothing is changed: decide per task, then release, block (with a \
         reason), close or claim it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "hours": { "type": "integer", "minimum": 1, "description": "Override the stale window for this call" }
            }
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let hours = args
            .get("hours")
            .and_then(Value::as_i64)
            .filter(|h| *h >= 1)
            .unwrap_or(i64::try_from(self.settings.stale_hours()).unwrap_or(i64::MAX));
        let ttl = ttl_secs(&self.settings);
        let proposals = self.db.with_conn(move |c| {
            let stale = t::stale_tasks(c, hours, ttl, MAX_STALE_LISTED)?;
            stale
                .iter()
                .map(|s| {
                    let open_children = t::list_subtasks(c, &s.task_id)?.iter().any(|k| k.status != t::STATUS_COMPLETE);
                    let checkpoint = t::latest_checkpoint(c, &s.task_id)?.is_some();
                    Ok(propose(s, open_children, checkpoint))
                })
                .collect::<Result<Vec<Value>>>()
        })?;
        Ok(json!({ "stale_after_hours": hours, "count": proposals.len(), "tasks": proposals }))
    }
}

pub(super) struct InitiativeProgressTool {
    pub(super) settings: TasksConfig,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for InitiativeProgressTool {
    fn name(&self) -> &str {
        "initiative_progress"
    }
    fn description(&self) -> &str {
        "How an initiative is going: tasks by status, share complete, summed estimates, worked time and how \
         many in-progress tasks look stale. An initiative is the epic in HQ: file a large piece of work \
         as its own initiative with a task per workstream."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "initiative": { "type": "string", "description": "Initiative id, slug, display prefix or name" }
            },
            "required": ["initiative"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let initiative = arg_str(&args, "initiative");
        if initiative.is_empty() {
            bail!("initiative is required");
        }
        let hours = i64::try_from(self.settings.stale_hours()).unwrap_or(i64::MAX);
        let ttl = ttl_secs(&self.settings);
        let rollup = self.db.with_conn(move |c| t::initiative_rollup(c, &initiative, hours, ttl))?;
        Ok(json!(rollup))
    }
}

pub(super) struct TaskRoutingAuditTool {
    pub(super) settings: TasksConfig,
    pub(super) vault_path: std::path::PathBuf,
    pub(super) db: Arc<Database>,
}

/// Most agent mailboxes one audit lists.
const MAX_MAILBOXES: usize = 200;

#[async_trait]
impl HqTool for TaskRoutingAuditTool {
    fn name(&self) -> &str {
        "task_routing_audit"
    }
    fn description(&self) -> &str {
        "Which tags currently route tasks to an agent mailbox, and how many open tasks each affects, next to how \
         many are assigned to that agent. Run it before moving routing from tags to assignees: it changes \
         nothing. When every task an agent should get is assigned to it, set tasks.route_tags to false and \
         tags stay purely topical."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, _args: Value) -> Result<Value> {
        let dir = self.vault_path.join(hq_core::mailbox::MAILBOX_DIR);
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names.truncate(MAX_MAILBOXES);
        let rows = self.db.with_conn(move |c| {
            names
                .iter()
                .map(|name| {
                    let open = |join: &str| -> Result<i64> {
                        Ok(c.query_row(
                            &format!(
                                "SELECT COUNT(DISTINCT t.id) FROM tasks t JOIN {join} x ON x.task_id = t.id \
                                 WHERE x.{col} = ?1 AND t.archived_at IS NULL AND t.status <> 'complete'",
                                col = if join == "task_tags" { "tag" } else { "assignee" }
                            ),
                            [name],
                            |r| r.get(0),
                        )?)
                    };
                    let tagged = open("task_tags")?;
                    let assigned = open("task_assignees")?;
                    let tagged_only: i64 = c.query_row(
                        "SELECT COUNT(DISTINCT t.id) FROM tasks t JOIN task_tags g ON g.task_id = t.id \
                         WHERE g.tag = ?1 AND t.archived_at IS NULL AND t.status <> 'complete' \
                           AND NOT EXISTS (SELECT 1 FROM task_assignees a WHERE a.task_id = t.id AND a.assignee = ?1)",
                        [name],
                        |r| r.get(0),
                    )?;
                    Ok(json!({ "mailbox": name, "open_tasks_tagged": tagged, "open_tasks_assigned": assigned, "tagged_but_not_assigned": tagged_only }))
                })
                .collect::<Result<Vec<Value>>>()
        })?;
        Ok(json!({
            "route_tags": self.settings.route_tags,
            "mailboxes": rows,
            "next": "A tagged_but_not_assigned count above zero is work that reaches that agent only through its tag. \
                     Assign those tasks to it (task_update assignees), then set tasks.route_tags to false.",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stale(idle_hours: i64) -> t::StaleTask {
        t::StaleTask {
            task_id: "tk-1".into(),
            display_id: "FR-1".into(),
            title: "t".into(),
            initiative_id: "in-1".into(),
            last_activity_at: "2026-01-01 00:00:00".into(),
            idle_hours,
        }
    }

    #[test]
    fn a_task_with_open_subtasks_is_reviewed_through_them_first() {
        assert_eq!(propose(&stale(24 * 100), true, true)["suggested_action"], "review_subtasks");
    }

    #[test]
    fn a_long_untouched_task_is_closed_or_released() {
        assert_eq!(propose(&stale(24 * 45), false, true)["suggested_action"], "close_or_release");
    }

    #[test]
    fn a_recent_one_with_a_checkpoint_is_resumed_and_without_one_released() {
        assert_eq!(propose(&stale(24 * 5), false, true)["suggested_action"], "resume");
        assert_eq!(propose(&stale(24 * 5), false, false)["suggested_action"], "release");
        assert!(propose(&stale(24 * 5), false, false)["why"].as_str().unwrap().contains("5 days"));
    }
}
