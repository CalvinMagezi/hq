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

pub fn create_a2a_tools(db: Arc<Database>) -> Vec<Box<dyn HqTool>> {
    vec![Box::new(AgentMessageSendTool { db })]
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
    let (Some(a), Some(b)) = (&from_row.mission_id, &to_row.mission_id) else {
        bail!("messages go between sessions working on the same task, and one of these has no task");
    };
    if a != b {
        bail!("session {to} works on a different task, so you cannot message it");
    }
    if let Some(hint) = task_hint.filter(|h| !h.is_empty()) {
        let hint = hint.to_string();
        let found = db.with_conn(move |c| t::get_task(c, &hint))?;
        if found.map(|task| task.id).as_deref() != Some(a.as_str()) {
            bail!("that is not the task these sessions share");
        }
    }
    let sent = {
        let sender = sender.to_string();
        db.with_conn(move |c| t::messages_sent_since(c, &sender, RATE_WINDOW_MINUTES))?
    };
    if sent >= MAX_MESSAGES_PER_WINDOW {
        bail!("rate limit: at most {MAX_MESSAGES_PER_WINDOW} messages per {RATE_WINDOW_MINUTES} minutes");
    }
    Ok(a.clone())
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
}
