//! `due-reminders`: one inbox item a day for each task the person made in their own chat that is
//! due today or tomorrow, or overdue by a week at most (`docs/plans/simple-hq.md`, phase 2, the
//! dated to-do offer). Tasks agents file through MCP have no chat origin and are never reminded.
//! The item is pushed like other inbox items (`due_reminder` is always pushed) and shown on the
//! phone. No model.

use anyhow::Result;
use chrono::{Duration, NaiveDate, Timelike};
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::tasks::{DueTask, due_from_chat};
use tracing::info;

pub use hq_core::types::DUE_REMINDER_SOURCE;
/// Local hours a reminder may arrive in, so nobody is woken by one.
const FIRST_HOUR: u32 = 8;
const LAST_HOUR: u32 = 21;
/// An overdue task is reminded for this many days, then left to the Tasks screen.
const OVERDUE_DAYS: i64 = 7;
/// Most reminders in one pass.
const MAX_REMINDERS: usize = 10;

fn when(due: NaiveDate, today: NaiveDate) -> String {
    match (due - today).num_days() {
        0 => "Due today".to_string(),
        1 => "Due tomorrow".to_string(),
        _ => format!("Overdue since {}", due.format("%a %-d %b")),
    }
}

/// The item for one task on `today`; the dedup key carries the day, so it comes once a day.
pub fn reminder(task: &DueTask, today: NaiveDate) -> ValueItem {
    let title = format!("{}: {}", when(task.due, today), task.title);
    let body = format!(
        "{} is due {}. Mark it done in Tasks when it is.",
        task.display_id,
        task.due.format("%A %-d %B")
    );
    ValueItem::new(DUE_REMINDER_SOURCE, ValueKind::Fyi, title, body)
        .with_dedup_key(format!("due-{}-{today}", task.id))
}

/// Whether a pass at this local hour may remind.
fn in_waking_hours(hour: u32) -> bool {
    (FIRST_HOUR..=LAST_HOUR).contains(&hour)
}

pub async fn run_due_reminders(db: &Database) -> Result<()> {
    let now = chrono::Local::now();
    if !in_waking_hours(now.hour()) {
        return Ok(());
    }
    let today = now.date_naive();
    let tasks = db.with_conn(|c| {
        due_from_chat(
            c,
            today - Duration::days(OVERDUE_DAYS),
            today + Duration::days(1),
            MAX_REMINDERS,
        )
    })?;
    for task in &tasks {
        hq_db::value_items::emit(db, &reminder(task, today))?;
    }
    if !tasks.is_empty() {
        info!(
            tasks = tasks.len(),
            "due-reminders: reminded about tasks due soon"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_reminder_says_when_in_words_and_comes_once_a_day() {
        let task = DueTask {
            id: "tk_1".into(),
            display_id: "INB-4".into(),
            title: "Collect 50,000 from the shop".into(),
            due: day("2026-11-07"),
        };
        let item = reminder(&task, day("2026-11-07"));
        assert_eq!(item.title, "Due today: Collect 50,000 from the shop");
        assert!(item.body.contains("Saturday 7 November"));
        assert_eq!(item.dedup_key.as_deref(), Some("due-tk_1-2026-11-07"));
        assert!(
            !item.is_device_only(),
            "a reminder is pushed like other items"
        );
        assert_eq!(
            reminder(&task, day("2026-11-06")).title,
            "Due tomorrow: Collect 50,000 from the shop"
        );
        assert!(
            reminder(&task, day("2026-11-09"))
                .title
                .starts_with("Overdue since Sat 7 Nov")
        );
        assert!(!item.title.contains('\u{2014}'));
    }

    #[test]
    fn reminders_wait_for_waking_hours() {
        assert!(!in_waking_hours(3));
        assert!(in_waking_hours(FIRST_HOUR));
        assert!(in_waking_hours(LAST_HOUR));
        assert!(!in_waking_hours(LAST_HOUR + 1));
    }
}
