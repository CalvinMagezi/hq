//! Agent tools over the `background_turns` registry: detached relay turns and
//! recurring watches.

use crate::registry::{HqTool, ToolPolicy, truncate_note};
use crate::util::arg_str;
use anyhow::Result;
use async_trait::async_trait;
use hq_core::identity::{RequestIdentity, RequestSource};
use hq_db::Database;
use hq_db::background_turns::{
    self, BackgroundTurnRow, DEFAULT_WATCH_EXPIRY_HOURS, MAX_WATCH_EXPIRY_HOURS,
    MAX_WATCH_INTERVAL_MINS, MIN_WATCH_INTERVAL_MINS,
};
use serde_json::{Value, json};
use std::sync::Arc;

const RESULT_EXCERPT_CHARS: usize = 600;

const NO_ORIGIN: &str = "watches need a chat origin: this session was not started from a Telegram or Discord chat, so there is nowhere to deliver firings";

pub fn create_background_turn_tools(db: Arc<Database>) -> Vec<Box<dyn HqTool>> {
    vec![Box::new(BackgroundTurnStatusTool { db })]
}

/// The chat a watch reports to, taken from the session's request identity.
#[derive(Debug, Clone)]
pub struct WatchOrigin {
    pub platform: &'static str,
    pub chat_id: String,
    pub user_id: String,
    /// A restricted audience (a guest): it may not tie a watch to a task, since the watch writes onto it.
    pub restricted: bool,
}

impl WatchOrigin {
    /// `None` for surfaces the watch scheduler cannot deliver to (CLI, web, proxy).
    pub fn from_identity(identity: &RequestIdentity) -> Option<Self> {
        let (platform, chat_id) = match &identity.source {
            RequestSource::Telegram { chat_id } => ("telegram", chat_id.to_string()),
            RequestSource::Discord { channel_id } => ("discord", channel_id.to_string()),
            RequestSource::ProxyApi { .. } | RequestSource::Web { .. } | RequestSource::LocalCli => {
                return None;
            }
        };
        Some(Self {
            platform,
            chat_id,
            user_id: identity.user_id.clone(),
            restricted: !identity.scope.is_unrestricted(),
        })
    }

    fn owns(&self, row: &BackgroundTurnRow) -> bool {
        row.platform == self.platform && row.chat_id == self.chat_id
    }
}

/// `watch_create`, `watch_list` and `watch_cancel`, bound to this session's chat.
pub fn create_watch_tools(db: Arc<Database>, origin: Option<WatchOrigin>) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(WatchCreateTool {
            db: db.clone(),
            origin: origin.clone(),
        }),
        Box::new(WatchListTool {
            db: db.clone(),
            origin: origin.clone(),
        }),
        Box::new(WatchCancelTool { db, origin }),
    ]
}

fn require_origin(origin: &Option<WatchOrigin>) -> Result<&WatchOrigin> {
    origin.as_ref().ok_or_else(|| anyhow::anyhow!(NO_ORIGIN))
}

fn watch_summary(row: &BackgroundTurnRow) -> Value {
    let interval = row.watch_interval_secs.unwrap_or_default();
    let next_run = row.watch_last_fired.unwrap_or(row.created_at) + interval;
    json!({
        "id": row.id,
        "short_ref": background_turns::short_ref(&row.id),
        "status": row.status,
        "interval_secs": interval,
        "next_run_at": epoch_to_rfc3339(next_run),
        "watch_until": row.watch_until.and_then(epoch_to_rfc3339),
        "prompt": truncate_note(&row.prompt, RESULT_EXCERPT_CHARS),
        "last_result": row.result_text.as_deref().map(|t| truncate_note(t, RESULT_EXCERPT_CHARS)),
    })
}

fn epoch_to_rfc3339(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0).map(|t| t.to_rfc3339())
}

fn row_summary(row: &BackgroundTurnRow) -> Value {
    json!({
        "id": row.id,
        "short_ref": background_turns::short_ref(&row.id),
        "kind": row.kind,
        "status": row.status,
        "platform": row.platform,
        "started_at": epoch_to_rfc3339(row.created_at),
        "updated_at": row
            .completed_at
            .or(row.watch_last_fired)
            .and_then(epoch_to_rfc3339),
        "blocked_on": row.blocked_on,
        "prompt": truncate_note(&row.prompt, RESULT_EXCERPT_CHARS),
        "result_excerpt": row.result_text.as_deref().map(|t| truncate_note(t, RESULT_EXCERPT_CHARS)),
    })
}

pub struct BackgroundTurnStatusTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for BackgroundTurnStatusTool {
    fn name(&self) -> &str {
        "background_turn_status"
    }
    fn description(&self) -> &str {
        "Look up a background turn or watch by its short ref (the 8 characters shown in chat, e.g. from \"Parked as turn `1a2b3c4d`\") or full id. Returns status (running, completed, failed, interrupted, cancelled), kind, start and last-update times, and an excerpt of the result."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Short ref (at least 6 characters) or full id." }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> &str {
        "scheduling"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let id = arg_str(&args, "id");
        let row = self
            .db
            .with_conn(|c| background_turns::get_by_prefix(c, id.trim()))?
            .ok_or_else(|| anyhow::anyhow!("no background turn matches `{id}`"))?;
        Ok(row_summary(&row))
    }
}

pub struct WatchCreateTool {
    db: Arc<Database>,
    origin: Option<WatchOrigin>,
}

#[async_trait]
impl HqTool for WatchCreateTool {
    fn name(&self) -> &str {
        "watch_create"
    }
    fn description(&self) -> &str {
        "Re-run a prompt in this chat on an interval until a condition holds or the watch expires, e.g. \"check every 10 minutes whether the deploy is green\". Each firing is a full agent turn whose reply is delivered here only when it changes; a firing that confirms the condition replies with WATCH_DONE, which stops the watch. Interval is clamped to 5-1440 minutes, duration to 1-8760 hours (default 720). Only works from a Telegram or Discord chat."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "interval_minutes": { "type": "integer", "description": "Minutes between firings, clamped to 5-1440." },
                "duration_hours": { "type": "integer", "description": "Hours until the watch expires, clamped to 1-8760. Default 720 (30 days)." },
                "prompt": { "type": "string", "description": "What each firing should check, including the condition that means done." },
                "task_id": { "type": "string", "description": "A task this watch follows (id or display id). Each changed result is noted on it, and the watch stops by itself when the task is complete or archived." }
            },
            "required": ["interval_minutes", "prompt"]
        })
    }
    fn category(&self) -> &str {
        "scheduling"
    }
    fn is_read_only(&self) -> bool {
        false
    }
    fn is_destructive(&self) -> bool {
        false
    }
    // A watch firing runs unattended; letting it create watches would let one
    // watch multiply itself.
    fn requires_live_user_turn(&self) -> bool {
        true
    }
    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let origin = require_origin(&self.origin)?;
        let prompt = arg_str(&args, "prompt");
        if prompt.trim().is_empty() {
            anyhow::bail!("prompt is required");
        }
        let interval_mins = args
            .get("interval_minutes")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("interval_minutes is required"))?
            .clamp(MIN_WATCH_INTERVAL_MINS, MAX_WATCH_INTERVAL_MINS);
        let hours = args
            .get("duration_hours")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_WATCH_EXPIRY_HOURS)
            .clamp(1, MAX_WATCH_EXPIRY_HOURS);
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        let task_ref = arg_str(&args, "task_id");
        if !task_ref.trim().is_empty() && origin.restricted {
            anyhow::bail!("this chat cannot attach a watch to a task");
        }
        let (row, followed) = self.db.with_conn(|c| {
            hq_db::tasks::in_write_tx(c, |c| {
                background_turns::insert_watch(
                    c,
                    &id,
                    origin.platform,
                    &origin.chat_id,
                    None,
                    Some(&origin.user_id),
                    prompt.trim(),
                    now,
                    interval_mins * 60,
                    Some(now + hours * 3600),
                )?;
                // An unknown or finished task refuses the whole watch, so none is left running unlinked.
                let followed = (!task_ref.trim().is_empty())
                    .then(|| background_turns::set_watch_task(c, &id, task_ref.trim()))
                    .transpose()?;
                Ok((background_turns::get(c, &id)?, followed))
            })
        })?;
        let row = row.ok_or_else(|| anyhow::anyhow!("watch `{id}` was not found after insert"))?;
        let mut summary = watch_summary(&row);
        if let Some(task) = followed {
            summary["task"] = json!({ "id": task.id, "display_id": task.display_id, "title": task.title });
        }
        Ok(summary)
    }
}

pub struct WatchListTool {
    db: Arc<Database>,
    origin: Option<WatchOrigin>,
}

#[async_trait]
impl HqTool for WatchListTool {
    fn name(&self) -> &str {
        "watch_list"
    }
    fn description(&self) -> &str {
        "List the running watches in this chat with their short ref, interval, next run, expiry and last result."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> &str {
        "scheduling"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }
    async fn execute(&self, _args: Value) -> Result<Value> {
        let origin = require_origin(&self.origin)?;
        let rows = self.db.with_conn(background_turns::list_running)?;
        let watches: Vec<Value> = rows
            .iter()
            .filter(|r| r.kind == background_turns::KIND_WATCH && origin.owns(r))
            .map(watch_summary)
            .collect();
        Ok(json!({ "watches": watches }))
    }
}

pub struct WatchCancelTool {
    db: Arc<Database>,
    origin: Option<WatchOrigin>,
}

#[async_trait]
impl HqTool for WatchCancelTool {
    fn name(&self) -> &str {
        "watch_cancel"
    }
    fn description(&self) -> &str {
        "Stop a running watch in this chat by its short ref or full id."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Short ref (at least 6 characters) or full id." }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> &str {
        "scheduling"
    }
    fn is_read_only(&self) -> bool {
        false
    }
    fn is_destructive(&self) -> bool {
        false
    }
    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let origin = require_origin(&self.origin)?;
        let id = arg_str(&args, "id");
        let row = self
            .db
            .with_conn(|c| background_turns::get_by_prefix(c, id.trim().trim_matches('`')))?
            .ok_or_else(|| anyhow::anyhow!("no watch matches `{id}`"))?;
        if row.kind != background_turns::KIND_WATCH || !origin.owns(&row) {
            anyhow::bail!("`{id}` is not a watch in this chat");
        }
        if row.status != background_turns::STATUS_RUNNING {
            anyhow::bail!(
                "watch `{}` is already {}",
                background_turns::short_ref(&row.id),
                row.status
            );
        }
        let now = chrono::Utc::now().timestamp();
        self.db
            .with_conn(|c| background_turns::mark_cancelled(c, &row.id, now))?;
        Ok(
            json!({ "id": row.id, "short_ref": background_turns::short_ref(&row.id), "status": background_turns::STATUS_CANCELLED }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_resolves_a_short_ref() {
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|c| {
            background_turns::insert(
                c,
                "abcdef12-3456",
                "telegram",
                "1",
                None,
                None,
                "p",
                0,
                None,
            )?;
            background_turns::mark_completed(c, "abcdef12-3456", "all green", 60)
        })
        .unwrap();
        let tool = BackgroundTurnStatusTool { db };
        let out = tool.execute(json!({ "id": "abcdef12" })).await.unwrap();
        assert_eq!(out["status"], "completed");
        assert_eq!(out["short_ref"], "abcdef12");
        assert_eq!(out["result_excerpt"], "all green");
        assert!(tool.execute(json!({ "id": "zzzzzzzz" })).await.is_err());
    }

    fn telegram_origin() -> Option<WatchOrigin> {
        WatchOrigin::from_identity(&RequestIdentity::from_telegram(42))
    }

    #[tokio::test]
    async fn a_watch_can_follow_an_open_task_and_a_bad_task_leaves_no_watch_behind() {
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-1", "personal", None, "Work", "work", "WK")?;
            hq_db::tasks::create_task(c, "tk-1", "in-1", &hq_db::tasks::NewTask { title: "Ship", created_by: "t", ..Default::default() })?;
            Ok(())
        })
        .unwrap();
        let tools = create_watch_tools(db.clone(), telegram_origin());
        let ok = tools[0]
            .execute(json!({ "interval_minutes": 10, "prompt": "is it merged?", "task_id": "WK-001" }))
            .await
            .unwrap();
        assert_eq!(ok["task"]["display_id"], "WK-001");
        let id = ok["id"].as_str().unwrap().to_string();
        assert_eq!(db.with_conn(|c| background_turns::watch_task_id(c, &id)).unwrap().as_deref(), Some("tk-1"));

        let running = |db: &Database| db.with_conn(background_turns::list_running).unwrap().len();
        assert_eq!(running(&db), 1);
        let bad = tools[0].execute(json!({ "interval_minutes": 10, "prompt": "x", "task_id": "NOPE-9" })).await;
        assert!(bad.is_err());
        assert_eq!(running(&db), 1, "the refused watch was not left running without its task");
    }

    #[tokio::test]
    async fn a_restricted_chat_cannot_attach_a_watch_to_a_task() {
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-1", "personal", None, "Work", "work", "WK")?;
            hq_db::tasks::create_task(c, "tk-1", "in-1", &hq_db::tasks::NewTask { title: "Private", created_by: "t", ..Default::default() })?;
            Ok(())
        })
        .unwrap();
        let mut origin = telegram_origin().unwrap();
        origin.restricted = true;
        let tools = create_watch_tools(db.clone(), Some(origin));
        let err = tools[0]
            .execute(json!({ "interval_minutes": 10, "prompt": "x", "task_id": "WK-001" }))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot attach"), "{err}");
        assert!(!err.contains("Private") && !err.contains("WK-001"), "the refusal says nothing about the task");
        assert!(db.with_conn(background_turns::list_running).unwrap().is_empty(), "no watch was left behind");
        let plain = tools[0].execute(json!({ "interval_minutes": 10, "prompt": "x" })).await;
        assert!(plain.is_ok(), "a watch with no task is still fine");
    }

    #[tokio::test]
    async fn watch_create_reads_the_row_back_and_clamps() {
        let db = Arc::new(Database::open_memory().unwrap());
        let tools = create_watch_tools(db.clone(), telegram_origin());
        let out = tools[0]
            .execute(json!({ "interval_minutes": 1, "duration_hours": 99999, "prompt": "is PR 7 merged?" }))
            .await
            .unwrap();
        assert_eq!(out["status"], "running");
        assert_eq!(out["interval_secs"], MIN_WATCH_INTERVAL_MINS * 60);
        let id = out["id"].as_str().unwrap();
        assert_eq!(out["short_ref"], background_turns::short_ref(id));
        let row = db
            .with_conn(|c| background_turns::get(c, id))
            .unwrap()
            .unwrap();
        assert_eq!(
            (row.platform.as_str(), row.chat_id.as_str()),
            ("telegram", "42")
        );
        assert_eq!(
            row.watch_until.unwrap() - row.created_at,
            MAX_WATCH_EXPIRY_HOURS * 3600
        );

        let listed = tools[1].execute(json!({})).await.unwrap();
        assert_eq!(listed["watches"].as_array().unwrap().len(), 1);
        let short = out["short_ref"].as_str().unwrap();
        tools[2].execute(json!({ "id": short })).await.unwrap();
        let listed = tools[1].execute(json!({})).await.unwrap();
        assert!(listed["watches"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn watch_tools_refuse_without_a_chat_origin() {
        let db = Arc::new(Database::open_memory().unwrap());
        let cli =
            WatchOrigin::from_identity(&RequestIdentity::from_proxy_user("web", vec![], None));
        assert!(cli.is_none());
        let tools = create_watch_tools(db, cli);
        let err = tools[0]
            .execute(json!({ "interval_minutes": 10, "prompt": "x" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("watches need a chat origin"));
        assert!(tools[0].requires_live_user_turn());
    }
}
