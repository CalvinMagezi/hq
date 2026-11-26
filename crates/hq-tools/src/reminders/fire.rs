//! One pass that rings every reminder that is due: an inbox item per ping, the previous ping
//! retired, the row advanced. The daemon task `reminders` calls it every half minute.
//!
//! Safe to repeat: each ping has its own dedup key (`reminder-<id>-<n>`), so a crash between the
//! item and the row update rings once, not twice.

use anyhow::Result;
use chrono::TimeZone;
use chrono_tz::Tz;
use hq_core::config::RemindersConfig;
use hq_core::types::{REMINDER_SOURCE, ValueItem, ValueKind};
use hq_db::Database;
use hq_db::reminders::{self, Fired, OUTCOME_SUPERSEDED, Reminder};
use rusqlite::params;
use tracing::warn;

use super::schedule::{NUDGE_PINGS, Plan, plan_fire, zone_of};

/// Most reminders rung in one pass; the rest follow on the next.
const MAX_PER_PASS: usize = 50;
const NOTE_CHARS: usize = 300;

/// What one pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub rang: usize,
    /// Stopped because the task they followed finished.
    pub stopped: usize,
}

/// A time in the owner's words: `4:00 pm today`, `9:00 am tomorrow`, `Thu 26 Nov, 4:00 pm`.
pub fn when_words(epoch: i64, now: i64, tz: Tz) -> String {
    let Some(t) = tz.timestamp_opt(epoch, 0).single() else {
        return "at an unknown time".to_string();
    };
    let clock = t.format("%-I:%M %p").to_string().to_lowercase();
    let today = tz
        .timestamp_opt(now, 0)
        .single()
        .map_or(t.date_naive(), |n| n.date_naive());
    match (t.date_naive() - today).num_days() {
        0 => format!("{clock} today"),
        1 => format!("{clock} tomorrow"),
        -1 => format!("{clock} yesterday"),
        _ => format!("{}, {clock}", t.format("%a %-d %b")),
    }
}

/// The inbox item for one ping.
pub fn build_item(r: &Reminder, plan: &Plan, now: i64) -> ValueItem {
    let tz = zone_of(r);
    let mut lines: Vec<String> = Vec::new();
    if r.nudge_step == 0 {
        let when = when_words(r.due_at, now, tz);
        lines.push(if plan.late {
            format!("Was due {when}.")
        } else {
            format!("Due {when}.")
        });
    } else if plan.last_ping {
        lines.push(format!(
            "Last reminder ({} of {NUDGE_PINGS}). Mark it done, snooze it, or drop it.",
            plan.ping_no
        ));
    } else {
        lines.push(format!(
            "Still open. Reminder {} of {NUDGE_PINGS}.",
            plan.ping_no
        ));
    }
    if let Some(note) = r.note.as_deref() {
        lines.push(note.chars().take(NOTE_CHARS).collect());
    }
    if r.tainted {
        lines.push("Set while HQ was reading outside content.".to_string());
    }
    let mut item = ValueItem::new(
        REMINDER_SOURCE,
        ValueKind::Fyi,
        r.title.clone(),
        lines.join("\n"),
    )
    .with_dedup_key(format!("reminder-{}-{}", r.id, r.fire_seq + 1));
    // A reminder made after reading outside text carries no link, so it cannot send the owner anywhere.
    if !r.tainted {
        item = item.with_link(format!("/reminders?id={}", r.id));
    }
    item
}

/// Why a reminder that follows a task should stop on its own, if it should.
fn task_finished(db: &Database, task_id: &str) -> Result<Option<&'static str>> {
    db.with_conn(|c| {
        Ok(match hq_db::tasks::get_task(c, task_id)? {
            None => Some("task gone"),
            Some(t) if t.archived_at.is_some() => Some("task archived"),
            Some(t) if t.status == hq_db::tasks::STATUS_COMPLETE => Some("task complete"),
            Some(_) => None,
        })
    })
}

/// The id the inbox holds for a ping's dedup key. It differs from the item built here when an
/// earlier pass already wrote that ping and crashed before the row was updated.
fn stored_item_id(db: &Database, dedup_key: &str, fallback: &str) -> String {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT id FROM value_items WHERE source_task = ?1 AND dedup_key = ?2 \
             ORDER BY created_at DESC LIMIT 1",
            params![REMINDER_SOURCE, dedup_key],
            |r| r.get::<_, String>(0),
        )
        .ok())
    })
    .ok()
    .flatten()
    .unwrap_or_else(|| fallback.to_string())
}

fn ring(db: &Database, r: &Reminder, cfg: &RemindersConfig, now: i64) -> Result<()> {
    let plan = plan_fire(r, now, cfg);
    let item = build_item(r, &plan, now);
    let key = item.dedup_key.clone().unwrap_or_default();
    let fallback_id = item.id.clone();
    // The previous ping no longer needs an answer: this one replaces it.
    if let Some(previous) = r.last_item_id.as_deref() {
        hq_db::value_items::retire(db, previous)?;
    }
    hq_db::value_items::emit(db, &item)?;
    let item_id = stored_item_id(db, &key, &fallback_id);
    reminders::record_fire(
        db,
        &r.id,
        now,
        &Fired {
            item_id: &item_id,
            status: plan.status,
            next_due_at: plan.next_due_at,
            occurrence_at: plan.occurrence_at,
            nudge_step: plan.nudge_step,
        },
    )
}

/// Rings every reminder due at `now`.
pub fn fire_due(db: &Database, cfg: &RemindersConfig, now: i64) -> Result<Report> {
    let mut report = Report::default();
    for r in reminders::due(db, now, MAX_PER_PASS)? {
        if let Some(task_id) = r.task_id.as_deref() {
            match task_finished(db, task_id) {
                Ok(Some(_)) => {
                    if let Some(previous) = r.last_item_id.as_deref() {
                        let _ = hq_db::value_items::retire(db, previous);
                    }
                    if let Err(e) = reminders::cancel(db, &r.id, now, OUTCOME_SUPERSEDED) {
                        warn!(reminder = %r.id, "reminders: stopping a finished task's reminder failed: {e}");
                    }
                    report.stopped += 1;
                    continue;
                }
                Ok(None) => {}
                Err(e) => warn!(reminder = %r.id, "reminders: task check failed: {e}"),
            }
        }
        match ring(db, &r, cfg, now) {
            Ok(()) => report.rang += 1,
            Err(e) => warn!(reminder = %r.id, "reminders: ringing failed, retrying next pass: {e}"),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reminders::schedule::Repeat;
    use chrono_tz::Africa::Kampala;
    use hq_db::reminders::{Limits, NewReminder, STATUS_ACTIVE, STATUS_DONE, STATUS_LAPSED};

    const LIMITS: Limits = Limits {
        max_active: 200,
        max_per_hour: 100,
    };

    fn t(d: u32, h: u32, m: u32) -> i64 {
        Kampala
            .with_ymd_and_hms(2026, 11, d, h, m, 0)
            .unwrap()
            .timestamp()
    }

    fn make(db: &Database, due: i64, nudge: bool, repeat: Option<&Repeat>) -> Reminder {
        let rule = repeat.map(Repeat::to_json);
        reminders::create(
            db,
            &NewReminder {
                title: "Call Moses about the 50,000",
                note: None,
                due_at: due,
                tz: "Africa/Kampala",
                repeat_rule: rule.as_deref(),
                nudge,
                task_id: None,
                thread_id: None,
                tainted: false,
                source: "chat",
            },
            due - 3600,
            LIMITS,
        )
        .unwrap()
    }

    fn pending_titles(db: &Database) -> Vec<(String, String)> {
        hq_db::value_items::list_by_state(db, hq_core::types::ValueState::Pending)
            .unwrap()
            .into_iter()
            .map(|i| (i.title, i.body))
            .collect()
    }

    #[test]
    fn nothing_rings_before_its_time_and_one_item_rings_at_it() {
        let db = Database::open_memory().unwrap();
        let due = t(26, 16, 0);
        let r = make(&db, due, false, None);
        let cfg = RemindersConfig::default();
        assert_eq!(fire_due(&db, &cfg, due - 1).unwrap(), Report::default());
        let report = fire_due(&db, &cfg, due + 10).unwrap();
        assert_eq!(report.rang, 1);
        let items = pending_titles(&db);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, "Call Moses about the 50,000");
        assert_eq!(items[0].1, "Due 4:00 pm today.");
        assert_eq!(
            reminders::get(&db, &r.id).unwrap().unwrap().status,
            STATUS_DONE
        );
        // Ringing again changes nothing: the one-shot is finished.
        assert_eq!(fire_due(&db, &cfg, due + 40).unwrap().rang, 0);
    }

    #[test]
    fn a_ping_that_comes_late_says_so() {
        let db = Database::open_memory().unwrap();
        let due = t(26, 16, 0);
        make(&db, due, false, None);
        fire_due(&db, &RemindersConfig::default(), due + 2 * 3600).unwrap();
        assert_eq!(pending_titles(&db)[0].1, "Was due 4:00 pm today.");
    }

    #[test]
    fn a_nudge_replaces_its_last_ping_until_the_ladder_ends() {
        let db = Database::open_memory().unwrap();
        let due = t(26, 10, 0);
        let r = make(&db, due, true, None);
        let cfg = RemindersConfig::default();
        let mut now = due;
        for ping in 1..=NUDGE_PINGS {
            fire_due(&db, &cfg, now).unwrap();
            let items = pending_titles(&db);
            assert_eq!(
                items.len(),
                1,
                "only the newest ping is waiting (ping {ping})"
            );
            let after = reminders::get(&db, &r.id).unwrap().unwrap();
            if after.status != STATUS_ACTIVE {
                assert_eq!(ping, NUDGE_PINGS);
                assert_eq!(after.status, STATUS_LAPSED);
                assert!(
                    items[0].1.starts_with("Last reminder (5 of 5)"),
                    "{}",
                    items[0].1
                );
                break;
            }
            now = after.due_at;
        }
        assert_eq!(
            reminders::get(&db, &r.id).unwrap().unwrap().fire_seq,
            NUDGE_PINGS
        );
    }

    #[test]
    fn a_repeat_rings_each_occurrence_with_its_own_item() {
        let db = Database::open_memory().unwrap();
        let due = t(26, 9, 0);
        let r = make(&db, due, false, Some(&Repeat::Daily { every: 1 }));
        let cfg = RemindersConfig::default();
        fire_due(&db, &cfg, due).unwrap();
        let after = reminders::get(&db, &r.id).unwrap().unwrap();
        assert_eq!(after.due_at, t(27, 9, 0));
        fire_due(&db, &cfg, after.due_at).unwrap();
        let keys: Vec<String> =
            hq_db::value_items::list_by_state(&db, hq_core::types::ValueState::Pending)
                .unwrap()
                .into_iter()
                .filter_map(|i| i.dedup_key)
                .collect();
        assert_eq!(keys.len(), 1, "the first day's item was retired: {keys:?}");
        assert!(keys[0].ends_with("-2"));
    }

    #[test]
    fn a_reminder_that_follows_a_finished_task_stops_without_ringing() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-1", "personal", None, "Work", "work", "WK")?;
            hq_db::tasks::create_task(
                c,
                "tk-1",
                "in-1",
                &hq_db::tasks::NewTask {
                    title: "Collect payment",
                    created_by: "t",
                    ..Default::default()
                },
            )?;
            hq_db::tasks::update_task(
                c,
                "tk-1",
                &hq_db::tasks::TaskPatch {
                    status: Some("complete".into()),
                    ..Default::default()
                },
                None,
            )?;
            Ok(())
        })
        .unwrap();
        let due = t(26, 16, 0);
        let r = reminders::create(
            &db,
            &NewReminder {
                title: "Collect",
                note: None,
                due_at: due,
                tz: "Africa/Kampala",
                repeat_rule: None,
                nudge: true,
                task_id: Some("tk-1"),
                thread_id: None,
                tainted: false,
                source: "chat",
            },
            due - 60,
            LIMITS,
        )
        .unwrap();
        let report = fire_due(&db, &RemindersConfig::default(), due).unwrap();
        assert_eq!((report.rang, report.stopped), (0, 1));
        assert!(pending_titles(&db).is_empty());
        assert_eq!(
            reminders::get(&db, &r.id).unwrap().unwrap().status,
            hq_db::reminders::STATUS_CANCELLED
        );
    }

    #[test]
    fn a_tainted_reminder_says_so_and_carries_no_link() {
        let db = Database::open_memory().unwrap();
        let due = t(26, 16, 0);
        let mut r = make(&db, due, false, None);
        r.tainted = true;
        let plan = plan_fire(&r, due, &RemindersConfig::default());
        let item = build_item(&r, &plan, due);
        assert!(
            item.body
                .contains("Set while HQ was reading outside content.")
        );
        assert!(item.link.is_none());
        r.tainted = false;
        let item = build_item(&r, &plan, due);
        assert_eq!(
            item.link.as_deref(),
            Some(format!("/reminders?id={}", r.id).as_str())
        );
    }

    #[test]
    fn times_are_worded_for_today_tomorrow_and_later() {
        let now = t(26, 12, 0);
        assert_eq!(when_words(t(26, 16, 0), now, Kampala), "4:00 pm today");
        assert_eq!(when_words(t(27, 9, 0), now, Kampala), "9:00 am tomorrow");
        assert_eq!(when_words(t(25, 9, 0), now, Kampala), "9:00 am yesterday");
        assert_eq!(
            when_words(t(30, 9, 30), now, Kampala),
            "Mon 30 Nov, 9:30 am"
        );
    }
}
