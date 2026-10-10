//! Typed links between a task and what it came from or produced, and the
//! deterministic advice returned when a task is created.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::config::TasksConfig;
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::sync::Arc;

use super::json::*;
use super::tools_lease::ActorHints;
use crate::registry::HqTool;
use crate::util::arg_str;

const KIND_LIST: &str = "vault_note, chat_thread, session, commit, pr, url, task";

/// A link requested alongside a new task, parsed and checked before anything is written.
#[derive(Debug, Clone)]
pub(super) struct LinkRequest {
    pub kind: String,
    pub reference: String,
    pub label: String,
    pub direction: Option<String>,
}

/// The `links` argument of `task_create`: an array of `{kind, ref, label?, direction?}`.
pub(super) fn link_requests(args: &Value) -> Result<Vec<LinkRequest>> {
    let Some(items) = args.get("links") else {
        return Ok(Vec::new());
    };
    let Some(items) = items.as_array() else {
        bail!("links must be an array of {{kind, ref}} objects");
    };
    items
        .iter()
        .map(|item| {
            let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
            let (Some(kind), Some(reference)) = (text("kind"), text("ref")) else {
                bail!("each link needs a kind and a ref");
            };
            let direction = agent_direction(&kind, text("direction").as_deref());
            Ok(LinkRequest { kind, reference, label: text("label").unwrap_or_default(), direction })
        })
        .collect()
}

/// A link an agent writes cannot claim to be where the task came from when it names a
/// chat or a session: only HQ records those origins, from what it actually observed.
pub(super) fn agent_direction(kind: &str, direction: Option<&str>) -> Option<String> {
    let claims_origin = direction == Some(t::DIRECTION_ORIGIN);
    if claims_origin && matches!(kind, t::LINK_CHAT_THREAD | t::LINK_SESSION) {
        return Some(t::DIRECTION_RELATED.to_string());
    }
    direction.map(str::to_string)
}

/// Refuses a malformed link before the task it belongs to is created.
pub(super) fn check_links(conn: &rusqlite::Connection, requests: &[LinkRequest]) -> Result<()> {
    for r in requests {
        t::normalize_link(conn, &r.kind, &r.reference, r.direction.as_deref())?;
    }
    Ok(())
}

pub(super) struct TaskLinkAddTool {
    pub(super) settings: TasksConfig,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskLinkAddTool {
    fn name(&self) -> &str {
        "task_link_add"
    }
    fn description(&self) -> &str {
        "Link a task to what it came from or produced, so it can be found from the other side. Kinds: \
         vault_note (a path such as Notebooks/Projects/plan.md), chat_thread (a thread id), session \
         (a harness session id), commit (a sha or repo@sha), pr (owner/repo#123 or a GitHub pull \
         request URL), url (http or https) and task (another task's id). Use direction origin for \
         what the task came from, produced for what it made, related otherwise. Adding a link that \
         exists returns it. Link every task that came from a note or a conversation."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "Internal id or display id" },
                "kind": { "type": "string", "enum": ["vault_note", "chat_thread", "session", "commit", "pr", "url", "task"] },
                "ref": { "type": "string", "description": "What to link to, in the shape its kind needs" },
                "label": { "type": "string", "description": "Short note on why, optional" },
                "direction": { "type": "string", "enum": ["origin", "related", "produced"], "default": "related" },
                "lease": { "type": "string", "description": "Your lease token from task_claim, to attribute this to you" },
                "actor": { "type": "string", "description": "Your name, used only when you have no lease" }
            },
            "required": ["task_id", "kind", "ref"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let (task_id, kind, reference) = (arg_str(&args, "task_id"), arg_str(&args, "kind"), arg_str(&args, "ref"));
        if task_id.is_empty() || kind.is_empty() || reference.is_empty() {
            bail!("task_id, kind and ref are required ({KIND_LIST})");
        }
        let label = arg_str(&args, "label");
        let direction = agent_direction(&kind, opt_str(&args, "direction").as_deref());
        let hints = ActorHints::from_args(&args, "actor");
        let settings = self.settings.clone();
        let (link, created) = self.db.with_conn(move |c| {
            let who = hints.resolve(c, &settings, None)?.name;
            t::add_task_link(c, &task_id, &kind, &reference, &label, direction.as_deref(), &who)
        })?;
        Ok(json!({ "created": created, "link": link }))
    }
}

pub(super) struct TaskLinkRemoveTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskLinkRemoveTool {
    fn name(&self) -> &str {
        "task_link_remove"
    }
    fn description(&self) -> &str {
        "Remove a link from a task. Pass the same kind and ref you added it with."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "Internal id or display id" },
                "kind": { "type": "string", "enum": ["vault_note", "chat_thread", "session", "commit", "pr", "url", "task"] },
                "ref": { "type": "string" }
            },
            "required": ["task_id", "kind", "ref"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let (task_id, kind, reference) = (arg_str(&args, "task_id"), arg_str(&args, "kind"), arg_str(&args, "ref"));
        if task_id.is_empty() || kind.is_empty() || reference.is_empty() {
            bail!("task_id, kind and ref are required");
        }
        let removed = self.db.with_conn(move |c| t::remove_task_link(c, &task_id, &kind, &reference))?;
        Ok(json!({ "removed": removed }))
    }
}

pub(super) struct TaskLinkListTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskLinkListTool {
    fn name(&self) -> &str {
        "task_link_list"
    }
    fn description(&self) -> &str {
        "List a task's links with task_id, or find the tasks that link to a thing with kind and ref, for \
         example every task that came from a vault note (kind vault_note, ref the note path) or a chat \
         thread. Use this before creating a task from a note to see whether one exists already."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "List this task's links" },
                "kind": { "type": "string", "enum": ["vault_note", "chat_thread", "session", "commit", "pr", "url", "task"], "description": "With ref, find the tasks linked to it" },
                "ref": { "type": "string" }
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
        let task_id = opt_str(&args, "task_id");
        let (kind, reference) = (opt_str(&args, "kind"), opt_str(&args, "ref"));
        // The tasks scope never learns the path of a note in the owner's vault, nor which tasks
        // started from one.
        let scoped = crate::harness_session::is_tasks_scope(&args);
        if scoped && kind.as_deref() == Some(t::LINK_VAULT_NOTE) {
            bail!("this connection cannot look up vault notes");
        }
        match (task_id, kind, reference) {
            (Some(task), _, _) => {
                let mut links = self.db.with_conn(move |c| t::list_task_links(c, &task))?;
                if scoped {
                    links.retain(|l| l.kind != t::LINK_VAULT_NOTE);
                }
                Ok(json!({ "count": links.len(), "links": links }))
            }
            (None, Some(kind), Some(reference)) => {
                let tasks = self.db.with_conn(move |c| t::tasks_linked_to(c, &kind, &reference))?;
                Ok(json!({ "count": tasks.len(), "tasks": tasks.iter().map(task_summary).collect::<Vec<_>>() }))
            }
            _ => bail!("pass task_id, or kind and ref together"),
        }
    }
}

/// Ids of the tasks already tied to this one on purpose: its parent and sub-tasks, its
/// siblings, and what it depends on or blocks.
fn relative_ids(conn: &rusqlite::Connection, task: &t::Task) -> std::collections::HashSet<String> {
    let mut ids: std::collections::HashSet<String> = hq_db::task_graph::explicit_links(conn, &task.id)
        .unwrap_or_default()
        .into_iter()
        .map(|l| l.task.id)
        .collect();
    if let Some(parent) = &task.parent_task_id
        && let Ok(siblings) = t::list_subtasks(conn, parent)
    {
        ids.extend(siblings.into_iter().map(|s| s.id));
    }
    ids
}

/// How many near-duplicates a create reply lists.
const MAX_REPORTED_DUPLICATES: usize = 3;

/// What HQ can say about a task that was just created, from the tasks it already
/// has: open tasks it may duplicate, and an estimate if similar finished tasks agree.
/// Hints with their evidence; nothing is changed. Any failure is simply no advice.
pub(super) fn advice_for_new_task(conn: &rusqlite::Connection, task: &t::Task) -> Value {
    let similar = match hq_db::task_graph::similar_to_text(
        conn,
        Some(&task.id),
        &task.initiative_id,
        &task.title,
        &task.description,
        &task.tags,
    ) {
        Ok(similar) => similar,
        Err(e) => {
            tracing::debug!(error = %e, "task advice: similarity unavailable");
            return json!({});
        }
    };
    // A parent, a sub-task, a sibling or a task it depends on is related by design, not a duplicate.
    let relatives = relative_ids(conn, task);
    let duplicates: Vec<Value> = similar
        .iter()
        .filter(|s| s.task.status != t::STATUS_COMPLETE && !relatives.contains(&s.task.id))
        .take(MAX_REPORTED_DUPLICATES)
        .map(|s| json!({
            "task_id": s.task.id,
            "display_id": s.task.display_id,
            "title": s.task.title,
            "status": s.task.status,
            "score": s.score,
            "evidence": s.evidence,
        }))
        .collect();
    let mut advice = json!({});
    if !duplicates.is_empty() {
        advice["similar_open_tasks"] = json!(duplicates);
        advice["similar_note"] = json!(
            "These open tasks read like this one. If one is the same work, link or continue it and delete this one."
        );
    }
    if task.estimate_minutes.is_none()
        && let Ok(Some(suggestion)) = hq_db::task_graph::suggest_estimate(conn, &similar)
    {
        advice["suggested_estimate"] = json!({
            "minutes": suggestion.minutes,
            "based_on": suggestion.based_on,
            "note": "The median time similar finished tasks took. Set estimate_minutes if you agree.",
        });
    }
    advice
}
