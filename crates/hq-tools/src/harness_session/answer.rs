//! A blocked session's dialog as data, and answering it by option instead of by keys.
//!
//! The host reads the dialog off the screen with its rule file and numbers it; an answer names
//! that number, so it always goes to the dialog that was read. An option that would let the
//! agent do more from now on without asking ("don't ask again", "remember this path") is the
//! owner's call: Drive, agent sessions and claude.ai connections may pick only one-time answers.

use super::*;
use crate::agent_host::{PendingDialog, PendingRead};

/// Who is answering, as far as the "remember" options are concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answerer {
    /// The owner, or a turn the owner typed.
    Owner,
    /// Something acting on its own; the text names it for the refusal.
    Automated(&'static str),
}

impl Answerer {
    /// Who a tool call answers as: a Drive turn, an agent session, a claude.ai connection, an
    /// `hq_ask` reply or a turn that read untrusted content is automated; anything else is the owner.
    pub fn of(args: &Value, chat: Option<&WatchingChat>) -> Self {
        if chat.is_some_and(|c| c.driver_turn) {
            Answerer::Automated("a Drive turn")
        } else if caller_session(args).is_some() || spawned_session(args).is_some() {
            Answerer::Automated("an agent session")
        } else if super::connector_label(args).is_some() {
            Answerer::Automated("a claude.ai connection")
        } else if chat.is_some_and(|c| c.from_ask) {
            Answerer::Automated("an hq_ask reply")
        } else if args.get(UNTRUSTED_TURN_ARG).and_then(Value::as_bool) == Some(true) {
            Answerer::Automated("a turn that read untrusted content")
        } else {
            Answerer::Owner
        }
    }
}

/// What to do with the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Option(u32),
    /// Dismiss it; the message, if any, is sent as a prompt once the agent is back at its prompt.
    Decline(Option<String>),
}

/// How long a decline's message waits for the agent to come back to its prompt.
const MESSAGE_WAIT: Duration = Duration::from_secs(20);

/// The dialog `session_id` waits at, whole: its number, kind, question, command, options and text.
pub fn pending(db: &Arc<Database>, session_id: &str) -> Result<Value> {
    let row = get_row(db, session_id)?;
    let host = require_alive(&row)?;
    let read = match host.pending(&row.agent_name) {
        Ok(read) => read,
        Err(e) if matches!(e.code(), Some("unsupported" | "unknown_method")) => {
            return Ok(json!({
                "session_id": session_id,
                "pending": null,
                "note": format!("host '{}' cannot read dialogs yet (it needs an update); read the screen with harness_session_logs and answer with harness_session_send keys", row.host),
            }));
        }
        Err(e) => return Err(e.into()),
    };
    Ok(match read {
        Some(read) => pending_view(session_id, &row.cwd, &read),
        None => json!({
            "session_id": session_id,
            "pending": null,
            "note": "the session is not waiting at a dialog HQ can read; if it is blocked, read it with harness_session_logs and answer with harness_session_send keys",
        }),
    })
}

fn pending_view(session_id: &str, cwd: &str, read: &PendingRead) -> Value {
    let d = &read.dialog;
    json!({
        "session_id": session_id,
        "seq": read.seq,
        "kind": d.kind,
        "rule": read.rule,
        "question": d.question,
        "command": d.command,
        "cwd": cwd,
        "options": d.options,
        "truncated": d.truncated,
        "text": d.text,
    })
}

/// Why `who` may not make `choice` on `dialog`, if it may not.
pub fn refusal(dialog: &PendingDialog, choice: &Choice, who: Answerer) -> Option<String> {
    let Choice::Option(id) = choice else {
        return None;
    };
    let Some(option) = dialog.option(*id) else {
        let ids: Vec<String> = dialog.options.iter().map(|o| o.id.to_string()).collect();
        return Some(format!(
            "the dialog has no option {id}; its options are {}",
            ids.join(", ")
        ));
    };
    if option.typed {
        return Some(format!(
            "option {id} ({}) opens a text field instead of answering; decline with a message instead",
            option.label
        ));
    }
    match who {
        Answerer::Automated(by) if option.remember => Some(format!(
            "option {id} ({}) lets the agent do more from now on without asking, so only the owner may pick it (from the web Sessions panel or their phone). {by} may pick a one-time answer or decline.",
            option.label
        )),
        _ => None,
    }
}

/// Answers dialog `seq` of `session_id`. The host checks the number again as it presses, so a
/// dialog that changed in between is never answered by mistake.
pub fn answer(
    db: &Arc<Database>,
    session_id: &str,
    seq: u64,
    choice: Choice,
    who: Answerer,
    chat: Option<&WatchingChat>,
) -> Result<Value> {
    let row = get_row(db, session_id)?;
    let host = require_alive(&row)?;
    answer_with(db, &host, &row, seq, choice, who, chat)
}

/// `answer` against a host already found for `row`.
pub(super) fn answer_with(
    db: &Arc<Database>,
    host: &Host,
    row: &HarnessSessionRow,
    seq: u64,
    choice: Choice,
    who: Answerer,
    chat: Option<&WatchingChat>,
) -> Result<Value> {
    let session_id = row.id.as_str();
    let read = host.pending(&row.agent_name)?.ok_or_else(|| {
        anyhow::anyhow!("session {session_id} is not waiting at a dialog HQ can read")
    })?;
    if read.seq != seq {
        bail!(
            "session {session_id} is at a different dialog now (number {}); read it again with harness_session_pending before answering",
            read.seq
        );
    }
    if let Some(why) = refusal(&read.dialog, &choice, who) {
        bail!(why);
    }
    if chat.is_some_and(|c| c.driver_turn)
        && let Some(why) = super::policy::refuse_drive(db, session_id, &read.dialog)
    {
        bail!(why);
    }
    let option = match &choice {
        Choice::Option(id) => Some(*id),
        Choice::Decline(_) => None,
    };
    let outcome = metered(db, session_id, chat, SendKind::Keys, || {
        host.answer(&row.agent_name, seq, option)
            .map_err(anyhow::Error::from)
    });
    screen_changed(session_id);
    let outcome = outcome?;
    // Who answered, for the session's record and the team's numbers (`harness_session_stats`).
    let by = match who {
        Answerer::Owner => "owner",
        Answerer::Automated(label) => label,
    };
    let picked = match &choice {
        Choice::Option(id) => json!({ "option": id }),
        Choice::Decline(_) => json!("decline"),
    };
    let detail = json!({
        "seq": seq,
        "kind": read.dialog.kind,
        "by": by,
        "choice": picked,
        "registered": outcome.registered,
    })
    .to_string();
    let id = session_id.to_string();
    let _ = db.with_conn(move |c| registry::record_event(c, &id, "answered", by, Some(&detail)));
    let mut report = json!({
        "session_id": session_id,
        "seq": seq,
        "registered": outcome.registered,
    });
    match &choice {
        Choice::Option(id) => {
            let label = read.dialog.option(*id).map(|o| o.label.as_str());
            report["answered"] = json!({ "option": id, "label": label });
        }
        Choice::Decline(_) => report["answered"] = json!("declined"),
    }
    if let Some(agent) = &outcome.agent {
        report["agent_status"] = json!(agent.status);
    }
    if !outcome.registered {
        report["not_registered"] = json!(true);
        report["note"] = json!(
            "the dialog is still open after the answer; read it again with harness_session_pending"
        );
        return Ok(report);
    }
    // Its notice is gone from every device, whichever way it was answered.
    if let Err(e) = hq_db::dialog_alerts::settle(db, session_id, seq, option.is_some()) {
        tracing::warn!(session = %session_id, error = %e, "answer: could not close the dialog's alerts");
    }
    if let Some(task) = record_on_task(db, session_id, mission::Event::Steered) {
        report["task"] = task;
    }
    if let Choice::Decline(Some(message)) = &choice {
        let settled = host.wait(
            &row.agent_name,
            &[AgentStatus::Idle, AgentStatus::Done],
            MESSAGE_WAIT,
        );
        match settled {
            Ok(_) => {
                send(db, session_id, message, chat)?;
                report["message_sent"] = json!(true);
            }
            Err(_) => {
                report["note"] = json!(
                    "the dialog is gone but the agent is not back at its prompt yet; send the message with harness_session_send"
                );
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_host::PendingOption;

    fn dialog() -> PendingDialog {
        let option = |id, label: &str, remember, typed| PendingOption {
            id,
            label: label.into(),
            remember,
            typed,
            selected: id == 1,
        };
        PendingDialog {
            kind: "path".into(),
            question: Some("Do you want to allow this?".into()),
            command: Some("/tmp".into()),
            options: vec![
                option(1, "Yes", false, false),
                option(2, "Yes, and remember this path", true, false),
                option(
                    3,
                    "No, and tell Copilot what to do differently",
                    false,
                    true,
                ),
            ],
            truncated: false,
            text: None,
        }
    }

    #[test]
    fn only_the_owner_may_pick_an_option_that_widens_permissions() {
        let d = dialog();
        let remember = Choice::Option(2);
        assert!(refusal(&d, &remember, Answerer::Owner).is_none());
        for by in ["a Drive turn", "an agent session", "a claude.ai connection"] {
            let why = refusal(&d, &remember, Answerer::Automated(by)).unwrap();
            assert!(why.contains("only the owner") && why.contains(by), "{why}");
            assert!(refusal(&d, &Choice::Option(1), Answerer::Automated(by)).is_none());
            assert!(refusal(&d, &Choice::Decline(None), Answerer::Automated(by)).is_none());
        }
    }

    #[test]
    fn a_text_field_option_or_a_missing_one_is_refused_for_everyone() {
        let d = dialog();
        assert!(
            refusal(&d, &Choice::Option(3), Answerer::Owner)
                .unwrap()
                .contains("decline with a message")
        );
        assert!(
            refusal(&d, &Choice::Option(7), Answerer::Owner)
                .unwrap()
                .contains("1, 2, 3")
        );
    }

    #[test]
    fn who_answers_comes_from_the_call() {
        let chat = |driver_turn, from_ask| WatchingChat {
            thread: "t".into(),
            drive_new: false,
            driver_turn,
            from_ask,
        };
        assert_eq!(Answerer::of(&json!({}), None), Answerer::Owner);
        assert_eq!(
            Answerer::of(&json!({}), Some(&chat(false, false))),
            Answerer::Owner
        );
        assert!(matches!(
            Answerer::of(&json!({}), Some(&chat(true, false))),
            Answerer::Automated(_)
        ));
        assert!(matches!(
            Answerer::of(&json!({}), Some(&chat(false, true))),
            Answerer::Automated(_)
        ));
        assert!(matches!(
            Answerer::of(&json!({ CALLER_SESSION_ARG: "hs-1" }), None),
            Answerer::Automated(_)
        ));
        assert!(matches!(
            Answerer::of(&json!({ SPAWNED_SESSION_ARG: "hs-1" }), None),
            Answerer::Automated(_)
        ));
        assert!(matches!(
            Answerer::of(&json!({ UNTRUSTED_TURN_ARG: true }), None),
            Answerer::Automated(_)
        ));
        assert_eq!(
            Answerer::of(&json!({ CONNECTOR_ARG: "Claude" }), None),
            Answerer::Automated("a claude.ai connection")
        );
    }
}
