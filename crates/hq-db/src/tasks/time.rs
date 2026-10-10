//! Time on a task, from three separate sources: leased active time (the union of
//! work lease intervals), wall-clock time in each status (from the event log) and
//! the estimate. Anything that cannot be known is `None`, never guessed: a task
//! that predates the full event log has no status durations.

use super::*;
use std::collections::BTreeMap;

/// Leases and events are stored as UTC `YYYY-MM-DD HH:MM:SS`; these are epoch
/// seconds from SQLite so the arithmetic stays in integers.
const EPOCH: &str = "CAST(strftime('%s', ";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TimeSummary {
    pub estimate_minutes: Option<i64>,
    /// Union of the task's lease intervals, so two sessions working at once are
    /// counted once. A live external lease counts up to its last heartbeat.
    pub leased_seconds: i64,
    pub lease_count: i64,
    /// A lease is open now.
    pub live: bool,
    /// Created to first in_progress. `None` when the task was never started or
    /// the start was not recorded.
    pub time_to_start_seconds: Option<i64>,
    /// First in_progress to completion. `None` unless the task is complete and both
    /// moments are known.
    pub cycle_seconds: Option<i64>,
    /// Seconds spent in each status so far. `None` when any event predates the
    /// full log, because the gaps cannot be filled honestly.
    pub status_seconds: Option<BTreeMap<String, i64>>,
    /// Leased minutes minus the estimate; positive means over. `None` without an estimate.
    pub variance_minutes: Option<i64>,
    /// Totals over the task's sub-tasks, present only when it has any.
    pub subtasks: Option<SubtaskRollup>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SubtaskRollup {
    pub count: i64,
    pub leased_seconds: i64,
    /// Sum of the sub-task estimates that exist.
    pub estimate_minutes: i64,
    pub with_estimate: i64,
}

/// Seconds covered by the intervals, with overlaps counted once.
pub fn union_seconds(intervals: &mut Vec<(i64, i64)>) -> i64 {
    intervals.retain(|(start, end)| end > start);
    intervals.sort_unstable();
    let mut total = 0;
    let mut current: Option<(i64, i64)> = None;
    for &(start, end) in intervals.iter() {
        current = match current {
            Some((cs, ce)) if start <= ce => Some((cs, ce.max(end))),
            Some((cs, ce)) => {
                total += ce - cs;
                Some((start, end))
            }
            None => Some((start, end)),
        };
    }
    total + current.map_or(0, |(cs, ce)| ce - cs)
}

/// One logged move: when, and between which statuses. Statuses are `None` for an
/// event recorded before the full log existed.
#[derive(Debug, Clone, PartialEq)]
pub struct Move {
    pub at: i64,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// Seconds in each status, from creation to `now`. `None` when any move lacks its
/// statuses, or when the moves do not end in the task's `status` now: a task that
/// moved before the log recorded its moves has gaps that cannot be filled honestly.
/// Time in `complete` is not counted: it is not time spent.
pub fn status_durations(created_at: i64, moves: &[Move], now: i64, status: &str) -> Option<BTreeMap<String, i64>> {
    let mut out: BTreeMap<String, i64> = BTreeMap::new();
    let mut add = |status: &str, seconds: i64| {
        if status != STATUS_COMPLETE && seconds > 0 {
            *out.entry(status.to_string()).or_insert(0) += seconds;
        }
    };
    let mut since = created_at;
    let mut current: Option<String> = None;
    for m in moves {
        let (from, to) = (m.from.as_deref()?, m.to.as_deref()?);
        if current.is_none() {
            current = Some(from.to_string());
        }
        // A clock that stepped back must not count a stretch twice.
        let at = m.at.max(since);
        add(current.as_deref().unwrap_or(from), at - since);
        since = at;
        current = Some(to.to_string());
    }
    // A task with no moves at all has been in its first status since it was made.
    let last = current.unwrap_or_else(|| STATUS_TO_DO.to_string());
    if last != status {
        return None;
    }
    add(&last, now - since);
    Some(out)
}

fn epoch(conn: &Connection, sql: &str, id: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(sql, params![id], |r| r.get::<_, Option<i64>>(0))
        .optional()?
        .flatten())
}

fn lease_intervals(conn: &Connection, task_id: &str) -> Result<(Vec<(i64, i64)>, bool)> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EPOCH}started_at) AS INTEGER), \
                {EPOCH}COALESCE(ended_at, CASE WHEN harness_session_id IS NULL \
                    THEN last_heartbeat_at ELSE datetime('now') END)) AS INTEGER), \
                ended_at IS NULL \
         FROM task_work_sessions WHERE task_id = ?1"
    ))?;
    let mut live = false;
    let mut out = Vec::new();
    for row in stmt.query_map(params![task_id], |r| {
        Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, bool>(2)?))
    })? {
        let (start, end, open) = row?;
        live |= open;
        // A timestamp SQLite cannot read leaves the interval out rather than failing the summary.
        if let (Some(start), Some(end)) = (start, end) {
            out.push((start, end));
        }
    }
    Ok((out, live))
}

/// The task's logged moves, or `None` when one has a timestamp that cannot be read.
fn moves(conn: &Connection, task_id: &str) -> Result<Option<Vec<Move>>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {EPOCH}occurred_at) AS INTEGER), from_status, to_status \
         FROM task_events WHERE task_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt
        .query_map(params![task_id], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<(Option<i64>, Option<String>, Option<String>)>>>()?;
    Ok(rows
        .into_iter()
        .map(|(at, from, to)| at.map(|at| Move { at, from, to }))
        .collect())
}

fn subtask_rollup(conn: &Connection, task: &Task) -> Result<Option<SubtaskRollup>> {
    let children = subtask_ids(conn, &task.id)?;
    if children.is_empty() {
        return Ok(None);
    }
    let mut rollup = SubtaskRollup { count: children.len() as i64, leased_seconds: 0, estimate_minutes: 0, with_estimate: 0 };
    for id in &children {
        let (mut intervals, _) = lease_intervals(conn, id)?;
        rollup.leased_seconds += union_seconds(&mut intervals);
        let estimate: Option<i64> = conn.query_row(
            "SELECT estimate_minutes FROM tasks WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        if let Some(minutes) = estimate {
            rollup.estimate_minutes += minutes;
            rollup.with_estimate += 1;
        }
    }
    Ok(Some(rollup))
}

/// Seconds of leased work on a task, overlapping sessions counted once.
pub fn leased_seconds(conn: &Connection, task_id: &str) -> Result<i64> {
    let (mut intervals, _) = lease_intervals(conn, task_id)?;
    Ok(union_seconds(&mut intervals))
}

/// Everything known about the time spent on `id_or_display_id`. Leases that went
/// silent past `ttl_secs` are closed first, so `live` and the totals never include
/// a session that is already gone.
pub fn time_summary(conn: &Connection, id_or_display_id: &str, ttl_secs: i64) -> Result<TimeSummary> {
    expire_stale_leases(conn, ttl_secs)?;
    let task = get_task(conn, id_or_display_id)?
        .ok_or_else(|| anyhow::anyhow!("no task '{id_or_display_id}'"))?;
    let created = epoch(conn, &format!("SELECT {EPOCH}created_at) AS INTEGER) FROM tasks WHERE id = ?1"), &task.id)?
        .ok_or_else(|| anyhow::anyhow!("task {} has no creation time", task.display_id))?;
    let now: i64 = conn.query_row(&format!("SELECT {EPOCH}'now') AS INTEGER)"), [], |r| r.get(0))?;
    let started = epoch(conn, &format!("SELECT {EPOCH}work_started_at) AS INTEGER) FROM tasks WHERE id = ?1"), &task.id)?;
    let completed = epoch(conn, &format!("SELECT {EPOCH}completed_at) AS INTEGER) FROM tasks WHERE id = ?1"), &task.id)?;

    let (mut intervals, live) = lease_intervals(conn, &task.id)?;
    let lease_count = intervals.len() as i64;
    let leased_seconds = union_seconds(&mut intervals);
    let history = moves(conn, &task.id)?;
    // With no recorded work there is nothing to compare, so no variance rather than "all of the estimate under".
    let variance_minutes = task.estimate_minutes.filter(|_| lease_count > 0).map(|e| leased_seconds / 60 - e);
    let cycle_seconds = match (started, completed) {
        (Some(s), Some(c)) if task.status == STATUS_COMPLETE && c >= s => Some(c - s),
        _ => None,
    };
    Ok(TimeSummary {
        estimate_minutes: task.estimate_minutes,
        leased_seconds,
        lease_count,
        live,
        time_to_start_seconds: started.map(|s| (s - created).max(0)),
        cycle_seconds,
        status_seconds: history.and_then(|moves| status_durations(created, &moves, now, &task.status)),
        variance_minutes,
        subtasks: subtask_rollup(conn, &task)?,
    })
}

/// One initiative's row in a time report.
#[derive(Debug, Clone, Serialize)]
pub struct InitiativeTime {
    pub initiative_id: String,
    pub name: String,
    pub leased_seconds: i64,
    pub tasks_worked: i64,
    pub tasks_completed: i64,
    /// Mean first-start-to-completion over completed tasks that recorded both.
    pub mean_cycle_seconds: Option<i64>,
    /// Completed tasks with an estimate and leased time: how many, and the mean of
    /// actual over estimate (1.0 is exact, above 1.0 is over).
    pub estimated_completed: i64,
    pub mean_actual_over_estimate: Option<f64>,
    /// Tasks in the initiative with no recorded start, counted but never guessed.
    pub unknown_tasks: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActorTime {
    pub actor: String,
    pub leased_seconds: i64,
    pub sessions: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeReport {
    pub since: String,
    pub initiatives: Vec<InitiativeTime>,
    pub actors: Vec<ActorTime>,
}

/// A task that counts toward a report, with what the report needs from it.
struct ReportTask {
    initiative_id: String,
    name: String,
    estimate: Option<i64>,
    cycle: Option<i64>,
    done: bool,
}

/// Time since `since_days` ago, by initiative and by actor. Leases are counted
/// from `started_at`; tasks from their lease and completion times.
pub fn time_report(conn: &Connection, since_days: i64, ttl_secs: i64) -> Result<TimeReport> {
    expire_stale_leases(conn, ttl_secs)?;
    let since: String = conn.query_row(
        "SELECT datetime('now', ?1)",
        params![format!("-{since_days} days")],
        |r| r.get(0),
    )?;
    let mut by_task: BTreeMap<String, ReportTask> = BTreeMap::new();
    {
        let mut stmt = conn.prepare(&format!(
            "SELECT t.id, i.id, i.name, t.estimate_minutes, \
                    {EPOCH}t.work_started_at) AS INTEGER), {EPOCH}t.completed_at) AS INTEGER), \
                    t.status = 'complete' \
             FROM tasks t JOIN initiatives i ON i.id = t.initiative_id \
             WHERE t.archived_at IS NULL AND t.id IN (SELECT task_id FROM task_work_sessions \
                            WHERE ended_at IS NULL OR ended_at >= ?1) \
                OR (t.completed_at IS NOT NULL AND t.completed_at >= ?1)"
        ))?;
        for row in stmt.query_map(params![since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, bool>(6)?,
            ))
        })? {
            let (task, init, name, estimate, started, completed, done) = row?;
            let cycle = match (started, completed) {
                (Some(s), Some(c)) if done && c >= s => Some(c - s),
                _ => None,
            };
            by_task.insert(task, ReportTask { initiative_id: init, name, estimate, cycle, done });
        }
    }
    let mut initiatives: BTreeMap<String, InitiativeTime> = BTreeMap::new();
    let mut ratios: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut cycles: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let since_epoch: i64 = conn.query_row(&format!("SELECT {EPOCH}?1) AS INTEGER)"), params![since], |r| r.get(0))?;
    for (task, row) in &by_task {
        let ReportTask { initiative_id: init, name, estimate, cycle, done } = row;
        // Only the part of each lease inside the window counts, so an old lease on a
        // task worked again this week adds nothing from last year.
        let (mut intervals, _) = lease_intervals(conn, task)?;
        let mut intervals: Vec<(i64, i64)> = intervals.drain(..).map(|(s, e)| (s.max(since_epoch), e)).collect();
        let leased = union_seconds(&mut intervals);
        let entry = initiatives.entry(init.clone()).or_insert_with(|| InitiativeTime {
            initiative_id: init.clone(),
            name: name.clone(),
            leased_seconds: 0,
            tasks_worked: 0,
            tasks_completed: 0,
            mean_cycle_seconds: None,
            estimated_completed: 0,
            mean_actual_over_estimate: None,
            unknown_tasks: 0,
        });
        entry.leased_seconds += leased;
        if leased > 0 {
            entry.tasks_worked += 1;
        }
        if *done {
            entry.tasks_completed += 1;
            match cycle {
                Some(c) => cycles.entry(init.clone()).or_default().push(*c),
                None => entry.unknown_tasks += 1,
            }
            if let (Some(est), true) = (estimate, leased > 0) {
                entry.estimated_completed += 1;
                ratios.entry(init.clone()).or_default().push(leased as f64 / (*est as f64 * 60.0));
            }
        }
    }
    for (init, entry) in initiatives.iter_mut() {
        entry.mean_cycle_seconds = cycles.get(init).filter(|v| !v.is_empty()).map(|v| v.iter().sum::<i64>() / v.len() as i64);
        entry.mean_actual_over_estimate = ratios
            .get(init)
            .filter(|v| !v.is_empty())
            .map(|v| (v.iter().sum::<f64>() / v.len() as f64 * 100.0).round() / 100.0);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT actor, {EPOCH}started_at) AS INTEGER), \
                {EPOCH}COALESCE(ended_at, CASE WHEN harness_session_id IS NULL \
                    THEN last_heartbeat_at ELSE datetime('now') END)) AS INTEGER) \
         FROM task_work_sessions WHERE ended_at IS NULL OR ended_at >= ?1"
    ))?;
    // Per actor the union of everything inside the window, so two sessions of one
    // agent working at once are one agent's time, not two.
    let mut per_actor: BTreeMap<String, (Vec<(i64, i64)>, i64)> = BTreeMap::new();
    for row in stmt.query_map(params![since], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?))
    })? {
        let (actor, start, end) = row?;
        if let (Some(start), Some(end)) = (start, end) {
            let entry = per_actor.entry(actor).or_default();
            entry.0.push((start.max(since_epoch), end));
            entry.1 += 1;
        }
    }
    let mut actors: Vec<ActorTime> = per_actor
        .into_iter()
        .map(|(actor, (mut intervals, sessions))| ActorTime {
            actor,
            leased_seconds: union_seconds(&mut intervals),
            sessions,
        })
        .collect();
    actors.sort_by(|a, b| b.leased_seconds.cmp(&a.leased_seconds).then(a.actor.cmp(&b.actor)));
    let mut initiatives: Vec<InitiativeTime> = initiatives.into_values().collect();
    initiatives.sort_by(|a, b| b.leased_seconds.cmp(&a.leased_seconds).then(a.name.cmp(&b.name)));
    Ok(TimeReport { since, initiatives, actors })
}
