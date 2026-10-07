//! MCP tools over the harness session manager.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

use super::WatchingChat;
use hq_db::harness_sessions_registry as registry;
use crate::registry::HqTool;
use crate::util::{arg_str, arg_str_list};

pub struct HarnessSessionSpawnTool {
    vault_path: PathBuf,
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    pub(crate) family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

#[async_trait]
impl HqTool for HarnessSessionSpawnTool {
    fn name(&self) -> &str {
        "harness_session_spawn"
    }
    fn description(&self) -> &str {
        "Spawn a long-lived interactive session for a coding-agent harness (claude-code, cursor, opencode, pi, kimi, codex, qwen, antigravity, github-copilot) in Herdr, on this machine or a remote host such as the user's laptop (see herdr_hosts). Returns a session_id for status/logs/wait/send/stop/resume. If the agent stops at a dialog the result carries the screen and no prompt is typed. A configured profile name (herdr.harness_profiles, such as a named Claude account) is accepted as the harness and reported back as `harness`. Pass task_id to make an HQ task the durable record of the work: the supervisor then comments on it and moves its status as the session starts, finishes a turn, blocks or exits (never to complete; a person verifies that). Aliases: agy = antigravity."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "harness": { "type": "string", "description": format!("One of: {} (built-in harnesses plus any herdr.harness_profiles)", super::spec::known_harnesses().join(", ")) },
                "prompt": { "type": "string", "description": "Initial prompt typed into the session after it starts" },
                "host": { "type": "string", "description": "Herdr host to run on, from herdr_hosts (default: the configured default host)" },
                "cwd": { "type": "string", "description": "Project directory on that host. Required; a home or root directory is refused." },
                "label": { "type": "string", "description": "Short human label, e.g. 'auth-refactor'", "default": "" },
                "task_id": { "type": "string", "description": "HQ task (id or display id such as FR-053) this session works on. Must exist and not be complete. Its title and description become the goal when `goal` is omitted." },
                "goal": { "type": "string", "description": "What the session should accomplish. HQ drives only a session with a specific goal and definition of done; without them it observes." },
                "done_criteria": { "type": "string", "description": "Observable conditions that show the goal is met, e.g. a named test passing." },
                "drive": { "type": "boolean", "description": "Web chat only. Omit to follow the default (a session no chat watched starts with Drive on, when its goal and definition of done pass the drive gate); false starts the watch with Drive off, or stops driving a session this chat already drives." }
            },
            "required": ["harness"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_if_spawned(&args)?;
        let harness = arg_str(&args, "harness");
        if let Some(guest) = &self.family_guest {
            let canonical = |h: &str| if h == "antigravity" { "agy".to_string() } else { h.to_string() };
            let allowed = &guest.allowed_harnesses;
            if !allowed.is_empty() && !allowed.iter().any(|a| canonical(a) == canonical(&harness)) {
                anyhow::bail!(
                    "{} only has access to these coding-agent harnesses through me: {}. '{harness}' is not one of them, so it would need a separate request. Tell {} plainly which harnesses are available.",
                    guest.name,
                    allowed.join(", "),
                    guest.name
                );
            }
        }
        let prompt = arg_str(&args, "prompt");
        let label = arg_str(&args, "label");
        let host = arg_str(&args, "host");
        let task_id = arg_str(&args, "task_id");
        let (goal, done) = (arg_str(&args, "goal"), arg_str(&args, "done_criteria"));
        let cfg = hq_core::config::HqConfig::load()?;
        if self.chat.as_ref().is_some_and(|c| c.driver_turn) {
            anyhow::bail!("a driver turn cannot start sessions; tell the user what you need");
        }
        let origin = super::start_origin(self.chat.as_ref());
        super::check_origin_cap(&self.db, origin)?;
        let cwd = super::require_cwd_in(
            args.get("cwd").and_then(|v| v.as_str()),
            &cfg.herdr,
            super::is_handoff_scope(&args),
        )?;
        let host_handle = crate::herdr::host(Some(host.as_str()).filter(|h| !h.is_empty()))?;
        let report = super::spawn_on(
            &self.vault_path,
            &self.db,
            host_handle,
            super::SpawnRequest {
                host: Some(host.as_str()).filter(|h| !h.is_empty()),
                harness: &harness,
                prompt: Some(prompt.as_str()).filter(|p| !p.is_empty()),
                cwd: &cwd,
                label: &label,
                mission_id: Some(task_id.as_str()).filter(|t| !t.is_empty()),
                watch: self.chat.as_ref().map(|c| c.new_watch(&args)),
                parent: None,
                goal: super::GoalText {
                    goal: Some(goal.as_str()).filter(|g| !g.is_empty()),
                    done_criteria: Some(done.as_str()).filter(|d| !d.is_empty()),
                },
            },
        )
        .await?;
        super::tag_origin(&self.db, &report, origin);
        Ok(report)
    }
}

pub struct HarnessSessionListTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionListTool {
    fn name(&self) -> &str {
        "harness_session_list"
    }
    fn description(&self) -> &str {
        "List harness sessions (running and recent) with live Herdr status (idle, working, blocked, done) and the host each runs on. A session on an unreachable host reports reachable=false instead of a guess. Pass task_id to list every session launched for one HQ task."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "description": "Optional filter: running, exited, stopped, orphaned" },
                "task_id": { "type": "string", "description": "Only the sessions of this HQ task (id or display id); ignores status" }
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
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            if task.is_empty() {
                super::list(&db, status)
            } else {
                super::list_for_task(&db, &task)
            }
        })
        .await?
    }
}

pub struct HarnessSessionStatusTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionStatusTool {
    fn name(&self) -> &str {
        "harness_session_status"
    }
    fn description(&self) -> &str {
        "Status of one harness session: registry row plus the live Herdr agent status and whether its host is reachable."
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
        let caller = super::caller_session(&args).map(str::to_string);
        tokio::task::spawn_blocking(move || {
            // A launched agent may look at itself, its parent and its children.
            db.with_conn(|c| crate::a2a::check_session_access(c, caller.as_deref(), &id))?;
            super::status(&db, &id)
        })
        .await?
    }
}

pub struct HarnessSessionLogsTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionLogsTool {
    fn name(&self) -> &str {
        "harness_session_logs"
    }
    fn description(&self) -> &str {
        "Recent terminal output of a harness session: read live while it runs, from the last stored snapshot after it ends."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "lines": { "type": "integer", "description": "How many trailing lines (default 40)", "default": 40 }
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
        tokio::task::spawn_blocking(move || super::tail_log(&db, &id, lines.clamp(1, 500))).await?
    }
}

pub struct HarnessSessionSendTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionSendTool {
    fn name(&self) -> &str {
        "harness_session_send"
    }
    fn description(&self) -> &str {
        "Steer a running harness session. `text` is submitted as a prompt. If the session is blocked at a dialog the text is refused; read the output, then pass `keys` (logical names such as enter, esc, down, ctrl+c) to answer it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "text": { "type": "string", "description": "Prompt to submit" },
                "keys": { "type": "array", "items": { "type": "string" }, "description": "Logical keys to press instead of text, e.g. [\"down\", \"enter\"]" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        super::spawned_may_target(&args, &id)?;
        let text = arg_str(&args, "text");
        let keys = arg_str_list(&args, "keys");
        if text.is_empty() && keys.is_empty() {
            anyhow::bail!("pass `text` to submit a prompt or `keys` to press keys");
        }
        let chat = self.chat.clone();
        tokio::task::spawn_blocking(move || {
            if keys.is_empty() {
                super::send(&db, &id, &text, chat.as_ref())
            } else {
                super::send_keys(&db, &id, &keys, chat.as_ref())
            }
        })
        .await?
    }
}

pub struct HarnessSessionStopTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionStopTool {
    fn name(&self) -> &str {
        "harness_session_stop"
    }
    fn description(&self) -> &str {
        "Stop a harness session (closes its Herdr workspace on its host; resume token is kept). Fails if the host is unreachable."
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
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        tokio::task::spawn_blocking(move || super::stop(&db, &id)).await?
    }
}

pub struct HarnessSessionResumeTool {
    vault_path: PathBuf,
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionResumeTool {
    fn name(&self) -> &str {
        "harness_session_resume"
    }
    fn description(&self) -> &str {
        "Resume a stopped/exited harness session using its saved resume token or session dir. Falls back to a fresh session in the same cwd when the harness has no resume support."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "prompt": { "type": "string", "description": "Optional prompt typed after resuming" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_if_spawned(&args)?;
        let prompt = arg_str(&args, "prompt");
        let id = arg_str(&args, "session_id");
        let reserved = super::reserve_send(&self.db, &id, self.chat.as_ref(), super::SendKind::Text)?;
        let result = super::resume(
            &self.vault_path,
            &self.db,
            &id,
            Some(prompt.as_str()).filter(|p| !p.is_empty()),
        )
        .await;
        super::settle_send(&self.db, &id, self.chat.as_ref(), super::SendKind::Text, reserved, result.is_ok());
        result
    }
}

pub struct HarnessSessionWaitTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionWaitTool {
    fn name(&self) -> &str {
        "harness_session_wait"
    }
    fn description(&self) -> &str {
        "Block until a harness session settles: idle or done (ready for input, usually finished a turn) or blocked (waiting on a dialog). Returns settled=false if the timeout passes first. Use after harness_session_send instead of polling logs."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "description": "Seconds to wait, 3 to 300 (default 120)", "default": 120 },
                "until": { "type": "array", "items": { "type": "string", "enum": ["idle", "working", "blocked", "done"] }, "description": "States that end the wait (default: idle, done, blocked)" }
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
        let secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(120);
        let until: Vec<crate::herdr::AgentStatus> = args
            .get("until")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(crate::herdr::AgentStatus::parse)
                    .collect()
            })
            .unwrap_or_default();
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        let timeout = std::time::Duration::from_secs(secs.clamp(3, 300));
        tokio::task::spawn_blocking(move || super::wait(&db, &id, &until, timeout)).await?
    }
}

pub struct HarnessSessionLinkTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionLinkTool {
    fn name(&self) -> &str {
        "harness_session_link"
    }
    fn description(&self) -> &str {
        "Link an existing harness session to an HQ task, for a session launched without task_id. From then on the supervisor records its lifecycle on that task, as harness_session_spawn does with task_id. Linking a running session moves a to_do, blocked or ready_for_review task to in_progress."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "task_id": { "type": "string", "description": "HQ task id or display id such as FR-053. Must exist and not be complete." },
                "drive": { "type": "boolean", "description": "Web chat only. Omit to follow the default (a session no chat watched starts with Drive on); false starts the watch with Drive off, or stops driving a session this chat already drives." }
            },
            "required": ["session_id", "task_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_attach(self.chat.as_ref())?;
        let db = self.db.clone();
        let (id, task) = (arg_str(&args, "session_id"), arg_str(&args, "task_id"));
        if id.is_empty() || task.is_empty() {
            anyhow::bail!("session_id and task_id are required");
        }
        let chat = self.chat.clone();
        tokio::task::spawn_blocking(move || {
            super::link(&db, &id, &task, chat.as_ref().map(|c| c.new_watch(&args)))
        })
        .await?
    }
}

pub struct HarnessSessionWatchTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionWatchTool {
    fn name(&self) -> &str {
        "harness_session_watch"
    }
    fn description(&self) -> &str {
        "Have this web chat watch a harness session: its finished turns, blocks and exit post into this chat instead of Telegram, and it shows in the chat's Watching panel. A session no chat watched starts with Drive on (HQ answering it and approving its prompts toward its task) unless you pass drive=false; one this chat already watches keeps its Drive switch, and one taken from another chat starts with Drive off. Only the user turns Drive back on, with the switch in that panel; drive=false stops driving. Only available in a web chat."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
        "drive": { "type": "boolean", "description": "Web chat only. Omit to follow the default (a session no chat watched starts with Drive on); false starts the watch with Drive off, or stops driving a session this chat already drives." }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(chat) = self.chat.clone() else {
            anyhow::bail!(
                "harness_session_watch only works from a web chat; this session has no chat thread"
            );
        };
        // An explicit true could re-enable Drive the user switched off, so only
        // the default or the user's switch turns it on.
        if args.get("drive").and_then(Value::as_bool) == Some(true) {
            anyhow::bail!(
                "drive=true is not accepted: omit drive to watch with the default (Drive on for a session no chat watched), and the user turns Drive on for any other with the switch in this chat's Watching panel. The session is not watched yet"
            );
        }
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        tokio::task::spawn_blocking(move || super::watch(&db, &id, chat.new_watch(&args))).await?
    }
}

pub struct HarnessSessionGoalTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionGoalTool {
    fn name(&self) -> &str {
        "harness_session_goal"
    }
    fn description(&self) -> &str {
        "Record what a harness session is for (`goal`) and what would be observable when it is met (`done_criteria`), for a session launched without them or whose aim changed. HQ drives a session only while both are specific enough to judge completion (not empty, not a placeholder, not just the goal repeated); editing them so they no longer are switches HQ to observing. A session exiting or going idle is never evidence the goal was met, and a person reviews the work. Every change is audited."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "goal": { "type": "string", "description": "What the session should accomplish, specific to the work" },
                "done_criteria": { "type": "string", "description": "Observable conditions that show the goal is met, e.g. a test passing or an endpoint's behaviour" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        if args.get(super::UNTRUSTED_TURN_ARG).and_then(Value::as_bool) == Some(true) {
            anyhow::bail!("this turn read untrusted content, so it cannot change a session's goal; ask the user");
        }
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        let (goal, done) = (arg_str(&args, "goal"), arg_str(&args, "done_criteria"));
        let actor = if self.chat.is_some() { registry::ACTOR_HQ } else { registry::ACTOR_MCP };
        tokio::task::spawn_blocking(move || {
            let text = super::GoalText {
                goal: Some(goal.as_str()).filter(|g| !g.is_empty()),
                done_criteria: Some(done.as_str()).filter(|d| !d.is_empty()),
            };
            super::set_goal(&db, &id, text, actor)
        })
        .await?
    }
}

pub struct HarnessSessionModeTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionModeTool {
    fn name(&self) -> &str {
        "harness_session_mode"
    }
    fn description(&self) -> &str {
        "Switch HQ between `drive` (answering the session and approving its prompts toward its goal) and `observe` (watching and reporting only) for a session this chat watches. `observe` always works and takes effect at once. `drive` needs a live session and a goal and definition of done that pass the drive gate (see harness_session_goal); otherwise HQ stays observing and the result says what is missing. Only HQ's steering changes: the agent is never paused or stopped by this, use harness_session_stop for that. Only available in a web chat."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "mode": { "type": "string", "enum": ["drive", "observe"] }
            },
            "required": ["session_id", "mode"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(chat) = self.chat.clone() else {
            anyhow::bail!("harness_session_mode only works from a web chat; this session has no chat thread");
        };
        let drive = match arg_str(&args, "mode").as_str() {
            "drive" => true,
            "observe" => false,
            other => anyhow::bail!("mode must be `drive` or `observe`, not '{other}'"),
        };
        if drive && (chat.driver_turn || chat.from_ask) {
            anyhow::bail!(
                "this turn cannot turn Drive on; only the user can, with the switch in the Watching panel"
            );
        }
        let untrusted = args.get(super::UNTRUSTED_TURN_ARG).and_then(Value::as_bool) == Some(true);
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        tokio::task::spawn_blocking(move || {
            let req = super::ModeRequest { thread: &chat.thread, drive, untrusted, actor: registry::ACTOR_HQ };
            super::set_mode(&db, &id, req)
        })
        .await?
    }
}

pub struct HarnessSessionAttachTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

#[async_trait]
impl HqTool for HarnessSessionAttachTool {
    fn name(&self) -> &str {
        "harness_session_attach"
    }
    fn description(&self) -> &str {
        "Attach this web chat to a coding agent Herdr already runs that HQ did not launch here, such as one the user started by hand (find it with herdr_agents). It becomes a tracked session this chat watches, observation-only: give it a goal and definition of done with harness_session_goal, then harness_session_mode to drive. A host that is unreachable or an agent that is gone is reported as such, never pretended attached. Only available in a web chat."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "Agent name from herdr_agents" },
                "host": { "type": "string", "description": "Herdr host from herdr_hosts (default: the configured default host)" }
            },
            "required": ["agent"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_attach(self.chat.as_ref())?;
        if let Some(guest) = &self.family_guest {
            anyhow::bail!("{} cannot attach to existing sessions; only the owner can", guest.name);
        }
        let Some(chat) = self.chat.clone() else {
            anyhow::bail!("harness_session_attach only works from a web chat; this session has no chat thread");
        };
        let target = arg_str(&args, "agent");
        if target.is_empty() {
            anyhow::bail!("`agent` is required; herdr_agents lists the names");
        }
        let host_name = arg_str(&args, "host");
        let host = crate::herdr::host(Some(host_name.as_str()).filter(|h| !h.is_empty()))?;
        let db = self.db.clone();
        crate::herdr::blocking(move || super::attach(&db, &host, &target, &chat.thread)).await?
    }
}

pub struct HarnessSessionHandoffTool {
    vault_path: PathBuf,
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

#[async_trait]
impl HqTool for HarnessSessionHandoffTool {
    fn name(&self) -> &str {
        "harness_session_handoff"
    }
    fn description(&self) -> &str {
        "Hand a piece of work to a coding agent in one call: files an HQ task (or reuses one), starts a session linked to it with the goal and acceptance criteria, and opens a web chat thread that owns the session so its finished turns, blocks and Drive show up there instead of the relay. Returns the task id and display id, session id, thread id and web links (/chat?thread=<id>, /tasks?task=<id>). Idempotent: the same external_id returns the same task, and a task that already has a live session gets no second one. If the host is unreachable or the agent stops at a dialog the result says so (blocked_at_dialog, prompt not typed) and nothing is claimed as running."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Task title. Required unless task_id names an existing task." },
                "description": { "type": "string", "description": "What the work is, for the task and the agent" },
                "acceptance": { "type": "string", "description": "Observable conditions that show the work is done (also accepted as done_criteria). HQ drives the session only when these are specific." },
                "done_criteria": { "type": "string", "description": "Alias of acceptance" },
                "harness": { "type": "string", "description": format!("One of: {}", super::spec::known_harnesses().join(", ")) },
                "cwd": { "type": "string", "description": "Project directory on that host. Required; a home or root directory is refused, as is anything matching herdr.spawn_cwd_deny." },
                "host": { "type": "string", "description": "Herdr host from herdr_hosts (default: the configured default host, normally local)" },
                "external_id": { "type": "string", "description": "Idempotency key for the task, unique per space. Not combinable with task_id." },
                "task_id": { "type": "string", "description": "Work on this existing HQ task (id or display id) instead of filing one. Must not be complete." },
                "prompt": { "type": "string", "description": "First prompt typed into the session. Default: the task title, description and acceptance criteria." },
                "space_id": { "type": "string", "description": "Space slug to file a new task in (default personal)" },
                "initiative": { "type": "string", "description": "Initiative name to file a new task under (default Inbox)" },
                "drive": { "type": "boolean", "description": "false starts the thread watching without HQ driving the session. Omit to follow herdr.drive_new_watches." }
            },
            "required": ["harness", "cwd"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_if_spawned(&args)?;
        if self.chat.as_ref().is_some_and(|c| c.driver_turn) {
            anyhow::bail!("a driver turn cannot start sessions; tell the user what you need");
        }
        if let Some(guest) = &self.family_guest {
            anyhow::bail!("{} cannot hand work to coding agents; only the owner can", guest.name);
        }
        let origin = super::start_origin(self.chat.as_ref());
        super::check_origin_cap(&self.db, origin)?;
        let cfg = hq_core::config::HqConfig::load()?;
        super::require_cwd_in(
            args.get("cwd").and_then(Value::as_str),
            &cfg.herdr,
            super::is_handoff_scope(&args),
        )?;
        let host_name = arg_str(&args, "host");
        let host = crate::herdr::host(Some(host_name.as_str()).filter(|h| !h.is_empty()))?;
        let owned = |key: &str| arg_str(&args, key);
        let acceptance = [owned("acceptance"), owned("done_criteria")]
            .into_iter()
            .find(|a| !a.trim().is_empty())
            .unwrap_or_default();
        let report = super::handoff::handoff(
            &self.vault_path,
            &self.db,
            host,
            super::handoff::HandoffRequest {
                title: owned("title"),
                description: owned("description"),
                acceptance,
                harness: owned("harness"),
                cwd: owned("cwd"),
                external_id: owned("external_id"),
                task_id: owned("task_id"),
                prompt: owned("prompt"),
                space_id: owned("space_id"),
                initiative: owned("initiative"),
                // A handoff from an hq_ask reply or over the handoff key never starts out driving,
                // so a handoff-key holder cannot fill the driven-session cap and starve the owner.
                drive_new: handoff_drives_new(cfg.herdr.drive_new_watches, &args, self.chat.as_ref()),
                drive_opted_out: args.get("drive").and_then(Value::as_bool) == Some(false),
            },
        )
        .await?;
        super::tag_origin(&self.db, &report, origin);
        Ok(report)
    }
}

/// Whether a handoff may start its session driven: not from the handoff key (so its holder cannot
/// fill the driven-session cap) and not from an `hq_ask` reply.
pub(super) fn handoff_drives_new(default_on: bool, args: &Value, chat: Option<&WatchingChat>) -> bool {
    default_on && !super::is_handoff_scope(args) && !chat.is_some_and(|c| c.from_ask)
}

/// `chat` is the web chat of the calling session, if any.
pub fn create_harness_session_tools(
    vault_path: PathBuf,
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
) -> Vec<Box<dyn HqTool>> {
    let family_guest_for_attach = family_guest.clone();
    let family_guest_for_handoff = family_guest.clone();
    vec![
        Box::new(HarnessSessionHandoffTool {
            vault_path: vault_path.clone(),
            db: db.clone(),
            chat: chat.clone(),
            family_guest: family_guest_for_handoff,
        }),
        Box::new(HarnessSessionSpawnTool {
            vault_path: vault_path.clone(),
            db: db.clone(),
            chat: chat.clone(),
            family_guest,
        }),
        Box::new(HarnessSessionListTool { db: db.clone() }),
        Box::new(HarnessSessionStatusTool { db: db.clone() }),
        Box::new(HarnessSessionLogsTool { db: db.clone() }),
        Box::new(HarnessSessionSendTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionWaitTool { db: db.clone() }),
        Box::new(HarnessSessionStopTool { db: db.clone() }),
        Box::new(HarnessSessionLinkTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionWatchTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionGoalTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionModeTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionAttachTool {
            db: db.clone(),
            chat: chat.clone(),
            family_guest: family_guest_for_attach,
        }),
        Box::new(HarnessSessionResumeTool {
            vault_path,
            db,
            chat,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family_guest::FamilyGuestContext;

    #[tokio::test]
    async fn family_guest_rejects_non_agy_harness() {
        let db = Arc::new(Database::open_memory().unwrap());
        let tool = HarnessSessionSpawnTool {
            vault_path: PathBuf::from("/tmp"),
            db,
            chat: None,
            family_guest: Some(FamilyGuestContext {
                name: "Bob".into(),
                origin_channel_id: 12345,
                owner_name: "Owner".into(),
                allowed_harnesses: vec!["agy".into()],
            }),
        };

        let err = tool
            .execute(json!({ "harness": "claude-code" }))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("Bob only has access to these coding-agent harnesses through me: agy")
        );
        assert!(err.to_string().contains("'claude-code' is not one of them"));

        let err_cursor = tool
            .execute(json!({ "harness": "cursor" }))
            .await
            .unwrap_err();
        assert!(err_cursor.to_string().contains("'cursor' is not one of them"));
    }

    #[tokio::test]
    async fn non_family_guest_allows_any_harness() {
        let db = Arc::new(Database::open_memory().unwrap());
        let tool = HarnessSessionSpawnTool {
            vault_path: PathBuf::from("/tmp"),
            db,
            chat: None,
            family_guest: None,
        };

        let result = tool.execute(json!({ "harness": "claude-code" })).await;
        if let Err(e) = result {
            assert!(!e.to_string().contains("only has access to the Agy"));
        }
    }
}
