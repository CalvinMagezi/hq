//! Driving sessions: send, answer, stop, resume, link, watch, goal, mode and attach.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use serde_json::{Value, json};
use std::sync::Arc;

use super::registry;
use crate::harness_session::WatchingChat;
use crate::registry::HqTool;
use crate::util::{arg_str, arg_str_list};

pub struct HarnessSessionSendTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionSendTool {
    fn name(&self) -> &str {
        "harness_session_send"
    }
    fn description(&self) -> &str {
        "Steer a running harness session. `text` is submitted as a prompt. If the session is blocked at a dialog the text is refused: read the dialog with harness_session_pending and answer it with harness_session_answer, or pass `keys` (logical names such as enter, esc, down, ctrl+c) when HQ cannot read it. Unless the owner typed this turn, keys at a dialog are refused except esc and ctrl+c at one HQ cannot read."
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
        crate::harness_session::spawned_may_target(&args, &id)?;
        crate::harness_session::connector_may_drive(&db, &args, &id)?;
        crate::harness_session::untrusted_may_target(&db, &args, self.chat.as_ref(), &id)?;
        let text = arg_str(&args, "text");
        let keys = arg_str_list(&args, "keys");
        if text.is_empty() && keys.is_empty() {
            anyhow::bail!("pass `text` to submit a prompt or `keys` to press keys");
        }
        let chat = self.chat.clone();
        let who = crate::harness_session::answer::Answerer::of(&args, chat.as_ref());
        tokio::task::spawn_blocking(move || {
            if keys.is_empty() {
                crate::harness_session::send(&db, &id, &text, chat.as_ref(), who)
            } else {
                crate::harness_session::send_keys(&db, &id, &keys, chat.as_ref(), who)
            }
        })
        .await?
    }
}

pub struct HarnessSessionAnswerTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionAnswerTool {
    fn name(&self) -> &str {
        "harness_session_answer"
    }
    fn description(&self) -> &str {
        "Answer the dialog a harness session waits at by option, not by keys: pass the `seq` from harness_session_pending and an `option` id, or `decline: true` (with an optional `message` the agent gets as its next prompt once the dialog is gone). Refused if the session is at a different dialog by now: read it again. Options with remember: true are the owner's call; Drive, agent sessions and claude.ai connections are refused them. The reply says registered: false (not_registered) when the dialog is still open afterwards."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "seq": { "type": "integer", "description": "The dialog's number from harness_session_pending" },
                "option": { "type": "integer", "description": "The id of the option to pick" },
                "decline": { "type": "boolean", "description": "Dismiss the dialog without picking an option" },
                "message": { "type": "string", "description": "With decline: what to tell the agent instead, sent once it is back at its prompt" }
            },
            "required": ["session_id", "seq"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        use crate::harness_session::answer::{Answerer, Choice};
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        crate::harness_session::spawned_may_target(&args, &id)?;
        crate::harness_session::connector_may_drive(&db, &args, &id)?;
        crate::harness_session::untrusted_may_target(&db, &args, self.chat.as_ref(), &id)?;
        let seq = args.get("seq").and_then(Value::as_u64).ok_or_else(|| {
            anyhow::anyhow!("pass `seq`, the dialog's number from harness_session_pending")
        })?;
        let option = args.get("option").and_then(Value::as_u64);
        let decline = args.get("decline").and_then(Value::as_bool) == Some(true);
        let message = Some(arg_str(&args, "message")).filter(|m| !m.trim().is_empty());
        let choice = match (option, decline) {
            (Some(o), false) => {
                Choice::Option(u32::try_from(o).map_err(|_| anyhow::anyhow!("no option {o}"))?)
            }
            (None, true) => Choice::Decline(message),
            _ => anyhow::bail!("pass either `option` or `decline: true`"),
        };
        let who = Answerer::of(&args, self.chat.as_ref());
        let chat = self.chat.clone();
        tokio::task::spawn_blocking(move || {
            crate::harness_session::answer::answer(&db, &id, seq, choice, who, chat.as_ref())
        })
        .await?
    }
}

pub struct HarnessSessionStopTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionStopTool {
    fn name(&self) -> &str {
        "harness_session_stop"
    }
    fn description(&self) -> &str {
        "Stop a harness session (closes its the host workspace on its host; resume token is kept). Fails if the host is unreachable."
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
        crate::harness_session::connector_may_drive(&db, &args, &id)?;
        tokio::task::spawn_blocking(move || crate::harness_session::stop(&db, &id)).await?
    }
}

pub struct HarnessSessionResumeTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
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
        crate::harness_session::refuse_if_spawned(&args)?;
        let prompt = arg_str(&args, "prompt");
        let id = arg_str(&args, "session_id");
        crate::harness_session::connector_may_drive(&self.db, &args, &id)?;
        let reserved = crate::harness_session::reserve_send(
            &self.db,
            &id,
            self.chat.as_ref(),
            crate::harness_session::SendKind::Text,
        )?;
        let result = crate::harness_session::resume(
            &self.db,
            &id,
            Some(prompt.as_str()).filter(|p| !p.is_empty()),
        )
        .await;
        crate::harness_session::settle_send(
            &self.db,
            &id,
            self.chat.as_ref(),
            crate::harness_session::SendKind::Text,
            reserved,
            result.is_ok(),
        );
        result
    }
}

pub struct HarnessSessionLinkTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionLinkTool {
    fn name(&self) -> &str {
        "harness_session_link"
    }
    fn description(&self) -> &str {
        "Link an existing harness session to an HQ task, for a session launched without task_id, or move a session on to the next task of a plan. From then on the supervisor records its lifecycle on that task, as harness_session_spawn does with task_id. Linking a running session moves a to_do, blocked or ready_for_review task to in_progress. When the session was on another task, its goal moves to the new one (Drive reads that task, not the launch brief) and the reply has next_message, a fixed line naming the task; continue: true sends it to the session."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "task_id": { "type": "string", "description": "HQ task id or display id such as FR-053. Must exist and not be complete." },
                "drive": { "type": "boolean", "description": "Web chat only. Omit to follow the default (a session no chat watched starts with Drive on); false starts the watch with Drive off, or stops driving a session this chat already drives." },
                "continue": { "type": "boolean", "description": "When the session moves from another task, send it \"Continue with <task id>: <title>. Its acceptance is on the task.\" (never the task's description)" }
            },
            "required": ["session_id", "task_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        crate::harness_session::refuse_attach(self.chat.as_ref())?;
        let db = self.db.clone();
        let (id, task) = (arg_str(&args, "session_id"), arg_str(&args, "task_id"));
        if id.is_empty() || task.is_empty() {
            anyhow::bail!("session_id and task_id are required");
        }
        crate::harness_session::connector_may_drive(&db, &args, &id)?;
        let chat = self.chat.clone();
        let tell = args.get("continue").and_then(Value::as_bool) == Some(true);
        tokio::task::spawn_blocking(move || {
            crate::harness_session::link_with(
                &db,
                &id,
                &task,
                chat.as_ref().map(|c| c.new_watch(&args)),
                tell,
                chat.as_ref(),
            )
        })
        .await?
    }
}

pub struct HarnessSessionWatchTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
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
        tokio::task::spawn_blocking(move || {
            crate::harness_session::watch(&db, &id, chat.new_watch(&args))
        })
        .await?
    }
}

pub struct HarnessSessionGoalTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
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
        if args
            .get(crate::harness_session::UNTRUSTED_TURN_ARG)
            .and_then(Value::as_bool)
            == Some(true)
        {
            anyhow::bail!(
                "this turn read untrusted content, so it cannot change a session's goal; ask the user"
            );
        }
        crate::harness_session::connector_may_drive(
            &self.db,
            &args,
            &arg_str(&args, "session_id"),
        )?;
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        let (goal, done) = (arg_str(&args, "goal"), arg_str(&args, "done_criteria"));
        let actor = if self.chat.is_some() {
            registry::ACTOR_HQ
        } else {
            registry::ACTOR_MCP
        };
        tokio::task::spawn_blocking(move || {
            let text = crate::harness_session::GoalText {
                goal: Some(goal.as_str()).filter(|g| !g.is_empty()),
                done_criteria: Some(done.as_str()).filter(|d| !d.is_empty()),
            };
            crate::harness_session::set_goal(&db, &id, text, actor)
        })
        .await?
    }
}

pub struct HarnessSessionModeTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
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
            anyhow::bail!(
                "harness_session_mode only works from a web chat; this session has no chat thread"
            );
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
        let untrusted = chat.untrusted_blocks_drive(&args);
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        tokio::task::spawn_blocking(move || {
            let req = crate::harness_session::ModeRequest {
                thread: &chat.thread,
                drive,
                untrusted,
                actor: registry::ACTOR_HQ,
            };
            crate::harness_session::set_mode(&db, &id, req)
        })
        .await?
    }
}

pub struct HarnessSessionAttachTool {
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
    pub(super) family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

#[async_trait]
impl HqTool for HarnessSessionAttachTool {
    fn name(&self) -> &str {
        "harness_session_attach"
    }
    fn description(&self) -> &str {
        "Attach this web chat to a coding agent the host already runs that HQ did not launch here, such as one the user started by hand (find it with host_agents). It becomes a tracked session this chat watches, observation-only: give it a goal and definition of done with harness_session_goal, then harness_session_mode to drive. A host that is unreachable or an agent that is gone is reported as such, never pretended attached. Only available in a web chat."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "Agent name from host_agents" },
                "host": { "type": "string", "description": "host from host_list (default: the configured default host)" }
            },
            "required": ["agent"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        crate::harness_session::refuse_attach(self.chat.as_ref())?;
        if let Some(guest) = &self.family_guest {
            anyhow::bail!(
                "{} cannot attach to existing sessions; only the owner can",
                guest.name
            );
        }
        let Some(chat) = self.chat.clone() else {
            anyhow::bail!(
                "harness_session_attach only works from a web chat; this session has no chat thread"
            );
        };
        let target = arg_str(&args, "agent");
        if target.is_empty() {
            anyhow::bail!("`agent` is required; host_agents lists the names");
        }
        let host_name = arg_str(&args, "host");
        let host = crate::agent_host::host(Some(host_name.as_str()).filter(|h| !h.is_empty()))?;
        let db = self.db.clone();
        crate::agent_host::blocking(move || {
            crate::harness_session::attach(&db, &host, &target, &chat.thread)
        })
        .await?
    }
}
