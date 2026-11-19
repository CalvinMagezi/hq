//! `daily-brief`: one inbox card a day with the next step of each focus lane and what waits on
//! the owner. Rules and queries only, no model, and it reads no message text. Silent when there
//! is nothing to say. Device-only: shown in the inbox, never pushed. Off with
//! `daemon.daily_brief: false`.

use anyhow::Result;
use chrono::Utc;
use hq_core::config::HqConfig;
use hq_core::types::{DAILY_BRIEF_SOURCE, ValueItem, ValueKind};
use hq_db::Database;
use hq_db::tasks::{TaskFilter, list_tasks};
use hq_tools::tasks::next_step::{LaneStatus, LaneTask, StepKind, next_step_for_lane};
use tracing::info;

/// Tasks read per lane.
const LANE_SCAN: usize = 200;
/// Days without a touch after which an in-progress task counts as stale.
const STALE_DAYS: i64 = 5;
/// The assignee name for the person using HQ.
const OWNER: &str = "owner";

fn status_of(s: &str) -> LaneStatus {
    match s {
        "in_progress" => LaneStatus::InProgress,
        "blocked" => LaneStatus::Blocked,
        "ready_for_review" => LaneStatus::ReadyForReview,
        "complete" => LaneStatus::Complete,
        _ => LaneStatus::ToDo,
    }
}

fn priority_rank(p: Option<&str>) -> u8 {
    match p.map(str::to_ascii_lowercase).as_deref() {
        Some("urgent") => 0,
        Some("high") => 1,
        Some("low") => 3,
        _ => 2,
    }
}

fn days_since(ts: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(ts.get(..19).unwrap_or(ts), "%Y-%m-%d %H:%M:%S")
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(ts.get(..19).unwrap_or(ts), "%Y-%m-%dT%H:%M:%S")
        })
        .map_or(0, |t| (Utc::now().naive_utc() - t).num_days())
}

fn lane_tasks(db: &Database, lane: &str) -> Result<Vec<LaneTask>> {
    let filter = TaskFilter {
        tag: Some(lane.to_string()),
        limit: Some(LANE_SCAN),
        ..TaskFilter::default()
    };
    db.with_conn(|c| {
        let tasks = list_tasks(c, &filter)?;
        let mut out = Vec::with_capacity(tasks.len());
        for t in tasks {
            let leased: bool = c
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM task_work_sessions WHERE task_id = ?1 AND ended_at IS NULL)",
                    [&t.id],
                    |r| r.get(0),
                )?;
            let checkpoint: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM task_checkpoints WHERE task_id = ?1)",
                [&t.id],
                |r| r.get(0),
            )?;
            let idle = days_since(&t.updated_at);
            let mine = t.assignees.iter().any(|a| a.eq_ignore_ascii_case(OWNER))
                || t.waiting_on.as_deref().is_some_and(|w| w.eq_ignore_ascii_case(OWNER));
            let status = status_of(&t.status);
            out.push(LaneTask {
                id: t.display_id.clone(),
                title: t.title.clone(),
                status,
                priority: priority_rank(t.priority.as_deref()),
                due: t.due_date.clone(),
                age_days: days_since(&t.created_at),
                owner_must_decide: mine && matches!(status, LaneStatus::Blocked | LaneStatus::ReadyForReview),
                dependencies_done: t.blocked_by.is_empty(),
                has_lease: leased,
                has_checkpoint: checkpoint,
                stale: status == LaneStatus::InProgress && !leased && idle >= STALE_DAYS,
            });
        }
        Ok(out)
    })
}

fn line_for(lane: &str, tasks: &[LaneTask]) -> Option<String> {
    let step = next_step_for_lane(tasks)?;
    let what = match step.kind {
        StepKind::OwnerDecision => "waiting for your decision",
        StepKind::Ready => "next up",
        StepKind::Resume => "pick back up",
        StepKind::Hygiene => "looks abandoned",
    };
    Some(format!("{lane}: {what}, {} ({})", step.title, step.task_id))
}

/// The brief for `day`, or `None` when there is nothing to say.
pub fn brief_item(lines: &[String], day: &str) -> Option<ValueItem> {
    if lines.is_empty() {
        return None;
    }
    Some(
        ValueItem::new(
            DAILY_BRIEF_SOURCE,
            ValueKind::Fyi,
            "Your day".to_string(),
            lines.join("\n"),
        )
        .with_dedup_key(format!("daily-brief-{day}")),
    )
}

pub async fn run_daily_brief(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.daily_brief {
        return Ok(());
    }
    let mut lines = Vec::new();
    for lane in hq_db::focus_lanes::list(db)?
        .into_iter()
        .filter(|l| !l.parked)
    {
        if let Some(line) = line_for(&lane.name, &lane_tasks(db, &lane.name)?) {
            lines.push(line);
        }
    }
    if let Some(item) = brief_item(&lines, &Utc::now().format("%Y-%m-%d").to_string()) {
        hq_db::value_items::emit(db, &item)?;
        info!(lanes = lines.len(), "daily-brief: wrote today's brief");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_lanes_and_nothing_to_say_is_silent() {
        assert!(brief_item(&[], "2030-01-01").is_none());
    }

    #[test]
    fn the_brief_lists_one_line_per_lane_and_is_once_a_day() {
        let lines = vec!["Alpha: next up, Fix it (T-1)".to_string()];
        let item = brief_item(&lines, "2030-01-01").unwrap();
        assert_eq!(item.dedup_key.as_deref(), Some("daily-brief-2030-01-01"));
        assert!(item.is_device_only());
        assert!(item.body.contains("Alpha"));
    }

    #[tokio::test]
    async fn a_run_with_no_lanes_writes_nothing() {
        let db = Database::open_memory().unwrap();
        let mut config = HqConfig::default();
        config.daemon.daily_brief = true;
        run_daily_brief(&db, &config).await.unwrap();
        let n: i64 = db
            .with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM value_items", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(n, 0);
    }
}
