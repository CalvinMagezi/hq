//! Task and comment tools.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

use super::json::*;
use super::placement::*;
use super::tools_lease::{ActorHints, add_warning, lease_policy};
use hq_core::config::TasksConfig;
use crate::registry::HqTool;
use crate::util::{arg_str, generate_id};

/// Longest title and longest free text (description, comment) the tasks scope may write.
/// The scope has no rate limit, so these bound what one call can add to the owner's database.
const SCOPED_TITLE_MAX: usize = 500;
const SCOPED_TEXT_MAX: usize = 20_000;

/// Refuses a tasks-scope write whose `field` is longer than `max` characters.
fn scoped_len(args: &Value, field: &str, max: usize) -> Result<()> {
    let len = args.get(field).and_then(Value::as_str).map_or(0, |s| s.chars().count());
    if len > max {
        bail!("{field} is {len} characters; this connection may write at most {max}");
    }
    Ok(())
}

/// Checks every text field a tasks-scope write carries.
fn check_scoped_text(args: &Value) -> Result<()> {
    scoped_len(args, "title", SCOPED_TITLE_MAX)?;
    scoped_len(args, "description", SCOPED_TEXT_MAX)?;
    scoped_len(args, "body", SCOPED_TEXT_MAX)
}

pub(super) struct TaskCreateTool {
    pub(super) settings: TasksConfig,
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
                "estimate_minutes": { "type": "integer", "minimum": 1, "description": "Planned effort in minutes. Time you actually spend is recorded from your lease, so set an estimate when you can and it will be compared." },
                "parent_id": { "type": "string", "description": "Make this a sub-task of that task (id or display id). The parent must be top level." },
                "depends_on": { "type": "array", "items": { "type": "string" }, "description": "Ids or display ids of tasks that must complete before this one" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Routing tags (e.g. 'hq', 'reviewer') plus any topical tags" },
                "created_by": { "type": "string", "description": "Who is filing this (agent id or a name). Ignored when a valid `lease` is given.", "default": "unknown" },
                "lease": { "type": "string", "description": "Your lease token from task_claim, to attribute this to you" },
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
        // The tasks scope sets no routing tags (a tag names a mailbox that an agent or the
        // owner's chat drains) and cannot choose who a write is attributed to.
        let scoped = crate::harness_session::is_tasks_scope(&args);
        if scoped {
            check_scoped_text(&args)?;
        }
        let tags = if scoped { Vec::new() } else { tags_from_args(&args, "tags") };
        let hints = ActorHints::from_args(&args, "created_by");
        let settings = self.settings.clone();
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
        let estimate_minutes = estimate_arg(&args)?.flatten();
        let id = generate_id("tk");
        let (task, created) = self.db.with_conn(move |c| {
            // A live lease names the filer; otherwise the caller's own words.
            let created_by = hints.resolve(c, &settings, None)?.name;
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
                    estimate_minutes,
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
        if !scoped {
            notify_tags(&self.vault_path, &task, &task.tags);
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
         is true; task_get returns one task in full. Replies are paged: check `has_more` and pass \
         `offset` for the rest, never assume one reply is the whole list."
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
                "include_description": { "type": "boolean", "description": "Include each task's full description (large lists may then be cut by the MCP gateway)" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Page size (default 100). The reply has `total` and `has_more`; read the next page with `offset`." },
                "offset": { "type": "integer", "minimum": 0, "description": "Rows to skip, for the next page" }
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
        let mut filter = t::TaskFilter {
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
            ..Default::default()
        };
        let with_description = args
            .get("include_description")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let limit = arg_usize(&args, "limit")
            .unwrap_or(DEFAULT_LIST_PAGE)
            .clamp(1, t::MAX_LIST_LIMIT);
        let offset = arg_usize(&args, "offset").unwrap_or(0);
        let caller = crate::harness_session::caller_session(&args).map(str::to_string);
        let (tasks, total) = self.db.with_conn(move |c| match caller {
            // A launched agent sees only the tasks it may use, so the scope is
            // applied before paging and the total counts what it can see.
            Some(session) => {
                let scope = crate::a2a::task_scope(c, &session)?;
                let visible = visible_tasks(c, &mut filter, &scope)?;
                let total = visible.len() as i64;
                Ok((visible.into_iter().skip(offset).take(limit).collect(), total))
            }
            None => {
                filter.limit = Some(limit);
                filter.offset = offset;
                Ok((t::list_tasks(c, &filter)?, t::count_tasks(c, &filter)?))
            }
        })?;
        // Full descriptions push a list past the MCP gateway's size cap, which cuts
        // the middle out of the JSON and silently drops tasks.
        let rows = tasks.iter().map(|task| {
            let mut row = task_json(task);
            if !with_description && let Some(obj) = row.as_object_mut() {
                obj.remove("description");
            }
            row
        });
        let next_offset = offset + tasks.len();
        Ok(json!({
            "count": tasks.len(),
            "total": total,
            "offset": offset,
            "has_more": (next_offset as i64) < total,
            "tasks": rows.collect::<Vec<_>>()
        }))
    }
}

/// The `estimate_minutes` argument: absent is untouched, null clears, a whole number
/// sets. Anything else (a string, 30.5) is an error, never a silent clear or drop.
fn estimate_arg(args: &Value) -> Result<Option<Option<i64>>> {
    match args.get("estimate_minutes") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => match v.as_i64() {
            Some(minutes) => Ok(Some(Some(minutes))),
            None => bail!("estimate_minutes must be a whole number of minutes, or null to clear it"),
        },
    }
}

/// Most recent work leases `task_get` returns.
const WORK_SESSIONS_SHOWN: usize = 20;

/// Default page for `task_list`: small enough that a reply with the default
/// fields stays well under the MCP gateway's size cap.
const DEFAULT_LIST_PAGE: usize = 100;

fn arg_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(Value::as_u64).map(|n| n as usize)
}

/// Every task matching `filter` that is in `scope`, read a page at a time so a
/// scoped session is never cut off by the page cap.
fn visible_tasks(
    c: &rusqlite::Connection,
    filter: &mut t::TaskFilter,
    scope: &std::collections::HashSet<String>,
) -> Result<Vec<t::Task>> {
    let mut visible = Vec::new();
    filter.limit = Some(t::MAX_LIST_LIMIT);
    filter.offset = 0;
    loop {
        let page = t::list_tasks(c, filter)?;
        let full = page.len() == t::MAX_LIST_LIMIT;
        visible.extend(page.into_iter().filter(|task| scope.contains(&task.id)));
        if !full {
            return Ok(visible);
        }
        filter.offset += t::MAX_LIST_LIMIT;
    }
}

// ─── task_get ───────────────────────────────────────────────────────────

pub(super) struct TaskGetTool {
    pub(super) settings: TasksConfig,
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
        let caller = crate::harness_session::caller_session(&args).map(str::to_string);
        let scoped = crate::harness_session::is_tasks_scope(&args);
        let ttl = super::tools_lease::ttl_secs(&self.settings);
        let (task, subtasks, dependents, events, sessions, time) = self.db.with_conn(move |c| {
            let task =
                t::get_task(c, &id)?.ok_or_else(|| anyhow::anyhow!("no task found for that id"))?;
            crate::a2a::check_task_access(c, caller.as_deref(), &task.id)?;
            let subtasks = t::list_subtasks(c, &task.id)?;
            let dependents = t::list_dependents(c, &task.id)?;
            let events = t::list_task_events(c, &task.id)?;
            // A lease that went silent is closed first, so a crashed session is
            // never shown as still working.
            t::expire_stale_leases(c, ttl)?;
            let sessions = t::list_work_sessions(c, &task.id, WORK_SESSIONS_SHOWN)?;
            let time = t::time_summary(c, &task.id, ttl)?;
            Ok::<_, anyhow::Error>((task, subtasks, dependents, events, sessions, time))
        })?;
        let mut value = task_json(&task);
        value["subtasks"] = json!(subtasks.iter().map(task_summary).collect::<Vec<_>>());
        value["dependents"] = json!(dependents.iter().map(task_summary).collect::<Vec<_>>());
        value["lifecycle_events"] = json!(events);
        value["time"] = json!(time);
        value["held_by"] = json!(sessions.iter().find(|s| s.ended_at.is_none()));
        value["work_sessions"] = json!(sessions);
        if scoped {
            hide_work_details_from_tasks_scope(&mut value);
        }
        Ok(value)
    }
}

/// A tasks-scope caller runs on a machine the owner may not control, so it learns who holds a
/// task and since when, never which machine, folder or branch any session worked in.
fn hide_work_details_from_tasks_scope(task: &mut Value) {
    let Some(obj) = task.as_object_mut() else { return };
    obj.remove("work_sessions");
    if let Some(held) = obj.get("held_by").filter(|h| !h.is_null()).cloned() {
        obj.insert(
            "held_by".into(),
            json!({
                "actor": held["actor"],
                "harness": held["harness"],
                "started_at": held["started_at"],
                "last_heartbeat_at": held["last_heartbeat_at"],
            }),
        );
    }
}

// ─── task_update ────────────────────────────────────────────────────────

pub(super) struct TaskUpdateTool {
    pub(super) settings: TasksConfig,
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
         rejected instead of silently overwriting. Only tags newly added by this call are notified; \
         any other edit is silent. Dependencies are soft: starting or completing a task with open \
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
                "estimate_minutes": { "type": ["integer", "null"], "minimum": 1, "description": "Planned effort in minutes, null clears" },
                "parent_id": { "type": ["string", "null"], "description": "New parent (id or display id), null promotes to top level" },
                "add_depends_on": { "type": "array", "items": { "type": "string" }, "description": "Tasks this one should wait for" },
                "remove_depends_on": { "type": "array", "items": { "type": "string" }, "description": "Dependencies to drop" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Replaces the full tag set" },
                "expected_status": { "type": "string", "description": "Claim-safe: only apply if the task is currently in this status" },
                "lease": { "type": "string", "description": "Your lease token from task_claim. Attributes the change to you, and is required to start a task when the instance asks for leases." },
                "actor": { "type": "string", "description": "Your name, used only when you have no lease" }
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
        let scoped = crate::harness_session::is_tasks_scope(&args);
        if scoped {
            check_scoped_text(&args)?;
        }
        // The goal of a linked session and the prompt of a launched one are built from a
        // task's title and description, so the tasks scope may rewrite them only on tasks it
        // filed itself. Status, priority, dates and comments stay open to it.
        let edits_text = args.get("title").is_some() || args.get("description").is_some();
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
            // The tasks scope cannot set routing tags: they name mailboxes other parties drain.
            tags: if scoped {
                None
            } else {
                args.get("tags").map(|_| tags_from_args(&args, "tags"))
            },
            estimate_minutes: estimate_arg(&args)?,
        };
        let expected_status = args
            .get("expected_status")
            .and_then(|v| v.as_str())
            .map(String::from);
        let add_deps = tags_from_args(&args, "add_depends_on");
        let remove_deps = tags_from_args(&args, "remove_depends_on");

        let id_for_update = id.clone();
        let hints = ActorHints::from_args(&args, "actor");
        let settings = self.settings.clone();
        let entering_in_progress = patch.status.as_deref() == Some(t::STATUS_IN_PROGRESS);
        // One transaction: the status and the dependency change commit together
        // or not at all, and "previous" is read under the same write lock.
        let (task, previous, unblocked, lease_warning) = self.db.with_conn(move |c| {
            t::in_write_tx(c, |c| {
                let previous = t::get_task(c, &id_for_update)?
                    .ok_or_else(|| anyhow::anyhow!("task {id_for_update} not found"))?;
                if scoped
                    && edits_text
                    && !crate::harness_session::is_tasks_scope_actor(&previous.created_by)
                {
                    bail!(
                        "this connection may edit the title and description only of tasks it created; \
                         add a comment to that task instead"
                    );
                }
                let actor = hints.resolve(c, &settings, Some(&previous.id))?;
                let started = entering_in_progress && previous.status != t::STATUS_IN_PROGRESS;
                let lease_warning = lease_policy(
                    settings.require_lease,
                    started,
                    actor.holds(&previous.id),
                    &previous.display_id,
                )?;
                let mut task = t::update_task_as(
                    c,
                    &id_for_update,
                    &patch,
                    expected_status.as_deref(),
                    &actor.ctx(),
                )?;
                if !add_deps.is_empty() || !remove_deps.is_empty() {
                    apply_dependency_changes(c, &task.id, &add_deps, &remove_deps, &actor.name)?;
                    task = t::get_task(c, &task.id)?
                        .ok_or_else(|| anyhow::anyhow!("task vanished after update"))?;
                }
                let unblocked = unblocked_by_transition(c, Some(&previous.status), &task)?;
                Ok((task, previous, unblocked, lease_warning))
            })
        })?;
        let became_ready_for_review = task.status == t::STATUS_READY_FOR_REVIEW
            && previous.status != t::STATUS_READY_FOR_REVIEW;
        if !scoped {
            notify_unblocked(&self.vault_path, &task, &unblocked);
            notify_tags(&self.vault_path, &task, &added_tags(&previous.tags, &task.tags));
        }
        // A web-UI approval item with text the caller chose, one per task: not for the tasks scope.
        if became_ready_for_review && !scoped {
            notify_ready_for_review(self.vault_path.clone(), task.clone()).await;
        }
        let mut out = task_json_with_warnings(&task);
        if let Some(warning) = lease_warning {
            add_warning(&mut out, warning);
        }
        Ok(out)
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
    pub(super) settings: TasksConfig,
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
                "author": { "type": "string", "default": "unknown", "description": "Your name. Ignored when a valid `lease` is given." },
                "lease": { "type": "string", "description": "Your lease token from task_claim, to attribute this comment to you" }
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
        if crate::harness_session::is_tasks_scope(&args) {
            check_scoped_text(&args)?;
        }
        // A launched agent that proved its session is that session, whatever
        // name it supplies; a live lease names its holder; otherwise the
        // caller's own words.
        let hints = ActorHints::from_args(&args, "author");
        let settings = self.settings.clone();
        let caller = crate::harness_session::caller_session(&args).map(str::to_string);
        let comment = self.db.with_conn(move |c| {
            let task = t::get_task(c, &task_id)?
                .ok_or_else(|| anyhow::anyhow!("no task found for that id"))?;
            crate::a2a::check_task_access(c, caller.as_deref(), &task.id)?;
            let author = hints.resolve(c, &settings, Some(&task.id))?.name;
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
        let caller = crate::harness_session::caller_session(&args).map(str::to_string);
        let comments = self.db.with_conn(move |c| {
            let task = t::get_task(c, &task_id)?
                .ok_or_else(|| anyhow::anyhow!("no task found for that id"))?;
            crate::a2a::check_task_access(c, caller.as_deref(), &task.id)?;
            t::list_comments(c, &task.id)
        })?;
        Ok(json!({
            "count": comments.len(),
            "comments": comments.iter().map(|c| json!({
                "id": c.id, "author": c.author, "body": c.body, "created_at": c.created_at,
                "kind": c.kind, "to_session": c.to_session_id, "reply_to": c.reply_to,
            })).collect::<Vec<_>>()
        }))
    }
}
