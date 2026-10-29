//! The dialog a "waiting for you" inbox item is about (`sql/102_dialog_alerts.sql`), so a
//! notification button answers that exact dialog. The session supervisor writes a row with the
//! alert; the notifications API and the web push dispatcher read it; the first answer claims it.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::Database;

/// Rows older than this are dropped when a new one is written; their items have long expired.
const KEEP_DAYS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogOption {
    pub id: u32,
    pub label: String,
    /// Picking it lets the agent do this again without asking.
    #[serde(default)]
    pub remember: bool,
    /// Picking it opens a text field instead of answering.
    #[serde(default)]
    pub typed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DialogAlert {
    pub item_id: String,
    pub session_id: String,
    /// The dialog's number on its host; the answer names it.
    pub pending_seq: u64,
    pub kind: String,
    pub question: Option<String>,
    pub command: Option<String>,
    pub options: Vec<DialogOption>,
    /// The option a button may pick, if any (see [`approve_option`]).
    pub approve_option: Option<u32>,
    /// Someone already answered it from a notification.
    pub answered: bool,
}

impl DialogAlert {
    /// The label of the option a button may pick.
    pub fn approve_label(&self) -> Option<&str> {
        let id = self.approve_option?;
        self.options
            .iter()
            .find(|o| o.id == id)
            .map(|o| o.label.as_str())
    }
}

/// Words an option a button may pick starts with. Anything else (a choice between answers, a
/// hooks review) is left to the owner on the agent's own page.
const APPROVING: &[&str] = &["yes", "allow", "approve", "accept"];

/// The one option a notification button may pick: it answers once (it is not a "remember"
/// option), it opens no text field, and it reads as a yes. `None` when there is no such option,
/// or the agent is asking a question rather than for permission.
pub fn approve_option(kind: &str, options: &[DialogOption]) -> Option<u32> {
    if kind == "question" {
        return None;
    }
    options
        .iter()
        .filter(|o| !o.remember && !o.typed)
        .find(|o| {
            let label = o.label.trim().to_lowercase();
            APPROVING.iter().any(|w| {
                label
                    .strip_prefix(w)
                    .is_some_and(|rest| rest.is_empty() || !rest.starts_with(char::is_alphanumeric))
            })
        })
        .map(|o| o.id)
}

/// What the supervisor knows about a dialog when it raises the alert.
pub struct NewDialogAlert<'a> {
    pub item_id: &'a str,
    pub session_id: &'a str,
    pub pending_seq: u64,
    pub kind: &'a str,
    pub question: Option<&'a str>,
    pub command: Option<&'a str>,
    pub options: &'a [DialogOption],
}

/// Records the dialog behind inbox item `item_id`. A second record for the same item is ignored.
pub fn record(db: &Database, alert: &NewDialogAlert<'_>) -> Result<()> {
    let options = serde_json::to_string(alert.options)?;
    let approve = approve_option(alert.kind, alert.options);
    db.with_conn(|c| {
        c.execute(
            "DELETE FROM dialog_alerts WHERE created_at < datetime('now', ?1)",
            params![format!("-{KEEP_DAYS} days")],
        )?;
        c.execute(
            "INSERT OR IGNORE INTO dialog_alerts \
             (item_id, session_id, pending_seq, kind, question, command, options, approve_option) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                alert.item_id,
                alert.session_id,
                alert.pending_seq as i64,
                alert.kind,
                alert.question,
                alert.command,
                options,
                approve,
            ],
        )?;
        Ok(())
    })
}

/// The dialog behind inbox item `item_id`, if it has one.
pub fn get(db: &Database, item_id: &str) -> Result<Option<DialogAlert>> {
    db.with_conn(|c| {
        let row = c
            .query_row(
                "SELECT item_id, session_id, pending_seq, kind, question, command, options, \
                 approve_option, answered_at IS NOT NULL FROM dialog_alerts WHERE item_id = ?1",
                params![item_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, Option<i64>>(7)?,
                        r.get::<_, bool>(8)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(
            |(item_id, session_id, seq, kind, question, command, options, approve, answered)| {
                DialogAlert {
                    item_id,
                    session_id,
                    pending_seq: seq as u64,
                    kind,
                    question,
                    command,
                    options: serde_json::from_str(&options).unwrap_or_default(),
                    approve_option: approve.map(|a| a as u32),
                    answered,
                }
            },
        ))
    })
}

/// A dialog still waiting for the owner: its alert is in the inbox and nobody has answered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenDecision {
    pub item_id: String,
    /// The alert's title ("Harness session '...' is waiting for you").
    pub title: String,
    pub asked_at: String,
    pub alert: DialogAlert,
}

/// The dialogs of `sessions` still waiting for the owner, oldest first.
pub fn open_for_sessions(db: &Database, sessions: &[String]) -> Result<Vec<OpenDecision>> {
    if sessions.is_empty() {
        return Ok(Vec::new());
    }
    let marks = (0..sessions.len())
        .map(|i| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT d.item_id, v.title, v.created_at FROM dialog_alerts d \
         JOIN value_items v ON v.id = d.item_id \
         WHERE d.answered_at IS NULL AND v.state IN ('pending', 'routed', 'delivered') \
         AND d.session_id IN ({marks}) ORDER BY v.created_at, d.item_id"
    );
    let found: Vec<(String, String, String)> = db.with_conn(|c| {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(sessions.iter()), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })?;
    let mut out = Vec::new();
    for (item_id, title, asked_at) in found {
        if let Some(alert) = get(db, &item_id)? {
            out.push(OpenDecision {
                item_id,
                title,
                asked_at,
                alert,
            });
        }
    }
    Ok(out)
}

/// Claims the dialog for one answer. False when it was already answered, so a second tap or a
/// replayed token answers nothing.
pub fn claim(db: &Database, item_id: &str, answer: &str) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE dialog_alerts SET answered_at = datetime('now'), answer = ?2 \
             WHERE item_id = ?1 AND answered_at IS NULL",
            params![item_id, answer],
        )?;
        Ok(n == 1)
    })
}

/// Gives a claim back when the answer could not be delivered (the host did not answer), so it
/// can be tried again. Not used when the dialog moved on: that dialog is gone for good.
pub fn release(db: &Database, item_id: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE dialog_alerts SET answered_at = NULL, answer = NULL WHERE item_id = ?1",
            params![item_id],
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: u32, label: &str, remember: bool, typed: bool) -> DialogOption {
        DialogOption {
            id,
            label: label.into(),
            remember,
            typed,
        }
    }

    fn copilot_path() -> Vec<DialogOption> {
        vec![
            option(1, "Yes", false, false),
            option(
                2,
                "Yes, and remember this path for this session",
                true,
                false,
            ),
            option(3, "No (Esc)", false, false),
        ]
    }

    #[test]
    fn a_button_picks_only_a_plain_yes_once() {
        assert_eq!(approve_option("path", &copilot_path()), Some(1));
        // Claude's trust dialog lists "No, exit" first.
        let trust = [
            option(1, "No, exit", false, false),
            option(2, "Yes, I trust this folder", false, false),
        ];
        assert_eq!(approve_option("trust", &trust), Some(2));
        // Codex's hooks review has no yes; trusting all is a remember option.
        let hooks = [
            option(1, "Review hooks", false, false),
            option(2, "Trust all and continue", true, false),
            option(
                3,
                "Continue without trusting (hooks won't run)",
                false,
                false,
            ),
        ];
        assert_eq!(approve_option("trust", &hooks), None);
        let only_remember = [option(1, "Yes, always", true, false)];
        assert_eq!(approve_option("command", &only_remember), None);
        assert_eq!(
            approve_option("command", &[option(1, "Yesterday", false, false)]),
            None
        );
        let question = [
            option(1, "Yes", false, false),
            option(2, "No", false, false),
        ];
        assert_eq!(approve_option("question", &question), None);
    }

    #[test]
    fn open_decisions_are_the_unanswered_alerts_still_in_the_inbox() {
        let db = Database::open_memory().unwrap();
        let options = copilot_path();
        for (item, session) in [("item-a", "hs-1"), ("item-b", "hs-1"), ("item-c", "hs-2")] {
            let v = hq_core::types::ValueItem {
                id: item.into(),
                ..hq_core::types::ValueItem::new(
                    "session-supervisor",
                    hq_core::types::ValueKind::ActionNeeded,
                    "waiting",
                    "body",
                )
            };
            crate::value_items::insert(&db, &v).unwrap();
            record(
                &db,
                &NewDialogAlert {
                    item_id: item,
                    session_id: session,
                    pending_seq: 1,
                    kind: "path",
                    question: Some("Allow?"),
                    command: Some("/tmp"),
                    options: &options,
                },
            )
            .unwrap();
        }
        claim(&db, "item-b", "approve").unwrap();
        crate::value_items::dismiss_by_id(&db, "item-c").unwrap();
        let open = open_for_sessions(&db, &["hs-1".into(), "hs-2".into()]).unwrap();
        let ids: Vec<&str> = open.iter().map(|d| d.item_id.as_str()).collect();
        assert_eq!(ids, ["item-a"], "answered and dismissed ones are not open");
        assert!(open_for_sessions(&db, &[]).unwrap().is_empty());
    }

    #[test]
    fn an_alert_is_recorded_once_read_back_and_answered_once() {
        let db = Database::open_memory().unwrap();
        let options = copilot_path();
        let alert = NewDialogAlert {
            item_id: "item-1",
            session_id: "hs-1",
            pending_seq: 42,
            kind: "path",
            question: Some("Do you want to allow this?"),
            command: Some("/tmp"),
            options: &options,
        };
        record(&db, &alert).unwrap();
        record(&db, &alert).unwrap();
        let got = get(&db, "item-1").unwrap().unwrap();
        assert_eq!(got.pending_seq, 42);
        assert_eq!(got.options, options);
        assert_eq!(got.approve_label(), Some("Yes"));
        assert!(!got.answered);
        assert!(get(&db, "nope").unwrap().is_none());

        assert!(claim(&db, "item-1", "approve").unwrap());
        assert!(!claim(&db, "item-1", "decline").unwrap(), "answered once");
        assert!(get(&db, "item-1").unwrap().unwrap().answered);
        release(&db, "item-1").unwrap();
        assert!(claim(&db, "item-1", "decline").unwrap());
    }
}
