//! The edge worker's CI waits (`docs/plans/edge-worker-ci-loop.md`). A wait is a commit the
//! worker pushed and then stopped working on: the daemon reads its checks on each tick instead of
//! the worker holding a chat turn open, hands the task over when CI passes, and resumes the
//! worker's own thread when it fails.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

pub const STATE_WAITING: &str = "waiting";
/// CI passed and the task was handed over.
pub const STATE_PASSED: &str = "passed";
/// CI failed and the worker was asked to fix it. Counts as a round.
pub const STATE_FAILED: &str = "failed";
/// The task was parked: too many rounds, or CI never finished.
pub const STATE_PARKED: &str = "parked";
/// The worker pushed again before CI finished, and a newer wait took over.
pub const STATE_REPLACED: &str = "replaced";
/// Someone else moved the task on, so there is nothing left to wait for.
pub const STATE_CLOSED: &str = "closed";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CiWait {
    pub id: i64,
    pub task_id: String,
    pub repo: String,
    pub branch: String,
    pub sha: String,
    pub pull_request: Option<String>,
    pub thread_id: String,
    /// 1 for the first push, one more for each round CI failed before it.
    pub round: i64,
    pub state: String,
    pub detail: Option<String>,
    pub created_at: String,
    /// Seconds since the wait was recorded.
    pub age_secs: i64,
    /// Seconds since the checks were last read successfully (since it was recorded, if never).
    pub unread_secs: i64,
}

const COLUMNS: &str = "id, task_id, repo, branch, sha, pull_request, thread_id, round, state, \
     detail, created_at, \
     CAST((julianday('now') - julianday(created_at)) * 86400 AS INTEGER), \
     CAST((julianday('now') - julianday(COALESCE(last_read_at, created_at))) * 86400 AS INTEGER)";

fn from_row(row: &Row<'_>) -> rusqlite::Result<CiWait> {
    Ok(CiWait {
        id: row.get(0)?,
        task_id: row.get(1)?,
        repo: row.get(2)?,
        branch: row.get(3)?,
        sha: row.get(4)?,
        pull_request: row.get(5)?,
        thread_id: row.get(6)?,
        round: row.get(7)?,
        state: row.get(8)?,
        detail: row.get(9)?,
        created_at: row.get(10)?,
        age_secs: row.get(11)?,
        unread_secs: row.get(12)?,
    })
}

pub struct NewWait<'a> {
    pub task_id: &'a str,
    pub repo: &'a str,
    pub branch: &'a str,
    pub sha: &'a str,
    pub pull_request: Option<&'a str>,
    pub thread_id: &'a str,
}

/// Records a wait for `task_id`, replacing any it already has open (the worker pushed again
/// before CI finished). The round is one more than the rounds CI has failed for the task.
pub fn open(conn: &Connection, new: &NewWait<'_>) -> Result<CiWait> {
    crate::tasks::in_write_tx(conn, |conn| {
        conn.execute(
            "UPDATE edge_ci_waits SET state = ?2, resolved_at = datetime('now') \
             WHERE task_id = ?1 AND state = ?3",
            params![new.task_id, STATE_REPLACED, STATE_WAITING],
        )?;
        let failed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edge_ci_waits WHERE task_id = ?1 AND state = ?2",
            params![new.task_id, STATE_FAILED],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO edge_ci_waits (task_id, repo, branch, sha, pull_request, thread_id, round) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new.task_id,
                new.repo,
                new.branch,
                new.sha,
                new.pull_request,
                new.thread_id,
                failed + 1
            ],
        )?;
        let id = conn.last_insert_rowid();
        get(conn, id)?.ok_or_else(|| anyhow::anyhow!("the wait vanished as it was recorded"))
    })
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<CiWait>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM edge_ci_waits WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()
    .map_err(Into::into)
}

/// Every wait still open, oldest first.
pub fn open_waits(conn: &Connection) -> Result<Vec<CiWait>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM edge_ci_waits WHERE state = ?1 ORDER BY created_at, id"
    ))?;
    let rows = stmt
        .query_map([STATE_WAITING], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The open wait for a task, if it has one.
pub fn open_for_task(conn: &Connection, task_id: &str) -> Result<Option<CiWait>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM edge_ci_waits WHERE task_id = ?1 AND state = ?2"),
        params![task_id, STATE_WAITING],
        from_row,
    )
    .optional()
    .map_err(Into::into)
}

/// Every wait for a task, oldest first.
pub fn for_task(conn: &Connection, task_id: &str) -> Result<Vec<CiWait>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM edge_ci_waits WHERE task_id = ?1 ORDER BY created_at, id"
    ))?;
    let rows = stmt
        .query_map([task_id], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Notes that the checks were looked at; `read` when GitHub answered.
pub fn touch(conn: &Connection, id: i64, read: bool) -> Result<()> {
    let sql = if read {
        "UPDATE edge_ci_waits SET checked_at = datetime('now'), last_read_at = datetime('now') \
         WHERE id = ?1"
    } else {
        "UPDATE edge_ci_waits SET checked_at = datetime('now') WHERE id = ?1"
    };
    conn.execute(sql, [id])?;
    Ok(())
}

/// Ends an open wait. False when it was no longer open (another tick or a newer push got there).
pub fn resolve(conn: &Connection, id: i64, state: &str, detail: &str) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE edge_ci_waits SET state = ?2, detail = ?3, resolved_at = datetime('now') \
         WHERE id = ?1 AND state = ?4",
        params![id, state, detail, STATE_WAITING],
    )?;
    Ok(changed == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn wait<'a>(task: &'a str, sha: &'a str) -> NewWait<'a> {
        NewWait {
            task_id: task,
            repo: "o/r",
            branch: "hq/w-1",
            sha,
            pull_request: Some("https://github.com/o/r/pull/7"),
            thread_id: "thread-1",
        }
    }

    #[test]
    fn a_newer_push_replaces_the_open_wait_and_only_failed_rounds_count() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let first = open(c, &wait("t1", "aaa"))?;
            assert_eq!(first.round, 1);
            assert_eq!(first.state, STATE_WAITING);
            // Pushed again before CI finished: still the first round.
            let second = open(c, &wait("t1", "bbb"))?;
            assert_eq!(second.round, 1);
            assert_eq!(get(c, first.id)?.unwrap().state, STATE_REPLACED);
            assert_eq!(open_for_task(c, "t1")?.unwrap().sha, "bbb");
            // CI failed on it: the next push is round two.
            assert!(resolve(c, second.id, STATE_FAILED, "tests")?);
            assert!(!resolve(c, second.id, STATE_PASSED, "late")?);
            let third = open(c, &wait("t1", "ccc"))?;
            assert_eq!(third.round, 2);
            assert_eq!(open_waits(c)?.len(), 1);
            assert_eq!(for_task(c, "t1")?.len(), 3);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn ages_count_from_the_last_successful_read() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let w = open(c, &wait("t1", "aaa"))?;
            c.execute(
                "UPDATE edge_ci_waits SET created_at = datetime('now', '-2 hours') WHERE id = ?1",
                [w.id],
            )?;
            touch(c, w.id, false)?;
            let w = get(c, w.id)?.unwrap();
            assert!(w.age_secs >= 7190 && w.unread_secs >= 7190);
            touch(c, w.id, true)?;
            let w = get(c, w.id)?.unwrap();
            assert!(w.age_secs >= 7190 && w.unread_secs < 60);
            Ok(())
        })
        .unwrap();
    }
}
