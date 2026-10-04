//! Task and comment tools.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::mailbox;
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

use super::json::*;
use super::placement::*;
use crate::registry::HqTool;
use crate::util::{arg_str, generate_id};

pub(super) struct TaskCreateTool {
    pub(super) vault_path: PathBuf,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskCreateTool {
    fn name(&self) -> &str {
        "task_create"
    }
    fn description(&self) -> &str {
        "Create a task. Route it to an agent by tagging it (e.g. \"hq\", \"reviewer\") — agents don't \
         have real accounts, tags are how work gets assigned, same convention ClickUp used. A tagged \
         agent is notified immediately via its mailbox. Resolves the initiative by id if given, else \
         finds-or-creates one by (space, initiative name). For actionable work only — knowledge or \
         reference material belongs in a vault note (vault_write_note) instead. To promote an existing \
         vault note into a task, use task_create_from_note rather than copying its content by hand. \
         Pass parent_id to make a sub-task (one level deep, filed in the parent's initiative), \
         start_date/due_date to schedule it on the timeline, and depends_on for tasks that must \
         finish first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Task title" },
                "description": { "type": "string", "description": "Longer description (optional)" },
                "initiative_id": { "type": "string", "description": "Initiative to file this under, if known" },
                "space_id": { "type": "string", "description": "Space slug (e.g. 'personal', 'professional') when resolving by name instead of initiative_id", "default": "personal" },
                "folder": { "type": "string", "description": "Optional folder name to find-or-create the initiative under (matches ClickUp's Space > Folder > List). Omit for a folderless initiative directly under the space." },
                "initiative": { "type": "string", "description": "Initiative name to find-or-create, when initiative_id is not given", "default": "Inbox" },
                "priority": { "type": "string", "enum": ["urgent", "high", "normal", "low"], "description": "Optional priority" },
                "due_date": { "type": "string", "description": "Optional due date, YYYY-MM-DD" },
                "start_date": { "type": "string", "description": "Optional start date, YYYY-MM-DD (not after due_date)" },
                "parent_id": { "type": "string", "description": "Make this a sub-task of that task (id or display id). The parent must be top level." },
                "depends_on": { "type": "array", "items": { "type": "string" }, "description": "Ids or display ids of tasks that must complete before this one" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Routing tags (e.g. 'hq', 'reviewer') plus any topical tags" },
                "created_by": { "type": "string", "description": "Who is filing this (agent id or a name)", "default": "unknown" },
                "external_id": { "type": "string", "description": "Idempotency key, unique per space (up to 200 characters). Calling again with the same external_id in the same space returns the existing task with deduplicated=true instead of creating a duplicate, so a retried or repeated request is safe." }
            },
            "required": ["title"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn search_hint(&self) -> Option<&str> {
        Some("create a task, optionally routed to an agent by tag")
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let title = arg_str(&args, "title");
        if title.is_empty() {
            bail!("title is required");
        }
        let description = arg_str(&args, "description");
        let priority = args
            .get("priority")
            .and_then(|v| v.as_str())
            .map(String::from);
        let due_date = opt_str(&args, "due_date");
        let start_date = opt_str(&args, "start_date");
        let parent_id = opt_str(&args, "parent_id");
        let depends_on = tags_from_args(&args, "depends_on");
        let tags = tags_from_args(&args, "tags");
        let created_by = {
            let v = arg_str(&args, "created_by");
            if v.is_empty() {
                "unknown".to_string()
            } else {
                v
            }
        };
        let initiative_id = args
            .get("initiative_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let space_id = {
            let v = arg_str(&args, "space_id");
            if v.is_empty() {
                "personal".to_string()
            } else {
                v
            }
        };
        let folder_name = args
            .get("folder")
            .and_then(|v| v.as_str())
            .map(String::from);
        let initiative_name = {
            let v = arg_str(&args, "initiative");
            if v.is_empty() { "Inbox".to_string() } else { v }
        };

        let external_id = opt_str(&args, "external_id");
        let id = generate_id("tk");
        let (task, created) = self.db.with_conn(move |c| {
            let (task, created) = create_task_in(
                c,
                &id,
                initiative_id.as_deref(),
                &Placement {
                    space_id: &space_id,
                    folder_name: folder_name.as_deref(),
                    initiative_name: &initiative_name,
                },
                &t::NewTask {
                    title: &title,
                    description: &description,
                    priority: priority.as_deref(),
                    due_date: due_date.as_deref(),
                    start_date: start_date.as_deref(),
                    parent_task_id: parent_id.as_deref(),
                    tags: &tags,
                    created_by: &created_by,
                    external_id: external_id.as_deref(),
                },
            )?;
            if depends_on.is_empty() || !created {
                return Ok((task, created));
            }
            apply_dependency_changes(c, &task.id, &depends_on, &[], &created_by)?;
            let task = t::get_task(c, &task.id)?
                .ok_or_else(|| anyhow::anyhow!("task {} vanished after creation", task.id))?;
            Ok((task, created))
        })?;

        if !created {
            let mut out = task_json(&task);
            out["deduplicated"] = json!(true);
            return Ok(out);
        }
        if !task.tags.is_empty() {
            let _ = mailbox::notify_tagged_agents(
                &self.vault_path,
                &task.id,
                &task.display_id,
                &task.title,
                &task.tags,
            );
        }
        Ok(task_json(&task))
    }
}

// ─── task_list ──────────────────────────────────────────────────────────

pub(super) struct TaskListTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskListTool {
    fn name(&self) -> &str {
        "task_list"
    }
    fn description(&self) -> &str {
        "List tasks, filtered by any combination of space, initiative, status, tag, or priority. \
         `tag` is also how you find your own queue: agents are routed by tag (see task_create), so \
         `{\"tag\": \"reviewer\"}` returns every task tagged for the agent \"reviewer\" regardless of \
         which Space/List it's filed under. Use parent_id to list one task's sub-tasks, or \
         top_level_only to hide sub-tasks. Rows leave out `description` unless include_description \
         is true; task_get returns one task in full."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "space_id": { "type": "string" },
                "initiative_id": { "type": "string" },
                "status": { "type": "string", "enum": ["to_do", "in_progress", "blocked", "ready_for_review", "complete"] },
                "tag": { "type": "string", "description": "Exact tag match, e.g. an agent id like \"reviewer\" or \"hq\" to find that agent's routed tasks" },
                "priority": { "type": "string", "enum": ["urgent", "high", "normal", "low"] },
                "parent_id": { "type": "string", "description": "Only the sub-tasks of this task (id or display id)" },
                "top_level_only": { "type": "boolean", "description": "Exclude sub-tasks" },
                "include_description": { "type": "boolean", "description": "Include each task's full description (large lists may then be cut by the MCP gateway)" }
            }
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn search_hint(&self) -> Option<&str> {
        Some("list/filter tasks by space, initiative, status, tag, priority; find my queue by tag")
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let filter = t::TaskFilter {
            space_id: args
                .get("space_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            initiative_id: args
                .get("initiative_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            status: args
                .get("status")
                .and_then(|v| v.as_str())
                .map(String::from),
            tag: args.get("tag").and_then(|v| v.as_str()).map(String::from),
            priority: args
                .get("priority")
                .and_then(|v| v.as_str())
                .map(String::from),
            parent_task_id: parent_filter(&args),
        };
        let with_description = args
            .get("include_description")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let tasks = self.db.with_conn(move |c| t::list_tasks(c, &filter))?;
        // Full descriptions push a list past the MCP gateway's size cap, which cuts
        // the middle out of the JSON and silently drops tasks.
        let rows = tasks.iter().map(|task| {
            let mut row = task_json(task);
            if !with_description && let Some(obj) = row.as_object_mut() {
                obj.remove("description");
            }
            row
        });
        Ok(json!({
            "count": tasks.len(),
            "tasks": rows.collect::<Vec<_>>()
        }))
    }
}

// ─── task_get ───────────────────────────────────────────────────────────

pub(super) struct TaskGetTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskGetTool {
    fn name(&self) -> &str {
        "task_get"
    }
    fn description(&self) -> &str {
        "Get a single task by its internal id or display id (e.g. \"AGENT-HQ-027\"), with its \
         sub-tasks and the tasks it is blocking. blocked_by lists open dependencies."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Internal id or display id" }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let id = arg_str(&args, "id");
        if id.is_empty() {
            bail!("id is required");
        }
        let (task, subtasks, dependents, events) = self.db.with_conn(move |c| {
            let task =
                t::get_task(c, &id)?.ok_or_else(|| anyhow::anyhow!("no task found for that id"))?;
            let subtasks = t::list_subtasks(c, &task.id)?;
            let dependents = t::list_dependents(c, &task.id)?;
            let events = t::list_task_events(c, &task.id)?;
            Ok::<_, anyhow::Error>((task, subtasks, dependents, events))
        })?;
        let mut value = task_json(&task);
        value["subtasks"] = json!(subtasks.iter().map(task_summary).collect::<Vec<_>>());
        value["dependents"] = json!(dependents.iter().map(task_summary).collect::<Vec<_>>());
        value["lifecycle_events"] = json!(events);
        Ok(value)
    }
}

// ─── task_update ────────────────────────────────────────────────────────

pub(super) struct TaskUpdateTool {
    pub(super) vault_path: PathBuf,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskUpdateTool {
    fn name(&self) -> &str {
        "task_update"
    }
    fn description(&self) -> &str {
        "Update a task's fields. Only fields present in the call are changed (pass a field as null \
         to clear priority/due_date). Set expected_status to make a status change claim-safe: if the \
         task isn't in that status anymore (e.g. another agent already claimed it), the update is \
         rejected instead of silently overwriting. Re-tags trigger a fresh mailbox notification to any \
         newly-added routing tag. Dependencies are soft: starting or completing a task with open \
         dependencies succeeds but returns a warning, and completing a task notifies the agents on \
         any task it unblocked. parent_id moves a task under a parent (null promotes it to top level)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Internal id or display id" },
                "title": { "type": "string" },
                "description": { "type": "string" },
                "status": { "type": "string", "enum": ["to_do", "in_progress", "blocked", "ready_for_review", "complete"] },
                "priority": { "type": ["string", "null"], "enum": ["urgent", "high", "normal", "low", null] },
                "due_date": { "type": ["string", "null"], "description": "YYYY-MM-DD, null clears" },
                "start_date": { "type": ["string", "null"], "description": "YYYY-MM-DD, null clears" },
                "parent_id": { "type": ["string", "null"], "description": "New parent (id or display id), null promotes to top level" },
                "add_depends_on": { "type": "array", "items": { "type": "string" }, "description": "Tasks this one should wait for" },
                "remove_depends_on": { "type": "array", "items": { "type": "string" }, "description": "Dependencies to drop" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Replaces the full tag set" },
                "expected_status": { "type": "string", "description": "Claim-safe: only apply if the task is currently in this status" }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn search_hint(&self) -> Option<&str> {
        Some("update task status/fields, optionally claim-safe")
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let id = arg_str(&args, "id");
        if id.is_empty() {
            bail!("id is required");
        }
        let patch = t::TaskPatch {
            title: args.get("title").and_then(|v| v.as_str()).map(String::from),
            description: args
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            status: args
                .get("status")
                .and_then(|v| v.as_str())
                .map(String::from),
            priority: args.get("priority").map(|v| v.as_str().map(String::from)),
            due_date: args.get("due_date").map(|v| v.as_str().map(String::from)),
            start_date: args.get("start_date").map(|v| v.as_str().map(String::from)),
            parent_task_id: args.get("parent_id").map(|v| v.as_str().map(String::from)),
            tags: args.get("tags").map(|_| tags_from_args(&args, "tags")),
        };
        let expected_status = args
            .get("expected_status")
            .and_then(|v| v.as_str())
            .map(String::from);
        let add_deps = tags_from_args(&args, "add_depends_on");
        let remove_deps = tags_from_args(&args, "remove_depends_on");

        let id_for_update = id.clone();
        let (task, became_ready_for_review, unblocked) = self.db.with_conn(move |c| {
            let previous_status = t::get_task(c, &id_for_update)?.map(|t| t.status);
            let mut task = t::update_task(c, &id_for_update, &patch, expected_status.as_deref())?;
            if !add_deps.is_empty() || !remove_deps.is_empty() {
                apply_dependency_changes(c, &task.id, &add_deps, &remove_deps, "agent")?;
                task = t::get_task(c, &task.id)?
                    .ok_or_else(|| anyhow::anyhow!("task vanished after update"))?;
            }
            let became_ready = task.status == t::STATUS_READY_FOR_REVIEW
                && previous_status.as_deref() != Some(t::STATUS_READY_FOR_REVIEW);
            let unblocked = unblocked_by_transition(c, previous_status.as_deref(), &task)?;
            Ok::<_, anyhow::Error>((task, became_ready, unblocked))
        })?;
        notify_unblocked(&self.vault_path, &task, &unblocked);

        if !task.tags.is_empty() {
            let _ = mailbox::notify_tagged_agents(
                &self.vault_path,
                &task.id,
                &task.display_id,
                &task.title,
                &task.tags,
            );
        }
        if became_ready_for_review {
            notify_ready_for_review(self.vault_path.clone(), task.clone()).await;
        }
        Ok(task_json_with_warnings(&task))
    }
}

// ─── task_delete ────────────────────────────────────────────────────────

pub(super) struct TaskDeleteTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskDeleteTool {
    fn name(&self) -> &str {
        "task_delete"
    }
    fn description(&self) -> &str {
        "Permanently delete a task with its comments and dependency links. A task with sub-tasks \
         is only deleted when cascade is true, which deletes the sub-tasks too."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Internal id or display id" },
                "cascade": { "type": "boolean", "description": "Also delete the task's sub-tasks", "default": false }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let id = arg_str(&args, "id");
        if id.is_empty() {
            bail!("id is required");
        }
        let cascade = args
            .get("cascade")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let target = id.clone();
        let deleted = self
            .db
            .with_conn(move |c| t::delete_task(c, &target, cascade))?;
        Ok(json!({ "deleted": true, "id": id, "deleted_ids": deleted }))
    }
}

// ─── task_comment_add / task_comment_list ──────────────────────────────

pub(super) struct TaskCommentAddTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskCommentAddTool {
    fn name(&self) -> &str {
        "task_comment_add"
    }
    fn description(&self) -> &str {
        "Add a comment to a task."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "Internal id or display id" },
                "body": { "type": "string" },
                "author": { "type": "string", "default": "unknown" }
            },
            "required": ["task_id", "body"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let task_id = arg_str(&args, "task_id");
        let body = arg_str(&args, "body");
        if task_id.is_empty() || body.is_empty() {
            bail!("task_id and body are required");
        }
        let author = {
            let v = arg_str(&args, "author");
            if v.is_empty() {
                "unknown".to_string()
            } else {
                v
            }
        };
        let comment = self.db.with_conn(move |c| {
            let task = t::get_task(c, &task_id)?
                .ok_or_else(|| anyhow::anyhow!("no task found for that id"))?;
            t::add_comment(c, &task.id, &author, &body, None)
        })?;
        Ok(json!({
            "id": comment.id,
            "task_id": comment.task_id,
            "author": comment.author,
            "body": comment.body,
            "created_at": comment.created_at,
        }))
    }
}

pub(super) struct TaskCommentListTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskCommentListTool {
    fn name(&self) -> &str {
        "task_comment_list"
    }
    fn description(&self) -> &str {
        "List comments on a task, oldest first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "Internal id or display id" }
            },
            "required": ["task_id"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let task_id = arg_str(&args, "task_id");
        if task_id.is_empty() {
            bail!("task_id is required");
        }
        let comments = self.db.with_conn(move |c| {
            let task = t::get_task(c, &task_id)?
                .ok_or_else(|| anyhow::anyhow!("no task found for that id"))?;
            t::list_comments(c, &task.id)
        })?;
        Ok(json!({
            "count": comments.len(),
            "comments": comments.iter().map(|c| json!({
                "id": c.id, "author": c.author, "body": c.body, "created_at": c.created_at,
            })).collect::<Vec<_>>()
        }))
    }
}
