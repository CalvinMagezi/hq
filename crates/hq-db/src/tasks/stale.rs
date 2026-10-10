//! Tasks that look abandoned, and the progress of an initiative as a whole. Nothing
//! here changes a task: a stale task is surfaced for a person or an agent to decide.

use super::*;

/// Hours with no sign of life after which an in-progress task counts as stale.
pub const DEFAULT_STALE_HOURS: i64 = 72;

#[derive(Debug, Clone, Serialize)]
pub struct StaleTask {
    pub task_id: String,
    pub display_id: String,
    pub title: String,
    pub initiative_id: String,
    pub last_activity_at: String,
    pub idle_hours: i64,
}

/// In-progress tasks nobody holds and nothing has touched for `stale_after_hours`: no
/// live lease, and no write, comment or lease heartbeat since. Oldest first.
pub fn stale_tasks(conn: &Connection, stale_after_hours: i64, ttl_secs: i64, limit: usize) -> Result<Vec<StaleTask>> {
    expire_stale_leases(conn, ttl_secs)?;
    stale_rows(conn, stale_after_hours, limit, None)
}

/// The rule itself. `only` narrows it to one task, so asking about one task does not scan them all.
/// Leases are assumed already expired by the caller.
fn stale_rows(conn: &Connection, stale_after_hours: i64, limit: usize, only: Option<&str>) -> Result<Vec<StaleTask>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, display_id, title, initiative_id, last_activity, \
                CAST((strftime('%s','now') - strftime('%s', last_activity)) / 3600 AS INTEGER) \
         FROM ( \
             SELECT t.id, t.display_id, t.title, t.initiative_id, \
                    MAX(t.updated_at, \
                        COALESCE((SELECT MAX(created_at) FROM task_comments \
                                  WHERE task_id = t.id AND author NOT LIKE 'watch-%'), ''), \
                        COALESCE((SELECT MAX(COALESCE(ended_at, last_heartbeat_at)) \
                                  FROM task_work_sessions WHERE task_id = t.id), '')) AS last_activity \
             FROM tasks t \
             WHERE t.status = 'in_progress' AND t.archived_at IS NULL AND (?2 IS NULL OR t.id = ?2) \
               AND NOT EXISTS (SELECT 1 FROM task_work_sessions s WHERE s.task_id = t.id AND s.ended_at IS NULL) \
         ) WHERE last_activity < datetime('now', ?1) ORDER BY last_activity LIMIT {limit}"
    ))?;
    let rows = stmt
        .query_map(params![format!("-{stale_after_hours} hours"), only], |r| {
            Ok(StaleTask {
                task_id: r.get(0)?,
                display_id: r.get(1)?,
                title: r.get(2)?,
                initiative_id: r.get(3)?,
                last_activity_at: r.get(4)?,
                idle_hours: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Whether one task is stale by the same rule.
pub fn is_stale(conn: &Connection, task_id: &str, stale_after_hours: i64, ttl_secs: i64) -> Result<bool> {
    expire_stale_leases(conn, ttl_secs)?;
    is_stale_now(conn, task_id, stale_after_hours)
}

/// `is_stale` for a caller that has just expired silent leases itself, as `task_get` does.
pub fn is_stale_now(conn: &Connection, task_id: &str, stale_after_hours: i64) -> Result<bool> {
    Ok(!stale_rows(conn, stale_after_hours, 1, Some(task_id))?.is_empty())
}

/// How an initiative, an epic in this system, is going.
#[derive(Debug, Clone, Serialize)]
pub struct InitiativeRollup {
    pub initiative_id: String,
    pub name: String,
    pub total: i64,
    pub to_do: i64,
    pub in_progress: i64,
    pub blocked: i64,
    pub ready_for_review: i64,
    pub complete: i64,
    /// Complete over total, 0.0 for an empty initiative.
    pub fraction_complete: f64,
    pub estimate_minutes: i64,
    pub tasks_with_estimate: i64,
    pub leased_seconds: i64,
    pub stale: i64,
}

/// Counts, estimates and worked time across every active task of an initiative.
pub fn initiative_rollup(conn: &Connection, initiative_ref: &str, stale_after_hours: i64, ttl_secs: i64) -> Result<InitiativeRollup> {
    let (id, name): (String, String) = conn
        .query_row(
            "SELECT id, name FROM initiatives WHERE id = ?1 OR slug = ?1 OR id_prefix = ?1 OR name = ?1",
            params![initiative_ref],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("no initiative '{initiative_ref}'"))?;
    let count = |status: &str| -> Result<i64> {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE initiative_id = ?1 AND status = ?2 AND archived_at IS NULL",
            params![id, status],
            |r| r.get(0),
        )?)
    };
    let (to_do, in_progress, blocked, ready_for_review, complete) = (
        count(STATUS_TO_DO)?,
        count(STATUS_IN_PROGRESS)?,
        count(STATUS_BLOCKED)?,
        count(STATUS_READY_FOR_REVIEW)?,
        count(STATUS_COMPLETE)?,
    );
    let total = to_do + in_progress + blocked + ready_for_review + complete;
    let (estimate_minutes, tasks_with_estimate): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(estimate_minutes), 0), COUNT(estimate_minutes) FROM tasks \
         WHERE initiative_id = ?1 AND archived_at IS NULL AND parent_task_id IS NULL",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut leased_seconds = 0;
    let mut stmt = conn.prepare("SELECT id FROM tasks WHERE initiative_id = ?1 AND archived_at IS NULL")?;
    for task in stmt.query_map(params![id], |r| r.get::<_, String>(0))? {
        leased_seconds += leased_seconds_of(conn, &task?)?;
    }
    let stale = stale_tasks(conn, stale_after_hours, ttl_secs, usize::MAX >> 1)?
        .iter()
        .filter(|s| s.initiative_id == id)
        .count() as i64;
    Ok(InitiativeRollup {
        initiative_id: id,
        name,
        total,
        to_do,
        in_progress,
        blocked,
        ready_for_review,
        complete,
        fraction_complete: if total == 0 { 0.0 } else { (complete as f64 / total as f64 * 1000.0).round() / 1000.0 },
        estimate_minutes,
        tasks_with_estimate,
        leased_seconds,
        stale,
    })
}

fn leased_seconds_of(conn: &Connection, task_id: &str) -> Result<i64> {
    leased_seconds(conn, task_id)
}
