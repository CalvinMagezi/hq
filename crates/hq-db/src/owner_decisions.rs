//! Decisions HQ puts to the owner, each with a recommended default and a deadline. Silence has a
//! meaning: at the deadline a reversible decision takes its default, an irreversible one is held
//! and nothing happens, so no answer never means yes to something that cannot be undone.
//!
//! Created by the runtime or the CLI, and answered only by the owner (`hq decide`), never by a
//! tool a model can call: an answer from a model would be the model deciding for the owner.

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use hq_core::types::{ValueItem, ValueKind};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::Database;

/// The source of the inbox item a decision raises.
pub const SOURCE: &str = "owner_decision";
const MAX_OPTIONS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
    pub default_option: String,
    pub deadline_at: DateTime<Utc>,
    pub reversible: bool,
    pub state: String,
    pub chosen: Option<String>,
}

pub struct NewDecision<'a> {
    pub question: &'a str,
    pub options: &'a [String],
    pub default_option: &'a str,
    pub deadline_at: DateTime<Utc>,
    /// True when going with the default can be undone. Only then does silence mean the default.
    pub reversible: bool,
}

/// Creates a decision and its inbox item. The default must be one of the options.
pub fn create(db: &Database, new: &NewDecision<'_>) -> Result<Decision> {
    if new.question.trim().is_empty() || new.options.len() < 2 || new.options.len() > MAX_OPTIONS {
        bail!("a decision needs a question and 2 to {MAX_OPTIONS} options");
    }
    if !new.options.iter().any(|o| o == new.default_option) {
        bail!("the default must be one of the options");
    }
    let id = Uuid::new_v4().simple().to_string()[..8].to_string();
    let when = if new.reversible {
        format!(
            "If you do not answer by {}, HQ goes with \"{}\".",
            new.deadline_at.format("%Y-%m-%d %H:%M UTC"),
            new.default_option
        )
    } else {
        format!(
            "If you do not answer by {}, HQ does nothing.",
            new.deadline_at.format("%Y-%m-%d %H:%M UTC")
        )
    };
    let item = ValueItem::new(
        SOURCE,
        ValueKind::Fyi,
        format!("Decision: {}", new.question.trim()),
        format!(
            "Options: {}. Recommended: {}. {when}\n\nAnswer with: hq decide answer {id} <option>",
            new.options.join(", "),
            new.default_option
        ),
    )
    .with_dedup_key(format!("owner-decision-{id}"))
    .owner_only(true);
    crate::value_items::insert(db, &item)?;
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO owner_decisions (id, question, options, default_option, deadline_at,
                 reversible, item_id, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                new.question.trim(),
                serde_json::to_string(new.options)?,
                new.default_option,
                new.deadline_at.to_rfc3339(),
                new.reversible,
                item.id,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    })?;
    get(db, &id)?.ok_or_else(|| anyhow::anyhow!("decision vanished after create"))
}

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Decision> {
    let options: String = r.get(2)?;
    let deadline: String = r.get(4)?;
    Ok(Decision {
        id: r.get(0)?,
        question: r.get(1)?,
        options: serde_json::from_str(&options).unwrap_or_default(),
        default_option: r.get(3)?,
        deadline_at: deadline.parse().unwrap_or_else(|_| Utc::now()),
        reversible: r.get::<_, i64>(5)? != 0,
        state: r.get(6)?,
        chosen: r.get(7)?,
    })
}

const COLUMNS: &str =
    "id, question, options, default_option, deadline_at, reversible, state, chosen";

pub fn get(db: &Database, id: &str) -> Result<Option<Decision>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!("SELECT {COLUMNS} FROM owner_decisions WHERE id = ?1"),
            params![id],
            from_row,
        )
        .optional()?)
    })
}

pub fn list_open(db: &Database) -> Result<Vec<Decision>> {
    db.with_conn(|c| {
        Ok(c.prepare(&format!(
            "SELECT {COLUMNS} FROM owner_decisions WHERE state = 'open' ORDER BY deadline_at"
        ))?
        .query_map([], from_row)?
        .collect::<rusqlite::Result<_>>()?)
    })
}

/// The owner's answer. Only an open decision takes one, and only one of its options.
pub fn answer(db: &Database, id: &str, option: &str) -> Result<Decision> {
    let Some(d) = get(db, id)? else {
        bail!("no decision {id}");
    };
    if d.state != "open" {
        bail!("decision {id} is already {}", d.state);
    }
    if !d.options.iter().any(|o| o == option) {
        bail!("choose one of: {}", d.options.join(", "));
    }
    settle(db, &d, "answered", Some(option))?;
    get(db, id)?.ok_or_else(|| anyhow::anyhow!("decision vanished"))
}

/// Settles every open decision past its deadline: a reversible one takes its default, any other is
/// held. Returns what was settled, for the log.
pub fn resolve_due(db: &Database, now: DateTime<Utc>) -> Result<Vec<Decision>> {
    let mut settled = Vec::new();
    for d in list_open(db)? {
        if d.deadline_at > now {
            continue;
        }
        if d.reversible {
            settle(db, &d, "defaulted", Some(&d.default_option))?;
        } else {
            settle(db, &d, "held", None)?;
        }
        if let Some(s) = get(db, &d.id)? {
            settled.push(s);
        }
    }
    Ok(settled)
}

fn settle(db: &Database, d: &Decision, state: &str, chosen: Option<&str>) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE owner_decisions SET state = ?2, chosen = ?3, resolved_at = ?4 WHERE id = ?1 AND state = 'open'",
            params![d.id, state, chosen, Utc::now().to_rfc3339()],
        )?;
        c.execute(
            "UPDATE value_items SET state = 'engaged', engaged_at = ?2 WHERE dedup_key = ?1 AND state IN ('pending','routed','delivered')",
            params![format!("owner-decision-{}", d.id), Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn opts() -> Vec<String> {
        vec!["ship".into(), "wait".into()]
    }

    fn make(db: &Database, reversible: bool, due_in_hours: i64) -> Decision {
        create(
            db,
            &NewDecision {
                question: "Ship the beta?",
                options: &opts(),
                default_option: "wait",
                deadline_at: Utc::now() + Duration::hours(due_in_hours),
                reversible,
            },
        )
        .unwrap()
    }

    #[test]
    fn a_reversible_decision_takes_its_default_at_the_deadline() {
        let db = Database::open_memory().unwrap();
        let d = make(&db, true, -1);
        let settled = resolve_due(&db, Utc::now()).unwrap();
        assert_eq!(settled.len(), 1);
        assert_eq!(
            (settled[0].state.as_str(), settled[0].chosen.as_deref()),
            ("defaulted", Some("wait"))
        );
        assert!(get(&db, &d.id).unwrap().unwrap().chosen.is_some());
    }

    #[test]
    fn an_irreversible_decision_is_held_never_defaulted() {
        let db = Database::open_memory().unwrap();
        make(&db, false, -1);
        let settled = resolve_due(&db, Utc::now()).unwrap();
        assert_eq!(
            (settled[0].state.as_str(), settled[0].chosen.clone()),
            ("held", None)
        );
    }

    #[test]
    fn an_answer_stops_the_clock_and_a_decision_not_yet_due_is_left_alone() {
        let db = Database::open_memory().unwrap();
        let later = make(&db, true, 5);
        assert!(resolve_due(&db, Utc::now()).unwrap().is_empty());
        let done = answer(&db, &later.id, "ship").unwrap();
        assert_eq!(done.chosen.as_deref(), Some("ship"));
        assert!(answer(&db, &later.id, "wait").is_err(), "answered once");
        assert!(
            resolve_due(&db, Utc::now() + Duration::hours(10))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn options_and_the_default_are_checked() {
        let db = Database::open_memory().unwrap();
        let bad = |options: &[String], default: &str| {
            create(
                &db,
                &NewDecision {
                    question: "q",
                    options,
                    default_option: default,
                    deadline_at: Utc::now(),
                    reversible: true,
                },
            )
        };
        assert!(bad(&["only".into()], "only").is_err());
        assert!(bad(&opts(), "other").is_err());
        let d = make(&db, true, 1);
        assert!(answer(&db, &d.id, "maybe").is_err());
    }
}
