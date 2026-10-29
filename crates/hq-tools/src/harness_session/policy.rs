//! The command policy a session runs under (docs/plans/remote-driven-teams.md, phase 2), and
//! what it says about the dialogs the session shows. Policy is code: the owner writes it, the
//! supervisor applies it, and a model never approves a command. A dialog the policy allows is
//! answered with its one-time yes, one it denies is declined with the rule named, and anything
//! else goes to the owner as before.

use super::answer::{self, Answerer, Choice};
use super::*;
use crate::agent_host::PendingDialog;
use hq_core::command_policy::{self as cp, Decision, Policy};

/// Who answers when the policy does, as far as `answer` is concerned: never the owner, so a
/// "remember" option stays out of reach.
pub const POLICY_ANSWERER: Answerer = Answerer::Automated("the command policy");

/// What the caller named in `policy`, if anything.
pub fn parse(args: &Value) -> Option<String> {
    args.get("policy")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
}

/// The policy a new session starts with: the one `asked` for, or its purpose's (`builder` for
/// code, `reviewer` for review, none for the rest). A session started by another may not run
/// under a wider policy than its starter's.
pub(super) fn choose(
    db: &Arc<Database>,
    asked: Option<&str>,
    purpose: Purpose,
    parent: Option<&str>,
) -> Result<Option<Policy>> {
    let configured = agent_host_config().command_policies;
    let name = match (asked, purpose) {
        (Some(name), _) => name,
        (None, Purpose::Code) => cp::BUILDER,
        (None, Purpose::Review) => cp::REVIEWER,
        (None, _) => cp::NONE,
    };
    let policy = cp::resolve(name, &configured).map_err(|e| anyhow::anyhow!(e))?;
    let Some(parent) = parent else {
        return Ok(policy);
    };
    let Some(theirs) = of(db, parent) else {
        return Ok(policy);
    };
    match &policy {
        None => bail!(
            "session {parent} runs under the '{}' command policy, so a session it starts needs one too, no wider than that",
            theirs.name
        ),
        Some(mine) if !mine.no_wider_than(&theirs) => bail!(
            "the '{}' command policy lets through more than '{}', which session {parent} runs under; start this one on '{}' or a narrower policy",
            mine.name,
            theirs.name,
            theirs.name
        ),
        Some(_) => Ok(policy),
    }
}

/// The policy `session_id` started with.
pub fn of(db: &Arc<Database>, session_id: &str) -> Option<Policy> {
    let id = session_id.to_string();
    db.with_conn(move |c| hq_db::session_policies::get(c, &id))
        .ok()
        .flatten()
}

/// Stores the policy a session started with and records the choice as an event.
pub(super) fn record(db: &Arc<Database>, session_id: &str, policy: &Policy) {
    let (id, p) = (session_id.to_string(), policy.clone());
    let detail = json!({ "policy": policy.name }).to_string();
    let stored = db.with_conn(move |c| {
        hq_db::session_policies::record(c, &id, &p)?;
        registry::record_event(c, &id, "policy", "launch", Some(&detail))
    });
    if let Err(e) = stored {
        tracing::warn!(session = session_id, error = %e, "the session's command policy was not recorded");
    }
}

/// What `policy` says about a dialog, or `None` for a kind it does not judge (a question, a
/// trust dialog): those always go to the owner.
pub fn judge(policy: &Policy, dialog: &PendingDialog, folder: &str) -> Option<Decision> {
    let subject = dialog.command.as_deref().unwrap_or_default();
    let decision = match dialog.kind.as_str() {
        "command" => policy.decide(subject),
        "edit" | "path" => policy.decide_path(subject, folder),
        _ => return None,
    };
    // A dialog cut short may hide the part that matters, so only a deny of what is visible
    // stands.
    if dialog.truncated && !matches!(decision, Decision::Deny { .. }) {
        return Some(Decision::Escalate {
            reason: "the dialog was too long to read whole".into(),
        });
    }
    Some(decision)
}

/// The one-time yes a policy may pick: never a "remember" option or a text field.
pub fn yes_option(dialog: &PendingDialog) -> Option<u32> {
    let options: Vec<hq_db::dialog_alerts::DialogOption> = dialog
        .options
        .iter()
        .map(|o| hq_db::dialog_alerts::DialogOption {
            id: o.id,
            label: o.label.clone(),
            remember: o.remember,
            typed: o.typed,
        })
        .collect();
    hq_db::dialog_alerts::approve_option(&dialog.kind, &options)
}

/// What the agent is told when its command is declined.
pub fn denial_message(policy: &Policy, rule: &str) -> String {
    format!(
        "HQ declined that: the '{}' command policy this session runs under denies `{rule}`. Find another way, or say what you need and why so the owner can decide.",
        policy.name
    )
}

/// What the policy did with one dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Acted {
    /// Answered with its one-time yes.
    Allowed { option: u32 },
    /// Declined, with the rule named to the agent.
    Denied,
    /// Left for the owner.
    Escalated { reason: String },
}

/// Applies `policy` to dialog `seq` of `row`: answers it (allowed), declines it (denied), or
/// leaves it (escalated). Blocking. Every decision is recorded as a `policy_decision` event.
/// An answer that fails (the dialog moved on, the host did not answer) comes back as an
/// escalation, so the owner hears about the dialog after all.
pub fn act(
    db: &Arc<Database>,
    host: &Host,
    row: &HarnessSessionRow,
    policy: &Policy,
    dialog: &PendingDialog,
    seq: u64,
) -> Option<Acted> {
    let decision = judge(policy, dialog, &row.cwd)?;
    let (choice, acted) = match &decision {
        Decision::Allow { .. } => match yes_option(dialog) {
            Some(id) => (Some(Choice::Option(id)), Acted::Allowed { option: id }),
            None => (
                None,
                Acted::Escalated {
                    reason: "the policy allows it, but the dialog has no one-time yes".into(),
                },
            ),
        },
        Decision::Deny { rule } => (
            Some(Choice::Decline(Some(denial_message(policy, rule)))),
            Acted::Denied,
        ),
        Decision::Escalate { reason } => (
            None,
            Acted::Escalated {
                reason: reason.clone(),
            },
        ),
    };
    let acted = match choice {
        Some(choice) => {
            match answer::answer_with(db, host, row, seq, choice, POLICY_ANSWERER, None) {
                Ok(report) if report["registered"] == true => acted,
                Ok(_) => Acted::Escalated {
                    reason: "the policy answered, but the dialog stayed open".into(),
                },
                Err(e) => Acted::Escalated {
                    reason: format!("the policy could not answer it: {e}"),
                },
            }
        }
        None => acted,
    };
    let outcome = match &acted {
        Acted::Allowed { .. } => "allowed",
        Acted::Denied => "denied",
        Acted::Escalated { .. } => "escalated",
    };
    let detail = json!({
        "policy": policy.name,
        "kind": dialog.kind,
        "command": dialog.command,
        "decision": decision.as_str(),
        "rule": decision.why(),
        "outcome": outcome,
        "seq": seq,
    })
    .to_string();
    let id = row.id.clone();
    let _ = db.with_conn(move |c| {
        registry::record_event(c, &id, "policy_decision", "supervisor", Some(&detail))
    });
    Some(acted)
}

/// Whether a Drive turn may answer this dialog: not a command dialog of a session that has a
/// policy, since the policy decided it and anything it escalated is the owner's.
pub fn refuse_drive(
    db: &Arc<Database>,
    session_id: &str,
    dialog: &PendingDialog,
) -> Option<String> {
    if dialog.kind != "command" {
        return None;
    }
    let policy = of(db, session_id)?;
    Some(format!(
        "session {session_id} runs under the '{}' command policy, which answers its command dialogs; one it left open waits for the owner, so Drive does not answer it",
        policy.name
    ))
}
