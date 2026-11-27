//! What Done, Snooze and Dismiss do to a reminder's inbox item. The inbox item is the ping; the
//! reminder behind it (`hq_db::reminders`) is what moves. Kept out of `notifications_api.rs`,
//! which only routes the action here.

use anyhow::{Result, bail};
use hq_core::types::REMINDER_SOURCE;
use hq_db::Database;
use hq_db::reminders::{self, OUTCOME_DISMISSED};
use hq_tools::reminders::schedule::after_done;

pub(crate) const DONE: &str = "done";
pub(crate) const SNOOZE: &str = "snooze";
const DEFAULT_SNOOZE_MINUTES: i64 = 60;
const MAX_SNOOZE_SECS: i64 = 90 * 24 * 3600;

/// The actions a reminder's ping offers while it waits.
pub(crate) const PENDING_ACTIONS: &[&str] = &[DONE, SNOOZE, "dismiss"];

/// Whether `action` is one only a reminder's ping takes.
pub(crate) fn is_reminder_action(action: &str) -> bool {
    matches!(action, DONE | SNOOZE)
}

/// Whether the value item `val_id` is a reminder's ping.
pub(crate) fn is_reminder_item(db: &Database, val_id: &str) -> bool {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT source_task FROM value_items WHERE id = ?1",
            [val_id],
            |r| r.get::<_, String>(0),
        )
        .map(|s| s == REMINDER_SOURCE)
        .unwrap_or(false))
    })
    .unwrap_or(false)
}

/// When a snooze ends: `until` (epoch seconds) when given, else `minutes` from now, else an hour.
/// Refuses a time in the past or more than 90 days out, before anything is recorded.
pub(crate) fn snooze_until(minutes: Option<i64>, until: Option<i64>, now: i64) -> Result<i64> {
    let at = match (until, minutes) {
        (Some(at), _) => at,
        (None, Some(m)) if m >= 1 => now + m.saturating_mul(60),
        (None, Some(_)) => bail!("snooze for at least one minute"),
        (None, None) => now + DEFAULT_SNOOZE_MINUTES * 60,
    };
    if at <= now {
        bail!("that time has already passed");
    }
    if at - now > MAX_SNOOZE_SECS {
        bail!("snooze for 90 days at most");
    }
    Ok(at)
}

/// Applies the owner's answer to the reminder whose latest ping is `val_id`. A ping that was
/// already replaced by a newer one has no reminder to move, and that is not an error.
pub(crate) fn apply(db: &Database, val_id: &str, action: &str, until: i64, now: i64) -> Result<()> {
    let Some(reminder) = reminders::get_by_item(db, val_id)? else {
        return Ok(());
    };
    match action {
        DONE => {
            reminders::acknowledge(db, &reminder.id, now, after_done(&reminder, now))?;
        }
        SNOOZE => {
            reminders::snooze(db, &reminder.id, until, now)?;
        }
        // Clearing today's ping of a repeat must not end the repeat, so it only answers this one.
        "dismiss" | "reject" => match after_done(&reminder, now) {
            Some(next) => {
                reminders::acknowledge(db, &reminder.id, now, Some(next))?;
            }
            None => {
                reminders::cancel(db, &reminder.id, now, OUTCOME_DISMISSED)?;
            }
        },
        _ => {}
    }
    Ok(())
}

/// The Undo of a dismissed ping: a reminder that was stopped by it comes back.
pub(crate) fn undo_dismiss(db: &Database, val_id: &str, now: i64) -> Result<()> {
    if let Some(reminder) = reminders::get_by_item(db, val_id)? {
        reminders::restore(db, &reminder.id, now)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::RemindersConfig;
    use hq_core::types::{ValueItem, ValueKind};
    use hq_db::reminders::{Limits, NewReminder, STATUS_ACTIVE, STATUS_CANCELLED, STATUS_DONE};
    use hq_tools::reminders::fire::fire_due;
    use hq_tools::reminders::schedule::Repeat;

    const T0: i64 = 1_800_000_000;

    fn ringing(nudge: bool, repeat: Option<Repeat>) -> (Database, String, String) {
        let db = Database::open_memory().unwrap();
        let rule = repeat.map(|r| r.to_json());
        let r = reminders::create(
            &db,
            &NewReminder {
                title: "Call Moses",
                note: None,
                due_at: T0,
                tz: "Africa/Kampala",
                repeat_rule: rule.as_deref(),
                nudge,
                task_id: None,
                thread_id: None,
                tainted: false,
                source: "chat",
            },
            T0 - 60,
            Limits {
                max_active: 10,
                max_per_hour: 10,
            },
        )
        .unwrap();
        fire_due(&db, &RemindersConfig::default(), T0).unwrap();
        let item = reminders::get(&db, &r.id)
            .unwrap()
            .unwrap()
            .last_item_id
            .unwrap();
        (db, r.id, item)
    }

    #[test]
    fn only_a_reminders_ping_takes_done_and_snooze() {
        let (db, _, item) = ringing(true, None);
        assert!(is_reminder_item(&db, &item));
        let other = ValueItem::new("agent_notify", ValueKind::Fyi, "x", "y");
        hq_db::value_items::emit(&db, &other).unwrap();
        assert!(!is_reminder_item(&db, &other.id));
        assert!(is_reminder_action("done") && is_reminder_action("snooze"));
        assert!(!is_reminder_action("dismiss"));
    }

    #[test]
    fn done_stops_a_nudge() {
        let (db, id, item) = ringing(true, None);
        apply(&db, &item, DONE, 0, T0 + 30).unwrap();
        assert_eq!(
            reminders::get(&db, &id).unwrap().unwrap().status,
            STATUS_DONE
        );
        assert!(
            reminders::due(&db, T0 + 10 * 86_400, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn snooze_moves_the_next_ping_to_the_chosen_time() {
        let (db, id, item) = ringing(true, None);
        apply(&db, &item, SNOOZE, T0 + 3600, T0 + 30).unwrap();
        let r = reminders::get(&db, &id).unwrap().unwrap();
        assert_eq!(
            (r.status.as_str(), r.due_at, r.nudge_step),
            (STATUS_ACTIVE, T0 + 3600, 0)
        );
    }

    #[test]
    fn done_on_a_repeat_only_answers_this_occurrence() {
        let (db, id, item) = ringing(false, Some(Repeat::Daily { every: 1 }));
        apply(&db, &item, DONE, 0, T0 + 30).unwrap();
        let r = reminders::get(&db, &id).unwrap().unwrap();
        assert_eq!(r.status, STATUS_ACTIVE);
        assert!(r.due_at > T0);
    }

    #[test]
    fn dismissing_a_nudge_stops_it_and_undo_brings_it_back_but_a_repeat_survives_a_dismiss() {
        let (db, id, item) = ringing(true, None);
        apply(&db, &item, "dismiss", 0, T0 + 30).unwrap();
        assert_eq!(
            reminders::get(&db, &id).unwrap().unwrap().status,
            STATUS_CANCELLED
        );
        undo_dismiss(&db, &item, T0 + 40).unwrap();
        assert_eq!(
            reminders::get(&db, &id).unwrap().unwrap().status,
            STATUS_ACTIVE
        );

        let (db, id, item) = ringing(true, Some(Repeat::Daily { every: 1 }));
        apply(&db, &item, "dismiss", 0, T0 + 30).unwrap();
        assert_eq!(
            reminders::get(&db, &id).unwrap().unwrap().status,
            STATUS_ACTIVE
        );
    }

    #[test]
    fn an_answer_to_a_ping_that_a_newer_one_replaced_changes_nothing() {
        let (db, id, item) = ringing(true, None);
        apply(&db, "val-that-is-gone", DONE, 0, T0 + 30).unwrap();
        assert_eq!(
            reminders::get(&db, &id).unwrap().unwrap().status,
            STATUS_ACTIVE
        );
        let _ = item;
    }

    #[test]
    fn a_snooze_is_checked_before_anything_is_recorded() {
        assert_eq!(snooze_until(None, None, T0).unwrap(), T0 + 3600);
        assert_eq!(snooze_until(Some(15), None, T0).unwrap(), T0 + 900);
        assert_eq!(
            snooze_until(Some(5), Some(T0 + 7200), T0).unwrap(),
            T0 + 7200
        );
        assert!(snooze_until(Some(0), None, T0).is_err());
        assert!(snooze_until(None, Some(T0 - 1), T0).is_err());
        assert!(snooze_until(None, Some(T0 + 91 * 86_400), T0).is_err());
    }
}
