//! Work lease tools (`task_claim`, `task_heartbeat`, `task_release`) and the
//! helpers that attribute a task write to whoever made it.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::config::{HqConfig, LeaseMode, TasksConfig};
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::sync::Arc;

use super::json::*;
use crate::registry::HqTool;
use crate::util::arg_str;

/// A heartbeat this many times per TTL keeps a lease alive through a missed one.
const HEARTBEATS_PER_TTL: u64 = 3;

/// The task settings in force. Read once when the tools are built, like the other
/// sections, so a config change applies on the next start.
pub fn task_settings() -> TasksConfig {
    HqConfig::load().map(|c| c.tasks).unwrap_or_default()
}

/// The lease ttl in force, for callers outside the tools such as the REST layer.
pub fn lease_ttl_secs() -> i64 {
    ttl_secs(&task_settings())
}

pub(super) fn ttl_secs(settings: &TasksConfig) -> i64 {
    i64::try_from(settings.lease_ttl()).unwrap_or(i64::MAX)
}

/// What a call says about who is making it, before it is checked against the
/// database. Built from the call's arguments, so it can be cloned into a closure.
#[derive(Clone)]
pub(super) struct ActorHints {
    caller_session: Option<String>,
    lease_token: Option<String>,
    named: String,
    tasks_scope: bool,
}

/// Who a write is attributed to, and the lease behind it when there is one.
pub(super) struct Actor {
    pub name: String,
    pub work_session_id: Option<String>,
    pub lease: Option<t::WorkSession>,
}

impl Actor {
    pub fn ctx(&self) -> t::WriteCtx<'_> {
        t::WriteCtx { actor: Some(&self.name), work_session_id: self.work_session_id.as_deref() }
    }

    /// Whether the actor holds a live lease on this task.
    pub fn holds(&self, task_id: &str) -> bool {
        self.lease.as_ref().is_some_and(|l| l.task_id == task_id)
    }
}

impl ActorHints {
    /// `name_arg` is the free-text field the tool already had (`author`,
    /// `created_by` or `actor`), used only when nothing stronger is present.
    pub fn from_args(args: &Value, name_arg: &str) -> Self {
        Self {
            caller_session: crate::harness_session::caller_session(args).map(str::to_string),
            lease_token: opt_str(args, "lease"),
            named: arg_str(args, name_arg),
            tasks_scope: crate::harness_session::is_tasks_scope(args),
        }
    }

    /// A launched session proven by its token wins, then a live lease, then the
    /// name the caller gave. A lease token that is wrong or ended is an error,
    /// never a silent downgrade to an anonymous write. A lease names who is
    /// acting anywhere, but it is recorded as the work session only on its own
    /// task (`target_task`), so a lease never inflates another task's history.
    pub fn resolve(
        &self,
        conn: &rusqlite::Connection,
        settings: &TasksConfig,
        target_task: Option<&str>,
    ) -> Result<Actor> {
        if let Some(session) = &self.caller_session {
            let lease = t::lease_for_session(conn, session)?;
            return Ok(Actor {
                name: session.clone(),
                work_session_id: lease.as_ref().map(|l| l.id.clone()),
                lease,
            });
        }
        if let Some(token) = &self.lease_token {
            let lease = t::require_live_lease(conn, token, ttl_secs(settings))?;
            // The tasks scope never writes under a name it did not get on that scope.
            if self.tasks_scope && !crate::harness_session::is_tasks_scope_actor(&lease.actor) {
                bail!("that lease was not taken on this connection; claim the task here to get your own");
            }
            let on_own_task = target_task.is_some_and(|task| task == lease.task_id);
            return Ok(Actor {
                name: lease.actor.clone(),
                work_session_id: on_own_task.then(|| lease.id.clone()),
                lease: Some(lease),
            });
        }
        // A name of the scope's shape would let the scope edit that task's text as its own.
        if !self.tasks_scope && crate::harness_session::is_tasks_scope_actor(&t::clean_label(&self.named)) {
            bail!("{} names are for the tasks-scoped key; write under your own name", crate::harness_session::TASKS_SCOPE_ACTOR);
        }
        let name = if self.tasks_scope {
            crate::harness_session::TASKS_SCOPE_ACTOR.to_string()
        } else if self.named.is_empty() {
            "unknown".to_string()
        } else {
            self.named.clone()
        };
        Ok(Actor { name, work_session_id: None, lease: None })
    }
}

/// What starting a task without a lease costs under `mode`: nothing, a warning
/// for the reply, or a refusal. Only a move into in_progress is ever checked.
pub(super) fn lease_policy(mode: LeaseMode, entering_in_progress: bool, holds: bool, display_id: &str) -> Result<Option<String>> {
    if !entering_in_progress || holds || mode == LeaseMode::Off {
        return Ok(None);
    }
    let how = format!(
        "{display_id} has no work lease from you: call task_claim with the task and your name, \
         then pass the returned lease on your updates"
    );
    match mode {
        LeaseMode::Enforce => bail!(how),
        _ => Ok(Some(how)),
    }
}

pub(super) fn add_warning(value: &mut Value, text: String) {
    match value.get_mut("warnings").and_then(Value::as_array_mut) {
        Some(list) => list.push(json!(text)),
        None => value["warnings"] = json!([text]),
    }
}

fn lease_json(lease: &t::WorkSession) -> Value {
    serde_json::to_value(lease).unwrap_or(Value::Null)
}

pub(super) struct TaskClaimTool {
    pub(super) settings: TasksConfig,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskClaimTool {
    fn name(&self) -> &str {
        "task_claim"
    }
    fn description(&self) -> &str {
        "Start working on a task: takes a work lease, moves the task to in_progress and returns a \
         lease token. Do this before you work on a task and keep the token for the rest of the \
         session. While you hold it nobody else can claim the task, and your time on it is recorded. \
         Pass the token as `lease` on task_update, task_comment_add and task_create so your work is \
         attributed to you. Call task_heartbeat now and then while working, and task_release when you \
         stop. A task held by someone else is refused; use takeover only when that session is gone."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "Internal id or display id" },
                "actor": { "type": "string", "description": "Your name, for example your agent name. Shown on the task." },
                "harness": { "type": "string", "description": "The tool you run in, for example claude-code, codex or cursor" },
                "session_ref": { "type": "string", "description": "Your own session id, if you have one. Claiming again with the same actor and session_ref replaces your lease." },
                "host": { "type": "string", "description": "Machine name" },
                "cwd": { "type": "string", "description": "Working directory" },
                "branch": { "type": "string", "description": "Git branch you work on" },
                "takeover": { "type": "boolean", "description": "End another session's lease on this task", "default": false }
            },
            "required": ["task_id", "actor"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn search_hint(&self) -> Option<&str> {
        Some("claim a task, start work, take a lease, lock a task for this session")
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let task_id = arg_str(&args, "task_id");
        if task_id.is_empty() {
            bail!("task_id is required");
        }
        let (actor, harness, session_ref) = (arg_str(&args, "actor"), arg_str(&args, "harness"), arg_str(&args, "session_ref"));
        let (host, cwd, branch) = (arg_str(&args, "host"), arg_str(&args, "cwd"), arg_str(&args, "branch"));
        let takeover = args.get("takeover").and_then(Value::as_bool).unwrap_or(false);
        let settings = self.settings.clone();
        let ttl = ttl_secs(&settings);
        let claimed = self.db.with_conn(move |c| {
            let who = t::LeaseIdentity {
                actor: &actor,
                harness: &harness,
                external_session_ref: &session_ref,
                host: &host,
                cwd: &cwd,
                branch: &branch,
            };
            t::claim(c, &task_id, &who, ttl, takeover)
        })?;
        Ok(json!({
            "lease": claimed.token,
            "lease_id": claimed.session.id,
            "moved_to_in_progress": claimed.moved,
            "ttl_secs": settings.lease_ttl(),
            "heartbeat_every_secs": settings.lease_ttl() / HEARTBEATS_PER_TTL,
            "task": task_json_with_warnings(&claimed.task),
            "next": "Work on the task. Pass `lease` on your task_update and task_comment_add calls. \
                     Call task_heartbeat while you work and task_release with a status when you stop.",
        }))
    }
}

pub(super) struct TaskHeartbeatTool {
    pub(super) settings: TasksConfig,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskHeartbeatTool {
    fn name(&self) -> &str {
        "task_heartbeat"
    }
    fn description(&self) -> &str {
        "Tell HQ you are still working on a claimed task. A lease that goes quiet for longer than its \
         ttl ends at your last heartbeat, so a crashed session costs no phantom time. Call it every \
         few minutes while you work. If it says the lease ended, claim the task again."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "lease": { "type": "string", "description": "The lease token task_claim returned" } },
            "required": ["lease"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let token = arg_str(&args, "lease");
        if token.is_empty() {
            bail!("lease is required");
        }
        let settings = self.settings.clone();
        let ttl = ttl_secs(&settings);
        let lease = self.db.with_conn(move |c| t::heartbeat(c, &token, ttl))?;
        Ok(json!({ "ok": true, "lease": lease_json(&lease), "ttl_secs": settings.lease_ttl() }))
    }
}

pub(super) struct TaskReleaseTool {
    pub(super) settings: TasksConfig,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskReleaseTool {
    fn name(&self) -> &str {
        "task_release"
    }
    fn description(&self) -> &str {
        "Stop working on a claimed task and say where it stands. Give a status (ready_for_review when \
         the work is done and needs checking, blocked when you cannot go on, to_do to hand it back) \
         and a short summary of what you did and what is next. Do not use complete unless the work \
         is verified. Ends your lease and your time on the task."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "lease": { "type": "string", "description": "The lease token task_claim returned" },
                "status": { "type": "string", "enum": ["to_do", "in_progress", "blocked", "ready_for_review", "complete"], "description": "Where the task stands now. Omit to leave it unchanged." },
                "summary": { "type": "string", "description": "What you did and what is next, left on the task thread" }
            },
            "required": ["lease"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let token = arg_str(&args, "lease");
        if token.is_empty() {
            bail!("lease is required");
        }
        let status = opt_str(&args, "status");
        let asked_for_status = status.is_some();
        let summary = arg_str(&args, "summary");
        let ttl = ttl_secs(&self.settings);
        let done = self.db.with_conn(move |c| t::release(c, &token, status.as_deref(), &summary, ttl))?;
        let mut out = json!({
            "released": true,
            "status_applied": done.status_applied,
            "minutes": done.session.active_seconds / 60,
            "lease": lease_json(&done.session),
            "task": task_json_with_warnings(&done.task),
        });
        if asked_for_status && !done.status_applied {
            add_warning(
                &mut out,
                "Your lease had expired and the task has moved on, so its status was left as it is. \
                 Your summary is on the task thread."
                    .to_string(),
            );
        }
        Ok(out)
    }
}
