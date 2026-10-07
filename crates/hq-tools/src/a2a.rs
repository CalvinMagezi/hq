//! Agent-to-agent messaging on the task system. A message is a comment on a task
//! the two sessions share, so the thread is the durable record. It is typed into
//! the recipient's pane when the recipient is next idle.
//!
//! The sender is whatever session the gateway attested. Nothing a caller writes
//! names the sender, so a session cannot message as another one.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_db::Database;
use hq_db::harness_sessions_registry::{self as registry, HarnessSessionRow};
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::harness_session::caller_session;
use crate::herdr::AgentStatus;
use crate::registry::HqTool;
use crate::util::arg_str;

/// Longest message body, in characters.
pub const MAX_MESSAGE_CHARS: usize = 8000;
/// Most messages one session may send inside the window.
pub const MAX_MESSAGES_PER_WINDOW: i64 = 60;
pub const RATE_WINDOW_MINUTES: i64 = 60;

/// How deep a chain of delegated sessions may go (a person's session is 0).
pub const MAX_SPAWN_DEPTH: i64 = 2;
/// Children one session may have running at once.
pub const MAX_LIVE_CHILDREN: usize = 3;
/// Sessions one session may start inside the rate window.
pub const MAX_CHILDREN_PER_WINDOW: i64 = 10;
/// The coding agent a delegation uses when the delegator names none.
const DEFAULT_DELEGATE_HARNESS: &str = "claude-code";
/// The most output quoted to a parent when a child reports in.
const REPORT_TAIL_CHARS: usize = 1500;

pub fn create_a2a_tools(vault_path: std::path::PathBuf, db: Arc<Database>) -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(AgentMessageSendTool { db: db.clone() }),
        Box::new(AgentDelegateTool { vault_path, db }),
    ]
}

/// What a session needs to start a child: the parent row and the task it works on.
#[derive(Debug)]
pub struct DelegateSlot {
    pub parent: HarnessSessionRow,
    pub task_id: String,
    /// The new child's depth.
    pub depth: i64,
}

/// Checks that `caller` may start another session: it is running, it works on a
/// task to attach the work to, the chain is not too deep, it does not already have
/// too many children running, and it is inside its hourly limit.
pub fn authorize_delegate(db: &Database, caller: &str) -> Result<DelegateSlot> {
    let Some(parent) = running(db, caller)? else {
        bail!("the delegating session is not running");
    };
    let Some(task_id) = parent.mission_id.clone() else {
        bail!("delegate from a session that works on a task: the new work is filed under it");
    };
    let depth = parent.spawn_depth + 1;
    if depth > MAX_SPAWN_DEPTH {
        bail!("delegation is at most {MAX_SPAWN_DEPTH} levels deep, and this session is already at the limit");
    }
    let id = parent.id.clone();
    let (live, recent) = db.with_conn(move |c| {
        Ok((
            registry::running_children(c, &id)?.len(),
            registry::children_started_since(c, &id, RATE_WINDOW_MINUTES)?,
        ))
    })?;
    if live >= MAX_LIVE_CHILDREN {
        bail!("this session already has {live} running children (at most {MAX_LIVE_CHILDREN}); wait for one to finish");
    }
    if recent >= MAX_CHILDREN_PER_WINDOW {
        bail!("rate limit: at most {MAX_CHILDREN_PER_WINDOW} delegated sessions per {RATE_WINDOW_MINUTES} minutes");
    }
    Ok(DelegateSlot { parent, task_id, depth })
}

fn tail_of(text: &str) -> String {
    let chars: Vec<char> = text.trim().chars().collect();
    chars[chars.len().saturating_sub(REPORT_TAIL_CHARS)..].iter().collect()
}

/// What an agent last said, taken from its screen: the text after its last reply
/// marker (`⏺`), up to the status line or prompt box that follows. None for a
/// screen without a marker, in which case the caller quotes the tail instead.
pub fn last_reply(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.lines().collect();
    let start = lines.iter().rposition(|l| l.trim_start().starts_with('⏺'))?;
    let mut reply: Vec<String> = Vec::new();
    for (i, line) in lines[start..].iter().enumerate() {
        let text = line.trim();
        let text = if i == 0 { text.trim_start_matches('⏺').trim() } else { text };
        let chrome = ['✻', '─', '❯', '⏵'].iter().any(|c| text.starts_with(*c));
        if chrome {
            break;
        }
        if !text.is_empty() {
            reply.push(text.to_string());
        }
    }
    let joined = reply.join("\n");
    (!joined.is_empty()).then_some(joined)
}

/// Tells a session's parent that it finished or exited, with the tail of its
/// output, as a message on the child's task thread. Nothing for a session that
/// has no parent. The supervisor delivers it when the parent is idle.
pub fn report_to_parent(db: &Database, child: &HarnessSessionRow, event: &str, output: &str) -> Result<Option<i64>> {
    let Some(parent) = child.parent_session_id.clone() else {
        return Ok(None);
    };
    let Some(task) = child.mission_id.clone() else {
        return Ok(None);
    };
    let body = match (last_reply(output), tail_of(output)) {
        (Some(reply), _) => format!(
            "Delegated session {} {event}. Its final reply:\n{}",
            child.id,
            tail_of(&reply)
        ),
        (None, tail) if !tail.is_empty() => {
            format!("Delegated session {} {event}. Its last output:\n{tail}", child.id)
        }
        _ => format!("Delegated session {} {event}.", child.id),
    };
    let from = child.id.clone();
    let message = db.with_conn(move |c| t::add_message(c, &task, &from, &parent, &body, None))?;
    Ok(Some(message.id))
}

fn running(db: &Database, id: &str) -> Result<Option<HarnessSessionRow>> {
    let id = id.to_string();
    Ok(db
        .with_conn(move |c| registry::get(c, &id))?
        .filter(|row| row.status == registry::STATUS_RUNNING))
}

/// Checks that `sender` may message `to` and returns the task whose thread holds
/// the message. The two sessions must work on the same task, the recipient must be
/// running, and the sender must be inside its rate limit.
pub fn authorize(
    db: &Database,
    sender: &str,
    to: &str,
    task_hint: Option<&str>,
    body: &str,
) -> Result<String> {
    if body.trim().is_empty() {
        bail!("a message needs a body");
    }
    if body.chars().count() > MAX_MESSAGE_CHARS {
        bail!("a message is at most {MAX_MESSAGE_CHARS} characters");
    }
    if sender == to {
        bail!("a session cannot message itself");
    }
    let Some(from_row) = running(db, sender)? else {
        bail!("the sending session is not running");
    };
    let Some(to_row) = running(db, to)? else {
        bail!("session {to} is not running, so it cannot receive a message");
    };
    let thread = thread_between(&from_row, &to_row)?;
    if let Some(hint) = task_hint.filter(|h| !h.is_empty()) {
        let hint = hint.to_string();
        let found = db.with_conn(move |c| t::get_task(c, &hint))?;
        if found.map(|task| task.id).as_deref() != Some(thread.as_str()) {
            bail!("that is not the task thread these sessions share");
        }
    }
    let sent = {
        let sender = sender.to_string();
        db.with_conn(move |c| t::messages_sent_since(c, &sender, RATE_WINDOW_MINUTES))?
    };
    if sent >= MAX_MESSAGES_PER_WINDOW {
        bail!("rate limit: at most {MAX_MESSAGES_PER_WINDOW} messages per {RATE_WINDOW_MINUTES} minutes");
    }
    Ok(thread)
}

/// The task thread a message between two sessions goes on: the task they both
/// work on, or, for a parent and the session it started, the child's task.
fn thread_between(from: &HarnessSessionRow, to: &HarnessSessionRow) -> Result<String> {
    if let (Some(a), Some(b)) = (&from.mission_id, &to.mission_id)
        && a == b
    {
        return Ok(a.clone());
    }
    let child = if to.parent_session_id.as_deref() == Some(from.id.as_str()) {
        Some(to)
    } else if from.parent_session_id.as_deref() == Some(to.id.as_str()) {
        Some(from)
    } else {
        None
    };
    match child {
        Some(child) => child
            .mission_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("the child session has no task to hold the message")),
        None if from.mission_id.is_none() || to.mission_id.is_none() => bail!(
            "messages go between sessions working on the same task, or a parent and its child, and one of these has no task"
        ),
        None => bail!("session {} works on a different task, so you cannot message it", to.id),
    }
}

pub struct AgentMessageSendTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for AgentMessageSendTool {
    fn name(&self) -> &str {
        "agent_message_send"
    }
    fn description(&self) -> &str {
        "Send a message to another running agent session that works on the same task. It is added to the task's thread and typed into that session when it is next idle. Only a launched agent session can use this; the sender is the session HQ verified, never a name you pass."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to_session": { "type": "string", "description": "Session id of the recipient (see harness_session_list)" },
                "body": { "type": "string", "description": "The message" },
                "task_id": { "type": "string", "description": "Optional: the shared task, to be explicit" },
                "reply_to": { "type": "integer", "description": "Optional: id of the message this answers" }
            },
            "required": ["to_session", "body"]
        })
    }
    fn category(&self) -> &str {
        "agent-comm"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(sender) = caller_session(&args) else {
            bail!("agent_message_send is for launched agent sessions: connect with the session's own token");
        };
        let (to, body) = (arg_str(&args, "to_session"), arg_str(&args, "body"));
        let hint = arg_str(&args, "task_id");
        let task = authorize(&self.db, sender, &to, Some(&hint), &body)?;
        let (from, reply_to) = (sender.to_string(), args.get("reply_to").and_then(Value::as_i64));
        let to_for_send = to.clone();
        let message = self
            .db
            .with_conn(move |c| t::add_message(c, &task, &from, &to, &body, reply_to))?;
        // Deliver at once when the recipient is idle; otherwise the supervisor
        // does it when the recipient next is. Either way the message is queued.
        let db = self.db.clone();
        let delivered = tokio::task::spawn_blocking(move || {
            let status = status_now(&db, &to_for_send)?;
            deliver_if_idle(&db, &to_for_send, status)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .flatten()
        .is_some();
        Ok(json!({
            "id": message.id,
            "task_id": message.task_id,
            "to_session": message.to_session_id,
            "delivered": delivered,
            "note": if delivered {
                "typed into the recipient"
            } else {
                "queued on the task thread; it is typed into the recipient when it is next idle"
            },
        }))
    }
}

pub struct AgentDelegateTool {
    vault_path: std::path::PathBuf,
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for AgentDelegateTool {
    fn name(&self) -> &str {
        "agent_delegate"
    }
    fn description(&self) -> &str {
        "Hand a piece of your task to a new agent session. HQ files a sub-task under your task, starts a session on it in your directory, and tells it who delegated. The result comes back to you as a message when it finishes; you can also message it with agent_message_send. Limited depth, running children and rate. Only a launched agent session can use this."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "What the sub-task is, one line" },
                "description": { "type": "string", "description": "Everything the worker needs: what to do and what done looks like" },
                "harness": { "type": "string", "description": "Which coding agent (default: claude-code)" }
            },
            "required": ["title", "description"]
        })
    }
    fn category(&self) -> &str {
        "agent-comm"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(caller) = caller_session(&args) else {
            bail!("agent_delegate is for launched agent sessions: connect with the session's own token");
        };
        let (title, description) = (arg_str(&args, "title"), arg_str(&args, "description"));
        if title.trim().is_empty() || description.trim().is_empty() {
            bail!("a delegation needs a title and a description");
        }
        // One delegation at a time, so two calls cannot both pass the limits
        // before either child exists.
        let _one_at_a_time = DELEGATION_LOCK.lock().await;
        let slot = authorize_delegate(&self.db, caller)?;
        let harness = pick_harness(&arg_str(&args, "harness"), &slot.parent.harness)?;
        crate::harness_session::resolve(&harness)?;
        let sub = file_subtask(&self.db, &slot, caller, &title, &description)?;
        let prompt = delegation_prompt(&slot.parent.id, &sub.display_id, &title, &description);
        let spawned = crate::harness_session::spawn_with(
            &self.vault_path,
            &self.db,
            crate::harness_session::SpawnRequest {
                host: Some(&slot.parent.host),
                harness: &harness,
                prompt: Some(&prompt),
                cwd: std::path::Path::new(&slot.parent.cwd),
                label: &format!("delegate: {title}"),
                mission_id: Some(&sub.id),
                watch: None,
                goal: crate::harness_session::GoalText::default(),
            },
        )
        .await?;
        let child = spawned
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("the new session reported no id"))?
            .to_string();
        let (id, parent, depth) = (child.clone(), slot.parent.id.clone(), slot.depth);
        self.db.with_conn(move |c| registry::set_parent(c, &id, &parent, depth))?;
        let (task, note) = (slot.task_id.clone(), format!("Delegated {} to session {child}: {title}", sub.display_id));
        let author = caller.to_string();
        self.db.with_conn(move |c| t::add_comment(c, &task, &author, &note, None))?;
        Ok(json!({
            "task_id": sub.display_id,
            "session_id": child,
            "note": "the worker was told who delegated; its result comes back to you as a message when it finishes",
        }))
    }
}

static DELEGATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Which agent a delegated session runs as: the default, or the delegator's own
/// kind. Never a configured account profile the delegator is not running as, which
/// would spend another account's credit, and not another agent kind until those
/// are wired for delegation.
fn pick_harness(requested: &str, parent: &str) -> Result<String> {
    let want = if requested.is_empty() { DEFAULT_DELEGATE_HARNESS } else { requested };
    if want == DEFAULT_DELEGATE_HARNESS || want == parent {
        return Ok(want.to_string());
    }
    bail!("a delegated session runs as {DEFAULT_DELEGATE_HARNESS} or as the delegator's own agent ({parent}), not {want}")
}

/// The sub-task that holds the delegated work, filed under the delegator's task
/// (or under that task's own parent, since sub-tasks go one level deep).
fn file_subtask(
    db: &Database,
    slot: &DelegateSlot,
    caller: &str,
    title: &str,
    description: &str,
) -> Result<t::Task> {
    let (task_id, caller, title, description) =
        (slot.task_id.clone(), caller.to_string(), title.to_string(), description.to_string());
    db.with_conn(move |c| {
        let Some(task) = t::get_task(c, &task_id)? else {
            bail!("the delegator's task no longer exists");
        };
        let parent = task.parent_task_id.clone().unwrap_or_else(|| task.id.clone());
        t::create_task(
            c,
            &crate::util::generate_id("tk"),
            &task.initiative_id,
            &t::NewTask {
                title: &title,
                description: &description,
                parent_task_id: Some(&parent),
                created_by: &caller,
                ..Default::default()
            },
        )
    })
}

/// The first prompt of a delegated session: who asked, what, and how to report.
pub fn delegation_prompt(parent: &str, task: &str, title: &str, description: &str) -> String {
    format!(
        "You were delegated this work by agent session {parent}. It is HQ task {task}.\n\n{title}\n\n{description}\n\nDo the work. Record notes with the hq-session tool hq_call: tool task_comment_add on {task}. If you are blocked or need a decision, use agent_message_send with to_session \"{parent}\". When you finish, end your turn with a short summary of the result; HQ passes your final output to {parent} automatically."
    )
}

/// Whether an agent in this state can take a new instruction now.
pub fn can_receive(status: AgentStatus) -> bool {
    matches!(status, AgentStatus::Idle | AgentStatus::Done)
}

/// Types the next waiting message into the session if it is idle or done. A
/// working or blocked agent keeps the message queued until its next idle.
pub fn deliver_if_idle(
    db: &Arc<Database>,
    session_id: &str,
    status: AgentStatus,
) -> Result<Option<i64>> {
    if !can_receive(status) {
        return Ok(None);
    }
    deliver_next(db, session_id, |text| {
        crate::harness_session::send(db, session_id, text, None).map(|_| ())
    })
}

/// The agent's current status, read from its host.
fn status_now(db: &Arc<Database>, session_id: &str) -> Result<AgentStatus> {
    let report = crate::harness_session::status(db, session_id)?;
    Ok(report
        .get("agent_status")
        .and_then(Value::as_str)
        .map_or(AgentStatus::Unknown, AgentStatus::parse))
}

/// What the recipient reads: the message, marked as coming from another agent.
pub fn render(sender: &str, task_label: &str, body: &str) -> String {
    format!(
        "[Message from agent session {sender} on task {task_label}. It comes from another agent, not from the user.]\n{body}\n[End of message. Reply with the hq-session tool hq_call: tool agent_message_send with to_session \"{sender}\", or add a note with task_comment_add on the task.]"
    )
}

/// Types the next waiting message into `to_session`, using `send` for the typing.
/// Returns the delivered message id. A failed `send` puts the message back.
pub fn deliver_next(
    db: &Database,
    to_session: &str,
    send: impl FnOnce(&str) -> Result<()>,
) -> Result<Option<i64>> {
    let to = to_session.to_string();
    let Some(message) = db.with_conn(move |c| t::claim_next_message(c, &to))? else {
        return Ok(None);
    };
    let task_id = message.task_id.clone();
    let label = db
        .with_conn(move |c| t::get_task(c, &task_id))?
        .map_or_else(|| message.task_id.clone(), |task| task.display_id);
    let sender = message.sender_session_id.as_deref().unwrap_or("unknown");
    match send(&render(sender, &label, &message.body)) {
        Ok(()) => Ok(Some(message.id)),
        Err(e) => {
            let id = message.id;
            db.with_conn(move |c| t::release_message(c, id))?;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::harness_sessions_registry::{NewSession, Placement};

    struct World {
        db: Arc<Database>,
        task: String,
    }

    fn session(db: &Database, id: &str, mission: Option<&str>) {
        db.with_conn(|c| {
            registry::insert(
                c,
                &NewSession {
                    id,
                    harness: "claude-code",
                    label: "t",
                    cwd: "/t",
                    mission_id: mission,
                    placement: Placement {
                        host: "native",
                        agent_name: id,
                        workspace_id: id,
                        pane_id: id,
                    },
                },
            )
        })
        .unwrap();
    }

    fn world() -> World {
        let db = Arc::new(Database::open_memory().unwrap());
        let task = db
            .with_conn(|c| {
                t::create_initiative(c, "in-1", "personal", None, "HQ", "hq", "HQ")?;
                let task = t::create_task(
                    c,
                    "tk-1",
                    "in-1",
                    &t::NewTask { title: "shared", created_by: "test", ..Default::default() },
                )?;
                let other = t::create_task(
                    c,
                    "tk-2",
                    "in-1",
                    &t::NewTask { title: "other", created_by: "test", ..Default::default() },
                )?;
                let _ = other;
                Ok(task.id)
            })
            .unwrap();
        session(&db, "hs-a", Some(&task));
        session(&db, "hs-b", Some(&task));
        session(&db, "hs-other", Some("tk-2"));
        session(&db, "hs-loose", None);
        World { db, task }
    }

    #[test]
    fn only_an_idle_or_done_agent_takes_a_message() {
        assert!(can_receive(AgentStatus::Idle) && can_receive(AgentStatus::Done));
        for busy in [AgentStatus::Working, AgentStatus::Blocked, AgentStatus::Unknown] {
            assert!(!can_receive(busy), "{busy:?}");
        }
    }

    #[test]
    fn two_sessions_on_the_same_task_may_message() {
        let w = world();
        assert_eq!(authorize(&w.db, "hs-a", "hs-b", None, "hi").unwrap(), w.task);
        assert_eq!(authorize(&w.db, "hs-a", "hs-b", Some(&w.task), "hi").unwrap(), w.task);
    }

    #[test]
    fn strangers_loose_sessions_and_wrong_tasks_are_refused() {
        let w = world();
        for (from, to, hint) in [
            ("hs-a", "hs-other", None),
            ("hs-a", "hs-loose", None),
            ("hs-loose", "hs-a", None),
            ("hs-a", "hs-b", Some("tk-2")),
            ("hs-a", "hs-a", None),
            ("hs-a", "hs-missing", None),
            ("hs-ghost", "hs-b", None),
        ] {
            assert!(authorize(&w.db, from, to, hint, "hi").is_err(), "{from} -> {to} {hint:?}");
        }
    }

    #[test]
    fn a_message_to_a_session_that_ended_is_refused() {
        let w = world();
        w.db.with_conn(|c| registry::set_status_exited_if_running(c, "hs-b")).unwrap();
        let err = authorize(&w.db, "hs-a", "hs-b", None, "hi").unwrap_err().to_string();
        assert!(err.contains("not running"), "{err}");
    }

    #[test]
    fn empty_and_oversized_bodies_are_refused() {
        let w = world();
        assert!(authorize(&w.db, "hs-a", "hs-b", None, "   ").is_err());
        let big = "x".repeat(MAX_MESSAGE_CHARS + 1);
        assert!(authorize(&w.db, "hs-a", "hs-b", None, &big).is_err());
        assert!(authorize(&w.db, "hs-a", "hs-b", None, &"x".repeat(MAX_MESSAGE_CHARS)).is_ok());
    }

    #[test]
    fn a_sender_is_limited_per_hour() {
        let w = world();
        w.db.with_conn(|c| {
            for i in 0..MAX_MESSAGES_PER_WINDOW {
                t::add_message(c, &w.task, "hs-a", "hs-b", &format!("m{i}"), None)?;
            }
            Ok(())
        })
        .unwrap();
        let err = authorize(&w.db, "hs-a", "hs-b", None, "one more").unwrap_err().to_string();
        assert!(err.contains("rate limit"), "{err}");
        assert!(authorize(&w.db, "hs-b", "hs-a", None, "someone else is fine").is_ok());
    }

    #[tokio::test]
    async fn the_tool_needs_an_attested_session_and_ignores_a_claimed_sender() {
        let w = world();
        let tool = AgentMessageSendTool { db: w.db.clone() };
        let claimed = json!({"to_session": "hs-b", "body": "hi", "from": "hs-a", "sender": "hs-a"});
        assert!(tool.execute(claimed).await.is_err(), "no attested session, no message");

        let mut attested = json!({"to_session": "hs-b", "body": "hi", "sender": "hs-other"});
        attested[crate::harness_session::CALLER_SESSION_ARG] = "hs-a".into();
        let sent = tool.execute(attested).await.unwrap();
        let thread = w.db.with_conn(|c| t::list_comments(c, &w.task)).unwrap();
        assert_eq!(thread.len(), 1);
        assert_eq!(thread[0].sender_session_id.as_deref(), Some("hs-a"));
        assert_eq!(sent["to_session"], "hs-b");
    }

    #[test]
    fn delivery_types_one_marked_message_and_a_failed_send_puts_it_back() {
        let w = world();
        w.db.with_conn(|c| {
            t::add_message(c, &w.task, "hs-a", "hs-b", "please review", None)?;
            t::add_message(c, &w.task, "hs-a", "hs-b", "second", None)
        })
        .unwrap();

        let mut typed = String::new();
        let first = deliver_next(&w.db, "hs-b", |text| {
            typed = text.to_string();
            Ok(())
        })
        .unwrap();
        assert!(first.is_some());
        assert!(typed.contains("please review") && typed.contains("hs-a"), "{typed}");
        assert!(typed.contains("not from the user"), "a peer's words are marked as such: {typed}");

        let failed = deliver_next(&w.db, "hs-b", |_| bail!("pane gone"));
        assert!(failed.is_err());
        let again = deliver_next(&w.db, "hs-b", |_| Ok(())).unwrap();
        assert!(again.is_some(), "the message that failed to type is delivered next time");
        assert!(deliver_next(&w.db, "hs-b", |_| Ok(())).unwrap().is_none());
    }

    fn child_of(db: &Database, parent: &str, id: &str, mission: &str, depth: i64) {
        session(db, id, Some(mission));
        db.with_conn(|c| registry::set_parent(c, id, parent, depth)).unwrap();
    }

    #[test]
    fn a_parent_and_its_child_may_message_on_the_childs_task_even_on_different_tasks() {
        let w = world();
        // hs-a works on tk-1; its child works on the sub-task tk-2.
        child_of(&w.db, "hs-a", "hs-kid", "tk-2", 1);
        assert_eq!(authorize(&w.db, "hs-a", "hs-kid", None, "go").unwrap(), "tk-2");
        assert_eq!(authorize(&w.db, "hs-kid", "hs-a", None, "done").unwrap(), "tk-2");
        // A sibling of the parent still cannot reach the child.
        assert!(authorize(&w.db, "hs-b", "hs-kid", None, "hi").is_err());
        // A child with no task has no thread to hold the message.
        session(&w.db, "hs-notask", None);
        w.db.with_conn(|c| registry::set_parent(c, "hs-notask", "hs-a", 1)).unwrap();
        assert!(authorize(&w.db, "hs-a", "hs-notask", None, "hi").is_err());
    }

    #[test]
    fn delegation_needs_a_running_session_with_a_task() {
        let w = world();
        let slot = authorize_delegate(&w.db, "hs-a").unwrap();
        assert_eq!((slot.task_id.as_str(), slot.depth), (w.task.as_str(), 1));
        assert!(authorize_delegate(&w.db, "hs-loose").is_err(), "no task, nothing to file under");
        assert!(authorize_delegate(&w.db, "hs-ghost").is_err());
        w.db.with_conn(|c| registry::set_status_exited_if_running(c, "hs-a")).unwrap();
        assert!(authorize_delegate(&w.db, "hs-a").is_err(), "an ended session cannot delegate");
    }

    #[test]
    fn delegation_depth_children_and_rate_are_limited() {
        let w = world();
        // Depth: a grandchild's child would be level 3.
        child_of(&w.db, "hs-a", "hs-l1", "tk-2", 1);
        child_of(&w.db, "hs-l1", "hs-l2", "tk-2", 2);
        let err = authorize_delegate(&w.db, "hs-l2").unwrap_err().to_string();
        assert!(err.contains("levels deep"), "{err}");
        assert!(authorize_delegate(&w.db, "hs-l1").is_ok(), "level 2 may still be started by level 1");

        // Running children: hs-l1 already has one (hs-l2); two more fills it.
        child_of(&w.db, "hs-l1", "hs-l3", "tk-2", 2);
        child_of(&w.db, "hs-l1", "hs-l4", "tk-2", 2);
        let err = authorize_delegate(&w.db, "hs-l1").unwrap_err().to_string();
        assert!(err.contains("running children"), "{err}");
        w.db.with_conn(|c| registry::set_status_exited_if_running(c, "hs-l4")).unwrap();
        assert!(authorize_delegate(&w.db, "hs-l1").is_ok(), "a finished child frees a slot");

        // Rate: the window counts children that already ended too.
        for i in 0..MAX_CHILDREN_PER_WINDOW {
            let id = format!("hs-old{i}");
            child_of(&w.db, "hs-b", &id, "tk-2", 1);
            w.db.with_conn(|c| registry::set_status_exited_if_running(c, &id)).unwrap();
        }
        let err = authorize_delegate(&w.db, "hs-b").unwrap_err().to_string();
        assert!(err.contains("rate limit"), "{err}");
    }

    #[test]
    fn a_finished_child_reports_to_its_parent_with_its_output() {
        let w = world();
        child_of(&w.db, "hs-a", "hs-kid", "tk-2", 1);
        let kid = w.db.with_conn(|c| registry::get(c, "hs-kid")).unwrap().unwrap();
        let sent = report_to_parent(&w.db, &kid, "finished", "the answer is 391").unwrap();
        assert!(sent.is_some());
        let mut typed = String::new();
        deliver_next(&w.db, "hs-a", |text| {
            typed = text.to_string();
            Ok(())
        })
        .unwrap();
        assert!(typed.contains("hs-kid") && typed.contains("the answer is 391"), "{typed}");

        let loner = w.db.with_conn(|c| registry::get(c, "hs-b")).unwrap().unwrap();
        assert!(report_to_parent(&w.db, &loner, "finished", "x").unwrap().is_none(), "no parent, nobody to tell");
    }

    #[test]
    fn a_report_quotes_only_the_tail_of_a_long_output() {
        let w = world();
        child_of(&w.db, "hs-a", "hs-kid", "tk-2", 1);
        let kid = w.db.with_conn(|c| registry::get(c, "hs-kid")).unwrap().unwrap();
        let long = format!("{}END", "x".repeat(10_000));
        report_to_parent(&w.db, &kid, "finished", &long).unwrap();
        let thread = w.db.with_conn(|c| t::list_comments(c, "tk-2")).unwrap();
        assert!(thread[0].body.ends_with("END") && thread[0].body.len() < 2000);
    }

    #[test]
    fn the_worker_is_told_who_delegated_what_and_how_to_report() {
        let text = delegation_prompt("hs-parent", "HQ-7", "multiply", "17 times 23");
        for needle in ["hs-parent", "HQ-7", "17 times 23", "task_comment_add", "agent_message_send"] {
            assert!(text.contains(needle), "{needle}: {text}");
        }
    }

    const SCREEN: &str = "▐▛███▛█   Claude Code v2.1.292\n\n❯ You were delegated this work by agent session hs-p.\n\n  multiply\n\n  Called hq-session 2 times\n\n⏺ 391\n  Note logged on PERSONAL-INBOX-002 (comment id 5).\n✻ Cooked for 8s · done 8:22 AM\n\n────────────\n❯\n────────────\n  ⏵⏵ bypass permissions on";

    #[test]
    fn the_final_reply_is_quoted_without_the_prompt_or_the_screen_chrome() {
        let reply = last_reply(SCREEN).unwrap();
        assert_eq!(reply, "391\nNote logged on PERSONAL-INBOX-002 (comment id 5).");
        assert!(!reply.contains("delegated") && !reply.contains("Cooked"));
    }

    #[test]
    fn the_last_reply_wins_and_a_screen_without_one_has_none() {
        let two = "⏺ first answer\n✻ done\n❯ next\n⏺ second answer\n✻ done";
        assert_eq!(last_reply(two).as_deref(), Some("second answer"));
        assert_eq!(last_reply("plain text with no marker"), None);
        assert_eq!(last_reply(""), None);
    }

    #[test]
    fn a_report_prefers_the_final_reply_and_falls_back_to_the_tail() {
        let w = world();
        child_of(&w.db, "hs-a", "hs-kid", "tk-2", 1);
        let kid = w.db.with_conn(|c| registry::get(c, "hs-kid")).unwrap().unwrap();
        report_to_parent(&w.db, &kid, "finished", SCREEN).unwrap();
        report_to_parent(&w.db, &kid, "finished", "no marker here, just text").unwrap();
        let thread = w.db.with_conn(|c| t::list_comments(c, "tk-2")).unwrap();
        assert!(thread[0].body.contains("final reply") && thread[0].body.contains("391"), "{}", thread[0].body);
        assert!(!thread[0].body.contains("delegated this work"));
        assert!(thread[1].body.contains("last output") && thread[1].body.contains("just text"));
    }

    #[test]
    fn a_worker_may_use_the_default_agent_or_the_delegators_own_and_no_other_profile() {
        assert_eq!(pick_harness("", "claude-code").unwrap(), "claude-code");
        assert_eq!(pick_harness("claude-code", "claude-code").unwrap(), "claude-code");
        // A configured account profile is not something an agent may choose.
        assert!(pick_harness("claude-kola", "claude-code").is_err());
        // But a delegator running as that profile may hand work to its own kind.
        assert_eq!(pick_harness("claude-kola", "claude-kola").unwrap(), "claude-kola");
        assert!(pick_harness("codex", "claude-code").is_err(), "other agents are not enabled for delegation yet");
    }
}
