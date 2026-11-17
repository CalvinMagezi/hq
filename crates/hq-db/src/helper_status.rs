//! What HQ knows about the coding helpers on each computer, and the runs that set them up
//! (`sql/138_helper_setup.sql`). Readings with a time, never promises.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HelperStatus {
    pub host: String,
    pub helper: String,
    pub installed: bool,
    /// `None`: HQ has no way to ask this program whether it is signed in.
    pub signed_in: Option<bool>,
    pub checked_at: i64,
    pub last_error: Option<String>,
}

/// Records a reading. A reading that cannot tell (`signed_in: None`) keeps the earlier answer
/// while the program stays installed, so a failed check never erases a known sign-in.
pub fn record(
    db: &Database,
    host: &str,
    helper: &str,
    installed: bool,
    signed_in: Option<bool>,
    last_error: Option<&str>,
) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO helper_status (host, helper, installed, signed_in, checked_at, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(host, helper) DO UPDATE SET
               installed = excluded.installed,
               signed_in = CASE WHEN excluded.installed = 0 THEN NULL
                                WHEN excluded.signed_in IS NULL THEN helper_status.signed_in
                                ELSE excluded.signed_in END,
               checked_at = excluded.checked_at,
               last_error = excluded.last_error",
            params![
                host,
                helper,
                installed,
                signed_in,
                Utc::now().timestamp(),
                last_error
            ],
        )?;
        Ok(())
    })
}

pub fn list(db: &Database) -> Result<Vec<HelperStatus>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT host, helper, installed, signed_in, checked_at, last_error
             FROM helper_status ORDER BY host, helper",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(HelperStatus {
                host: r.get(0)?,
                helper: r.get(1)?,
                installed: r.get(2)?,
                signed_in: r.get(3)?,
                checked_at: r.get(4)?,
                last_error: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Helpers HQ last saw installed on some computer, whatever their sign-in.
pub fn any_installed(db: &Database) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM helper_status WHERE installed = 1)",
            [],
            |r| r.get(0),
        )?)
    })
}

/// Whether HQ has ever checked any computer.
pub fn any_checked(db: &Database) -> Result<bool> {
    db.with_conn(|c| {
        Ok(
            c.query_row("SELECT EXISTS(SELECT 1 FROM helper_status)", [], |r| {
                r.get(0)
            })?,
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupRun {
    pub id: i64,
    pub host: String,
    pub helper: String,
    pub step: String,
    pub state: String,
    pub tail: String,
    pub urls: Vec<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

pub fn start_run(db: &Database, host: &str, helper: &str, step: &str) -> Result<i64> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO helper_setup_runs (host, helper, step, state, started_at)
             VALUES (?1, ?2, ?3, 'running', ?4)",
            params![host, helper, step, Utc::now().timestamp()],
        )?;
        Ok(c.last_insert_rowid())
    })
}

/// Updates what a running step has printed so far.
pub fn note_run(db: &Database, id: i64, tail: &str, urls: &[String]) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE helper_setup_runs SET tail = ?2, urls = ?3 WHERE id = ?1 AND state = 'running'",
            params![id, tail, serde_json::to_string(urls)?],
        )?;
        Ok(())
    })
}

pub fn finish_run(db: &Database, id: i64, ok: bool, tail: &str, urls: &[String]) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE helper_setup_runs SET state = ?2, tail = ?3, urls = ?4, finished_at = ?5
             WHERE id = ?1",
            params![
                id,
                if ok { "ok" } else { "failed" },
                tail,
                serde_json::to_string(urls)?,
                Utc::now().timestamp()
            ],
        )?;
        Ok(())
    })
}

fn run_from_row(r: &rusqlite::Row) -> rusqlite::Result<SetupRun> {
    let urls: String = r.get(6)?;
    Ok(SetupRun {
        id: r.get(0)?,
        host: r.get(1)?,
        helper: r.get(2)?,
        step: r.get(3)?,
        state: r.get(4)?,
        tail: r.get(5)?,
        urls: serde_json::from_str(&urls).unwrap_or_default(),
        started_at: r.get(7)?,
        finished_at: r.get(8)?,
    })
}

const RUN_COLUMNS: &str =
    "id, host, helper, step, state, tail, urls, started_at, finished_at FROM helper_setup_runs";

pub fn get_run(db: &Database, id: i64) -> Result<Option<SetupRun>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!("SELECT {RUN_COLUMNS} WHERE id = ?1"),
            params![id],
            run_from_row,
        )
        .optional()?)
    })
}

/// The newest run of each helper on each computer, for the setup panel.
pub fn latest_runs(db: &Database) -> Result<Vec<SetupRun>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {RUN_COLUMNS} WHERE id IN (SELECT MAX(id) FROM helper_setup_runs GROUP BY host, helper)
             ORDER BY host, helper"
        ))?;
        let rows = stmt.query_map([], run_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Runs still marked running that started before `before` (the process that ran them is gone).
pub fn fail_stale_runs(db: &Database, before: i64) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE helper_setup_runs SET state = 'failed', finished_at = ?1,
               tail = CASE WHEN tail = '' THEN 'HQ stopped before this finished.' ELSE tail END
             WHERE state = 'running' AND started_at < ?1",
            params![before],
        )?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_that_cannot_tell_keeps_the_known_sign_in() {
        let db = Database::open_memory().unwrap();
        record(&db, "laptop", "cursor", true, Some(true), None).unwrap();
        record(
            &db,
            "laptop",
            "cursor",
            true,
            None,
            Some("status timed out"),
        )
        .unwrap();
        let s = &list(&db).unwrap()[0];
        assert_eq!(s.signed_in, Some(true));
        assert_eq!(s.last_error.as_deref(), Some("status timed out"));
        record(&db, "laptop", "cursor", false, None, None).unwrap();
        assert_eq!(list(&db).unwrap()[0].signed_in, None);
        assert!(!any_installed(&db).unwrap());
        assert!(any_checked(&db).unwrap());
    }

    #[test]
    fn a_run_is_followed_from_start_to_finish() {
        let db = Database::open_memory().unwrap();
        let id = start_run(&db, "laptop", "codex", "login").unwrap();
        note_run(&db, id, "open this link", &["https://example.com/x".into()]).unwrap();
        let run = get_run(&db, id).unwrap().unwrap();
        assert_eq!(run.state, "running");
        assert_eq!(run.urls, ["https://example.com/x"]);
        finish_run(&db, id, true, "done", &[]).unwrap();
        let run = latest_runs(&db).unwrap().remove(0);
        assert_eq!((run.state.as_str(), run.tail.as_str()), ("ok", "done"));
        assert!(run.finished_at.is_some());
        // A finished run is not touched by later notes.
        note_run(&db, id, "late", &[]).unwrap();
        assert_eq!(get_run(&db, id).unwrap().unwrap().tail, "done");
    }

    #[test]
    fn a_run_left_running_by_a_stopped_process_is_failed() {
        let db = Database::open_memory().unwrap();
        let id = start_run(&db, "laptop", "codex", "install").unwrap();
        assert_eq!(fail_stale_runs(&db, i64::MAX).unwrap(), 1);
        assert_eq!(get_run(&db, id).unwrap().unwrap().state, "failed");
    }
}
