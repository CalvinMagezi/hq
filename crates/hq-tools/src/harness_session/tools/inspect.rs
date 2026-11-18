//! Reading sessions: list, status, logs, pending prompts and wait.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use serde_json::{Value, json};
use std::sync::Arc;

use super::MAX_FAN_IN;
use super::registry;
use crate::registry::HqTool;
use crate::util::{arg_str, arg_str_list};

pub struct HarnessSessionListTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionListTool {
    fn name(&self) -> &str {
        "harness_session_list"
    }
    fn description(&self) -> &str {
        "List harness sessions (running and recent) with live the host status (idle, working, blocked, done) and the host each runs on. A session on an unreachable host reports reachable=false instead of a guess. Pass task_id to list every session launched for one HQ task."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "description": "Optional filter: running, exited, stopped, orphaned" },
                "task_id": { "type": "string", "description": "Only the sessions of this HQ task (id or display id); ignores status" },
                "worktree_status": { "type": "boolean", "description": "Default false. When true, each running session with a worktree also reports worktree_status (changed files, ahead and behind its start), one host call each." }
            }
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let status = args
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let task = arg_str(&args, "task_id");
        let with_worktrees = args
            .get("worktree_status")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            // A claude.ai connection sees only the sessions it may drive; the owner's stay theirs.
            let keep = |r: &registry::HarnessSessionRow| {
                crate::harness_session::connector_may_drive(&db, &args, &r.id).is_ok()
            };
            if task.is_empty() {
                crate::harness_session::list_with(&db, status, with_worktrees, &keep)
            } else {
                crate::harness_session::list_for_task(&db, &task, &keep)
            }
        })
        .await?
    }
}

pub struct HarnessSessionStatusTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionStatusTool {
    fn name(&self) -> &str {
        "harness_session_status"
    }
    fn description(&self) -> &str {
        "Status of one harness session: registry row plus the live host agent status and whether its host is reachable. A session with a live worktree also reports worktree_status (changed files, ahead and behind its start)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        // Its row holds the goal, folder, branch and resume token, which a connection reads
        // only for sessions it may drive.
        crate::harness_session::connector_may_drive(&db, &args, &id)?;
        let caller = crate::harness_session::caller_session(&args).map(str::to_string);
        tokio::task::spawn_blocking(move || {
            // A launched agent may look at itself, its parent and its children.
            db.with_conn(|c| {
                crate::session_policy::check_session_access(c, caller.as_deref(), &id)
            })?;
            crate::harness_session::status(&db, &id)
        })
        .await?
    }
}

pub struct HarnessSessionLogsTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionLogsTool {
    fn name(&self) -> &str {
        "harness_session_logs"
    }
    fn description(&self) -> &str {
        "Recent terminal output of a harness session: read live while it runs, from the last stored snapshot after it ends. To follow a session without reading the same lines again, pass `since` (\"\" on the first call, then the `next` each reply returns): the reply holds only the lines that scrolled off the screen since then, plus the current screen. An agent that redraws a full-screen view in place (Copilot) scrolls nothing off, so only its screen changes."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "lines": { "type": "integer", "description": "How many trailing lines (default 40); with `since`, the most new lines to return", "default": 40 },
                "since": { "type": "string", "description": "A cursor from an earlier reply's `next`, or \"\" to start one" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(40) as usize;
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        crate::harness_session::connector_may_drive(&db, &args, &id)?;
        let caller = crate::harness_session::caller_session(&args).map(str::to_string);
        let since = args
            .get("since")
            .and_then(Value::as_str)
            .map(str::to_string);
        tokio::task::spawn_blocking(move || {
            // A launched agent may read the sessions of its own team.
            db.with_conn(|c| {
                crate::session_policy::check_session_access(c, caller.as_deref(), &id)
            })?;
            match since {
                Some(cursor) => {
                    crate::harness_session::logs_since(&db, &id, &cursor, lines.clamp(1, 500))
                }
                None => crate::harness_session::tail_log(&db, &id, lines.clamp(1, 500)),
            }
        })
        .await?
    }
}

pub struct HarnessSessionPendingTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionPendingTool {
    fn name(&self) -> &str {
        "harness_session_pending"
    }
    fn description(&self) -> &str {
        "The dialog a blocked harness session waits at, as data: its number (`seq`), kind (command, edit, path, question, trust, other), question, the command or path it is about, the session's folder, every option, and the dialog's text. An option with `remember: true` would let the agent do more from now on without asking; `typed: true` opens a text field. Answer with harness_session_answer. `pending` is null when the session waits at no dialog HQ can read."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        crate::harness_session::connector_may_drive(&db, &args, &id)?;
        let caller = crate::harness_session::caller_session(&args).map(str::to_string);
        tokio::task::spawn_blocking(move || {
            db.with_conn(|c| {
                crate::session_policy::check_session_access(c, caller.as_deref(), &id)
            })?;
            crate::harness_session::answer::pending(&db, &id)
        })
        .await?
    }
}

pub struct HarnessSessionWaitTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionWaitTool {
    fn name(&self) -> &str {
        "harness_session_wait"
    }
    fn description(&self) -> &str {
        "Block until a harness session settles: idle or done (ready for input, usually finished a turn) or blocked (waiting on a dialog). Returns settled=false if the timeout passes first. Use after harness_session_send instead of polling logs. Pass `session_ids` (up to 10) instead of `session_id` to wait for several at once against one deadline: the reply has one report per session, each with its own turn result, and settled is true only when every one settled."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "session_ids": { "type": "array", "items": { "type": "string" }, "maxItems": 10, "description": "Wait for all of these instead of one session_id" },
                "timeout_secs": { "type": "integer", "description": "Seconds to wait, 3 to 45 (default 40). To wait longer, call again.", "default": 40 },
                "until": { "type": "array", "items": { "type": "string", "enum": ["idle", "working", "blocked", "done"] }, "description": "States that end the wait (default: idle, done, blocked)" }
            }
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(crate::harness_session::DEFAULT_WAIT.as_secs());
        let until: Vec<crate::agent_host::AgentStatus> = args
            .get("until")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(crate::agent_host::AgentStatus::parse)
                    .collect()
            })
            .unwrap_or_default();
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        let many = arg_str_list(&args, "session_ids");
        let timeout = std::time::Duration::from_secs(
            secs.clamp(3, crate::harness_session::MAX_WAIT.as_secs()),
        );
        let caller = crate::harness_session::caller_session(&args).map(str::to_string);
        if many.is_empty() && id.is_empty() {
            anyhow::bail!("pass `session_id`, or `session_ids` to wait for several");
        }
        if !many.is_empty() {
            if many.len() > MAX_FAN_IN {
                anyhow::bail!("wait for at most {MAX_FAN_IN} sessions at once");
            }
            for one in &many {
                crate::harness_session::connector_may_drive(&db, &args, one)?;
            }
            return tokio::task::spawn_blocking(move || {
                let may_reach = |one: &str| {
                    db.with_conn(|c| {
                        crate::session_policy::check_session_access(c, caller.as_deref(), one)
                    })
                };
                Ok(crate::harness_session::wait_all(
                    &db, &many, &until, timeout, &may_reach,
                ))
            })
            .await?;
        }
        crate::harness_session::connector_may_drive(&db, &args, &id)?;
        tokio::task::spawn_blocking(move || {
            db.with_conn(|c| {
                crate::session_policy::check_session_access(c, caller.as_deref(), &id)
            })?;
            crate::harness_session::wait(&db, &id, &until, timeout)
        })
        .await?
    }
}
