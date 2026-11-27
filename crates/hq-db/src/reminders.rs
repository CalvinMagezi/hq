//! Reminders the owner set (`sql/156_reminders.sql`): what to say, when, how often, and whether to
//! keep after them until they answer. This module only stores and moves rows. When a reminder is
//! next due, and how a repeat or a nudge advances, is decided by `hq_tools::reminders::schedule`,
//! which passes the result in. Nothing here calls a model.

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, Row, params};
use uuid::Uuid;

use crate::Database;

pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_DONE: &str = "done";
pub const STATUS_CANCELLED: &str = "cancelled";
/// A nudge that pinged every time on its ladder and was never answered.
pub const STATUS_LAPSED: &str = "lapsed";

pub const OUTCOME_FIRED: &str = "fired";
pub const OUTCOME_DONE: &str = "done";
pub const OUTCOME_SNOOZED: &str = "snoozed";
pub const OUTCOME_DISMISSED: &str = "dismissed";
pub const OUTCOME_SUPERSEDED: &str = "superseded";
pub const OUTCOME_MISSED: &str = "missed";

const MAX_TITLE_CHARS: usize = 200;
const MAX_NOTE_CHARS: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: String,
    pub title: String,
    pub note: Option<String>,
    pub status: String,
    /// When the next ping is due (UTC epoch seconds).
    pub due_at: i64,
    /// The time the owner meant, which a nudge's follow-up pings are measured from.
    pub occurrence_at: i64,
    pub tz: String,
    /// JSON of `hq_tools::reminders::schedule::Repeat`, or None for a reminder that happens once.
    pub repeat_rule: Option<String>,
    pub nudge: bool,
    /// Pings sent for the current occurrence.
    pub nudge_step: i64,
    /// Pings sent over the reminder's life; part of each inbox item's dedup key.
    pub fire_seq: i64,
    pub last_item_id: Option<String>,
    pub task_id: Option<String>,
    pub thread_id: Option<String>,
    /// Set while a session that had read outside text made it. Shown to the owner, and it carries no link.
    pub tainted: bool,
    pub source: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_fired_at: Option<i64>,
    pub acked_at: Option<i64>,
}

const COLS: &str = "id, title, note, status, due_at, occurrence_at, tz, repeat_rule, nudge, nudge_step, fire_seq, last_item_id, task_id, thread_id, tainted, source, created_at, updated_at, last_fired_at, acked_at";

fn map(r: &Row<'_>) -> rusqlite::Result<Reminder> {
    Ok(Reminder {
        id: r.get(0)?,
        title: r.get(1)?,
        note: r.get(2)?,
        status: r.get(3)?,
        due_at: r.get(4)?,
        occurrence_at: r.get(5)?,
        tz: r.get(6)?,
        repeat_rule: r.get(7)?,
        nudge: r.get::<_, i64>(8)? != 0,
        nudge_step: r.get(9)?,
        fire_seq: r.get(10)?,
        last_item_id: r.get(11)?,
        task_id: r.get(12)?,
        thread_id: r.get(13)?,
        tainted: r.get::<_, i64>(14)? != 0,
        source: r.get(15)?,
        created_at: r.get(16)?,
        updated_at: r.get(17)?,
        last_fired_at: r.get(18)?,
        acked_at: r.get(19)?,
    })
}

pub struct NewReminder<'a> {
    pub title: &'a str,
    pub note: Option<&'a str>,
    pub due_at: i64,
    pub tz: &'a str,
    pub repeat_rule: Option<&'a str>,
    pub nudge: bool,
    pub task_id: Option<&'a str>,
    pub thread_id: Option<&'a str>,
    pub tainted: bool,
    pub source: &'a str,
}

/// How many reminders may exist, so a runaway producer cannot fill the owner's phone.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_active: i64,
    pub max_per_hour: i64,
}

fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

/// Creates a reminder. Refuses an empty title and anything past `limits`.
pub fn create(db: &Database, new: &NewReminder<'_>, now: i64, limits: Limits) -> Result<Reminder> {
    let title = clip(new.title, MAX_TITLE_CHARS);
    if title.is_empty() {
        bail!("a reminder needs a title");
    }
    if count_active(db)? >= limits.max_active {
        bail!(
            "there are already {} active reminders; finish or cancel some first",
            limits.max_active
        );
    }
    if count_created_since(db, now - 3600)? >= limits.max_per_hour {
        bail!("too many reminders created in the last hour; try again later");
    }
    let id = Uuid::new_v4().simple().to_string()[..12].to_string();
    let note = new
        .note
        .map(|n| clip(n, MAX_NOTE_CHARS))
        .filter(|n| !n.is_empty());
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO reminders (id, title, note, status, due_at, occurrence_at, tz, repeat_rule, nudge, \
             task_id, thread_id, tainted, source, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'active', ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            params![
                id,
                title,
                note,
                new.due_at,
                new.tz,
                new.repeat_rule,
                new.nudge,
                new.task_id,
                new.thread_id,
                new.tainted,
                new.source,
                now
            ],
        )?;
        Ok(())
    })?;
    get(db, &id)?.ok_or_else(|| anyhow::anyhow!("reminder {id} vanished after insert"))
}

pub fn get(db: &Database, id: &str) -> Result<Option<Reminder>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!("SELECT {COLS} FROM reminders WHERE id = ?1"),
            params![id],
            map,
        )
        .optional()?)
    })
}

/// The reminder whose latest ping is the inbox item `item_id`.
pub fn get_by_item(db: &Database, item_id: &str) -> Result<Option<Reminder>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!("SELECT {COLS} FROM reminders WHERE last_item_id = ?1"),
            params![item_id],
            map,
        )
        .optional()?)
    })
}

/// Reminders in any of `statuses` (all when empty), soonest first, at most `limit`.
pub fn list(db: &Database, statuses: &[&str], limit: usize) -> Result<Vec<Reminder>> {
    db.with_conn(|c| {
        let wanted = if statuses.is_empty() {
            "1".to_string()
        } else {
            let quoted: Vec<String> = statuses
                .iter()
                .map(|s| format!("'{}'", s.replace('\'', "")))
                .collect();
            format!("status IN ({})", quoted.join(","))
        };
        let mut stmt = c.prepare(&format!(
            "SELECT {COLS} FROM reminders WHERE {wanted} ORDER BY due_at ASC, created_at ASC LIMIT ?1"
        ))?;
        let rows = stmt
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], map)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Active reminders whose next ping is due at or before `now`, oldest first.
pub fn due(db: &Database, now: i64, limit: usize) -> Result<Vec<Reminder>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {COLS} FROM reminders WHERE status = 'active' AND due_at <= ?1 \
             ORDER BY due_at ASC LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![now, i64::try_from(limit).unwrap_or(i64::MAX)], map)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

pub fn count_active(db: &Database) -> Result<i64> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT COUNT(*) FROM reminders WHERE status = 'active'",
            [],
            |r| r.get(0),
        )?)
    })
}

pub fn count_created_since(db: &Database, since: i64) -> Result<i64> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT COUNT(*) FROM reminders WHERE created_at >= ?1",
            params![since],
            |r| r.get(0),
        )?)
    })
}

/// What a ping changes on its reminder. `status` is `active` while more pings are due.
pub struct Fired<'a> {
    pub item_id: &'a str,
    pub status: &'a str,
    pub next_due_at: i64,
    pub occurrence_at: i64,
    pub nudge_step: i64,
}

/// Records one ping: moves the reminder to its next due time (or ends it) and logs the firing.
/// The previous ping, if nobody answered it, is logged as superseded.
pub fn record_fire(db: &Database, id: &str, now: i64, fired: &Fired<'_>) -> Result<()> {
    db.with_conn(|c| {
        let tx = rusqlite::Transaction::new_unchecked(c, rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE reminders SET fire_seq = fire_seq + 1, last_fired_at = ?2, last_item_id = ?3, \
             status = ?4, due_at = ?5, occurrence_at = ?6, nudge_step = ?7, updated_at = ?2 \
             WHERE id = ?1 AND status = 'active'",
            params![
                id,
                now,
                fired.item_id,
                fired.status,
                fired.next_due_at,
                fired.occurrence_at,
                fired.nudge_step
            ],
        )?;
        if changed == 0 {
            return Ok(());
        }
        let seq: i64 = tx.query_row(
            "SELECT fire_seq FROM reminders WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE reminder_firings SET outcome = 'superseded' \
             WHERE reminder_id = ?1 AND outcome = 'fired'",
            params![id],
        )?;
        tx.execute(
            "INSERT INTO reminder_firings (reminder_id, seq, fired_at, item_id, outcome) \
             VALUES (?1, ?2, ?3, ?4, 'fired')",
            params![id, seq, now, fired.item_id],
        )?;
        tx.commit()?;
        Ok(())
    })
}

fn set_latest_outcome(c: &rusqlite::Connection, id: &str, outcome: &str) -> Result<()> {
    c.execute(
        "UPDATE reminder_firings SET outcome = ?2 WHERE id = \
         (SELECT id FROM reminder_firings WHERE reminder_id = ?1 AND outcome = 'fired' \
          ORDER BY seq DESC LIMIT 1)",
        params![id, outcome],
    )?;
    Ok(())
}

/// The owner is done with this occurrence. `next_occurrence` is the next time a repeating reminder
/// is due; None ends the reminder. False when the reminder is not active.
pub fn acknowledge(
    db: &Database,
    id: &str,
    now: i64,
    next_occurrence: Option<i64>,
) -> Result<bool> {
    db.with_conn(|c| {
        let n = match next_occurrence {
            Some(next) => c.execute(
                "UPDATE reminders SET due_at = ?2, occurrence_at = ?2, nudge_step = 0, acked_at = ?3, \
                 updated_at = ?3 WHERE id = ?1 AND status = 'active'",
                params![id, next, now],
            )?,
            None => c.execute(
                "UPDATE reminders SET status = 'done', acked_at = ?2, updated_at = ?2 \
                 WHERE id = ?1 AND status = 'active'",
                params![id, now],
            )?,
        };
        if n > 0 {
            set_latest_outcome(c, id, OUTCOME_DONE)?;
        }
        Ok(n > 0)
    })
}

/// Moves the next ping to `until`. A nudge starts its ladder over from there.
pub fn snooze(db: &Database, id: &str, until: i64, now: i64) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE reminders SET due_at = ?2, nudge_step = 0, updated_at = ?3 \
             WHERE id = ?1 AND status = 'active'",
            params![id, until, now],
        )?;
        if n > 0 {
            set_latest_outcome(c, id, OUTCOME_SNOOZED)?;
        }
        Ok(n > 0)
    })
}

/// Stops a reminder for good (`cancelled`), with the outcome of its last ping.
pub fn cancel(db: &Database, id: &str, now: i64, outcome: &str) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE reminders SET status = 'cancelled', updated_at = ?2 \
             WHERE id = ?1 AND status = 'active'",
            params![id, now],
        )?;
        if n > 0 {
            set_latest_outcome(c, id, outcome)?;
        }
        Ok(n > 0)
    })
}

/// Edits an active reminder. A new `due_at` also becomes the occurrence its follow-ups count from.
#[derive(Default)]
pub struct Patch<'a> {
    pub title: Option<&'a str>,
    pub note: Option<Option<&'a str>>,
    pub due_at: Option<i64>,
    pub repeat_rule: Option<Option<&'a str>>,
    pub nudge: Option<bool>,
}

pub fn update(db: &Database, id: &str, patch: &Patch<'_>, now: i64) -> Result<bool> {
    let Some(current) = get(db, id)? else {
        return Ok(false);
    };
    if current.status != STATUS_ACTIVE {
        return Ok(false);
    }
    let title = match patch.title {
        Some(t) => {
            let t = clip(t, MAX_TITLE_CHARS);
            if t.is_empty() {
                bail!("a reminder needs a title");
            }
            t
        }
        None => current.title.clone(),
    };
    let note = match patch.note {
        Some(n) => n.map(|n| clip(n, MAX_NOTE_CHARS)).filter(|n| !n.is_empty()),
        None => current.note.clone(),
    };
    let (due_at, occurrence_at, step) = match patch.due_at {
        Some(due) => (due, due, 0),
        None => (current.due_at, current.occurrence_at, current.nudge_step),
    };
    let repeat = match patch.repeat_rule {
        Some(r) => r.map(str::to_string),
        None => current.repeat_rule.clone(),
    };
    let nudge = patch.nudge.unwrap_or(current.nudge);
    db.with_conn(|c| {
        c.execute(
            "UPDATE reminders SET title = ?2, note = ?3, due_at = ?4, occurrence_at = ?5, nudge_step = ?6, \
             repeat_rule = ?7, nudge = ?8, updated_at = ?9 WHERE id = ?1 AND status = 'active'",
            params![id, title, note, due_at, occurrence_at, step, repeat, nudge, now],
        )?;
        Ok(true)
    })
}

/// Brings a cancelled reminder back, as the inbox's Undo does. Its due time stays as it was.
pub fn restore(db: &Database, id: &str, now: i64) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE reminders SET status = 'active', updated_at = ?2 WHERE id = ?1 AND status = 'cancelled'",
            params![id, now],
        )? > 0)
    })
}

/// How a ping ended, by reminder id, newest first. For `hq reminders` and `hq doctor`.
pub fn firings(db: &Database, id: &str, limit: usize) -> Result<Vec<(i64, i64, String)>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT seq, fired_at, outcome FROM reminder_firings WHERE reminder_id = ?1 \
             ORDER BY seq DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![id, i64::try_from(limit).unwrap_or(i64::MAX)], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000;
    const LIMITS: Limits = Limits {
        max_active: 200,
        max_per_hour: 30,
    };

    fn new(title: &str) -> NewReminder<'_> {
        NewReminder {
            title,
            note: None,
            due_at: T0 + 600,
            tz: "Africa/Kampala",
            repeat_rule: None,
            nudge: false,
            task_id: None,
            thread_id: None,
            tainted: false,
            source: "chat",
        }
    }

    fn fired(item: &str, next: i64, step: i64) -> Fired<'_> {
        Fired {
            item_id: item,
            status: STATUS_ACTIVE,
            next_due_at: next,
            occurrence_at: T0 + 600,
            nudge_step: step,
        }
    }

    #[test]
    fn a_reminder_is_created_trimmed_and_found_when_due() {
        let db = Database::open_memory().unwrap();
        let r = create(&db, &new("  Call Moses  "), T0, LIMITS).unwrap();
        assert_eq!(r.title, "Call Moses");
        assert_eq!(r.status, STATUS_ACTIVE);
        assert!(due(&db, T0 + 599, 10).unwrap().is_empty());
        assert_eq!(due(&db, T0 + 600, 10).unwrap().len(), 1);
    }

    #[test]
    fn an_empty_title_and_the_limits_are_refused() {
        let db = Database::open_memory().unwrap();
        assert!(create(&db, &new("   "), T0, LIMITS).is_err());
        let tight = Limits {
            max_active: 1,
            max_per_hour: 30,
        };
        create(&db, &new("a"), T0, tight).unwrap();
        assert!(create(&db, &new("b"), T0, tight).is_err());
        let hourly = Limits {
            max_active: 100,
            max_per_hour: 1,
        };
        assert!(create(&db, &new("c"), T0 + 10, hourly).is_err());
        // An hour later the hourly count has rolled off.
        assert!(create(&db, &new("c"), T0 + 4000, hourly).is_ok());
    }

    #[test]
    fn a_ping_moves_the_due_time_logs_a_firing_and_supersedes_the_last() {
        let db = Database::open_memory().unwrap();
        let r = create(&db, &new("x"), T0, LIMITS).unwrap();
        record_fire(&db, &r.id, T0 + 600, &fired("it1", T0 + 2400, 1)).unwrap();
        record_fire(&db, &r.id, T0 + 2400, &fired("it2", T0 + 9600, 2)).unwrap();
        let after = get(&db, &r.id).unwrap().unwrap();
        assert_eq!(after.fire_seq, 2);
        assert_eq!(after.last_item_id.as_deref(), Some("it2"));
        assert_eq!(after.due_at, T0 + 9600);
        let log = firings(&db, &r.id, 10).unwrap();
        assert_eq!(log[0].2, OUTCOME_FIRED);
        assert_eq!(log[1].2, OUTCOME_SUPERSEDED);
        assert_eq!(get_by_item(&db, "it2").unwrap().unwrap().id, r.id);
    }

    #[test]
    fn done_ends_a_one_shot_and_rolls_a_repeat_forward() {
        let db = Database::open_memory().unwrap();
        let once = create(&db, &new("once"), T0, LIMITS).unwrap();
        record_fire(&db, &once.id, T0 + 600, &fired("a", T0 + 2400, 1)).unwrap();
        assert!(acknowledge(&db, &once.id, T0 + 700, None).unwrap());
        assert_eq!(get(&db, &once.id).unwrap().unwrap().status, STATUS_DONE);
        assert_eq!(firings(&db, &once.id, 1).unwrap()[0].2, OUTCOME_DONE);
        // A second answer to the same reminder changes nothing.
        assert!(!acknowledge(&db, &once.id, T0 + 800, None).unwrap());

        let daily = create(&db, &new("daily"), T0, LIMITS).unwrap();
        record_fire(&db, &daily.id, T0 + 600, &fired("b", T0 + 2400, 1)).unwrap();
        assert!(acknowledge(&db, &daily.id, T0 + 700, Some(T0 + 86_400)).unwrap());
        let after = get(&db, &daily.id).unwrap().unwrap();
        assert_eq!(after.status, STATUS_ACTIVE);
        assert_eq!(after.due_at, T0 + 86_400);
        assert_eq!(after.nudge_step, 0);
    }

    #[test]
    fn snooze_moves_the_next_ping_and_restarts_the_ladder() {
        let db = Database::open_memory().unwrap();
        let r = create(&db, &new("x"), T0, LIMITS).unwrap();
        record_fire(&db, &r.id, T0 + 600, &fired("a", T0 + 2400, 2)).unwrap();
        assert!(snooze(&db, &r.id, T0 + 5000, T0 + 700).unwrap());
        let after = get(&db, &r.id).unwrap().unwrap();
        assert_eq!((after.due_at, after.nudge_step), (T0 + 5000, 0));
        assert_eq!(firings(&db, &r.id, 1).unwrap()[0].2, OUTCOME_SNOOZED);
    }

    #[test]
    fn cancel_and_restore_round_trip_and_only_touch_the_right_status() {
        let db = Database::open_memory().unwrap();
        let r = create(&db, &new("x"), T0, LIMITS).unwrap();
        assert!(!restore(&db, &r.id, T0).unwrap());
        assert!(cancel(&db, &r.id, T0 + 1, OUTCOME_DISMISSED).unwrap());
        assert!(due(&db, T0 + 10_000, 10).unwrap().is_empty());
        assert!(!cancel(&db, &r.id, T0 + 2, OUTCOME_DISMISSED).unwrap());
        assert!(restore(&db, &r.id, T0 + 3).unwrap());
        assert_eq!(due(&db, T0 + 10_000, 10).unwrap().len(), 1);
    }

    #[test]
    fn an_edit_to_the_time_restarts_the_occurrence() {
        let db = Database::open_memory().unwrap();
        let r = create(&db, &new("x"), T0, LIMITS).unwrap();
        record_fire(&db, &r.id, T0 + 600, &fired("a", T0 + 2400, 3)).unwrap();
        let patch = Patch {
            title: Some("renamed"),
            due_at: Some(T0 + 50_000),
            nudge: Some(true),
            ..Patch::default()
        };
        assert!(update(&db, &r.id, &patch, T0 + 700).unwrap());
        let after = get(&db, &r.id).unwrap().unwrap();
        assert_eq!(after.title, "renamed");
        assert_eq!(
            (after.due_at, after.occurrence_at, after.nudge_step),
            (T0 + 50_000, T0 + 50_000, 0)
        );
        assert!(after.nudge);
        assert!(
            update(&db, "missing", &Patch::default(), T0)
                .map(|ok| !ok)
                .unwrap()
        );
    }

    #[test]
    fn list_filters_by_status() {
        let db = Database::open_memory().unwrap();
        let a = create(&db, &new("a"), T0, LIMITS).unwrap();
        create(&db, &new("b"), T0, LIMITS).unwrap();
        cancel(&db, &a.id, T0 + 1, OUTCOME_DISMISSED).unwrap();
        assert_eq!(list(&db, &[STATUS_ACTIVE], 10).unwrap().len(), 1);
        assert_eq!(list(&db, &[], 10).unwrap().len(), 2);
    }
}
