//! What HQ Coder must not lose about a task's run. The task stays the record of the work.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};

pub const PHASE_QUEUED: &str = "queued";
pub const PHASE_WORKING: &str = "working";
pub const PHASE_DONE: &str = "done";
pub const STOP_BUDGET: &str = "budget";
pub const STOP_CREDITS: &str = "credits";

const COLUMNS: &str = "task_id, repo, budget_usd, credit_cap, credit_start, model, phase, attempts, \
     spent_usd, stop_reason, origin_thread, next_check_at";

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Job {
    pub task_id: String,
    pub repo: String,
    pub budget_usd: f64,
    pub credit_cap: Option<f64>,
    /// The account's credit counter when the job first started.
    pub credit_start: Option<f64>,
    pub model: Option<String>,
    pub phase: String,
    pub attempts: i64,
    pub spent_usd: f64,
    pub stop_reason: Option<String>,
    pub origin_thread: Option<String>,
    /// Not before this time (RFC 3339), after a run that failed for a passing reason.
    pub next_check_at: Option<String>,
}

impl Job {
    /// Whether the job's wait, if it has one, is over.
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.next_check_at
            .as_deref()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .is_none_or(|at| at <= now)
    }

    /// The counter value at which the job must stop, once it has a starting reading.
    pub fn credit_ceiling(&self) -> Option<f64> {
        Some(self.credit_start? + self.credit_cap?)
    }

    fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            task_id: r.get(0)?,
            repo: r.get(1)?,
            budget_usd: r.get(2)?,
            credit_cap: r.get(3)?,
            credit_start: r.get(4)?,
            model: r.get(5)?,
            phase: r.get(6)?,
            attempts: r.get(7)?,
            spent_usd: r.get(8)?,
            stop_reason: r.get(9)?,
            origin_thread: r.get(10)?,
            next_check_at: r.get(11)?,
        })
    }
}

/// The settings a job starts with.
pub struct NewJob<'a> {
    pub task_id: &'a str,
    pub repo: &'a str,
    pub budget_usd: f64,
    pub credit_cap: Option<f64>,
    pub model: Option<&'a str>,
    pub origin_thread: Option<&'a str>,
}

/// The task's job, made from `new` the first time. An existing job keeps its own settings.
pub fn ensure(conn: &Connection, new: &NewJob<'_>) -> Result<Job> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO coder_jobs
           (task_id, repo, budget_usd, credit_cap, model, origin_thread, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
        params![
            new.task_id,
            new.repo,
            new.budget_usd,
            new.credit_cap,
            new.model,
            new.origin_thread,
            now
        ],
    )?;
    get(conn, new.task_id)?.ok_or_else(|| anyhow::anyhow!("job {} was not saved", new.task_id))
}

pub fn get(conn: &Connection, task_id: &str) -> Result<Option<Job>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM coder_jobs WHERE task_id = ?1"),
            params![task_id],
            Job::from_row,
        )
        .optional()?)
}

/// One update a run makes, stamped so a job's last change is known.
fn touch(conn: &Connection, task_id: &str, set: &str, value: &dyn rusqlite::ToSql) -> Result<()> {
    conn.execute(
        &format!("UPDATE coder_jobs SET {set}, updated_at = ?2 WHERE task_id = ?3"),
        params![value, Utc::now().to_rfc3339(), task_id],
    )?;
    Ok(())
}

/// Called after each model call, so a killed run has already paid for what it used.
pub fn add_spend(conn: &Connection, task_id: &str, usd: f64) -> Result<()> {
    touch(conn, task_id, "spent_usd = spent_usd + ?1", &usd)
}

pub fn set_credit_start(conn: &Connection, task_id: &str, reading: f64) -> Result<()> {
    touch(
        conn,
        task_id,
        "credit_start = COALESCE(credit_start, ?1)",
        &reading,
    )
}

pub fn set_stop_reason(conn: &Connection, task_id: &str, reason: &str) -> Result<()> {
    touch(conn, task_id, "stop_reason = ?1", &reason)
}

/// Back in the queue, not to start again for `wait_secs`.
pub fn requeue(conn: &Connection, task_id: &str, wait_secs: i64) -> Result<()> {
    let at = (Utc::now() + Duration::seconds(wait_secs)).to_rfc3339();
    conn.execute(
        "UPDATE coder_jobs SET phase = ?1, next_check_at = ?2, updated_at = ?3 WHERE task_id = ?4",
        params![PHASE_QUEUED, at, Utc::now().to_rfc3339(), task_id],
    )?;
    Ok(())
}

/// The owner sent the job back in: its tries and last stop are forgotten, and a new limit, when
/// given, replaces the old one.
pub fn reset(
    conn: &Connection,
    task_id: &str,
    budget_usd: Option<f64>,
    credit_cap: Option<f64>,
) -> Result<()> {
    conn.execute(
        "UPDATE coder_jobs SET attempts = 0, stop_reason = NULL, next_check_at = NULL, phase = ?1,
           budget_usd = COALESCE(?2, budget_usd), credit_cap = COALESCE(?3, credit_cap),
           updated_at = ?4 WHERE task_id = ?5",
        params![
            PHASE_QUEUED,
            budget_usd,
            credit_cap,
            Utc::now().to_rfc3339(),
            task_id
        ],
    )?;
    Ok(())
}

/// Newest first.
pub fn recent(conn: &Connection, limit: usize) -> Result<Vec<Job>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM coder_jobs ORDER BY updated_at DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map(params![limit as i64], Job::from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Jobs still marked as working that a chat is waiting to hear about.
pub fn working_with_origin(conn: &Connection) -> Result<Vec<Job>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM coder_jobs WHERE phase = ?1 AND origin_thread IS NOT NULL"
    ))?;
    let rows = stmt.query_map(params![PHASE_WORKING], Job::from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn finish(conn: &Connection, task_id: &str) -> Result<()> {
    touch(conn, task_id, "phase = ?1", &PHASE_DONE)
}

pub fn begin_attempt(conn: &Connection, task_id: &str) -> Result<()> {
    touch(
        conn,
        task_id,
        "attempts = attempts + 1, stop_reason = NULL, phase = ?1",
        &PHASE_WORKING,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn_with_task() -> (crate::Database, String) {
        let db = crate::Database::open_memory().unwrap();
        let id = "tk-job".to_string();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO initiatives (id, space_id, name, slug, id_prefix) VALUES ('in-1', 'personal', 'Work', 'work', 'W')",
                [],
            )?;
            crate::tasks::create_task(
                c,
                &id,
                "in-1",
                &crate::tasks::NewTask {
                    title: "job",
                    created_by: "test",
                    ..Default::default()
                },
            )?;
            Ok(())
        })
        .unwrap();
        (db, id)
    }

    fn new_job(task_id: &str) -> NewJob<'_> {
        NewJob {
            task_id,
            repo: "o/r",
            budget_usd: 1.0,
            credit_cap: Some(50.0),
            model: None,
            origin_thread: None,
        }
    }

    #[test]
    fn a_job_keeps_its_settings_and_what_it_has_spent() {
        let (db, id) = conn_with_task();
        db.with_conn(|c| {
            ensure(c, &new_job(&id))?;
            add_spend(c, &id, 0.25)?;
            add_spend(c, &id, 0.25)?;
            let again = NewJob {
                budget_usd: 9.0,
                ..new_job(&id)
            };
            let job = ensure(c, &again)?;
            assert_eq!(job.budget_usd, 1.0, "an existing job keeps its budget");
            assert_eq!(job.spent_usd, 0.5);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_credit_ceiling_comes_from_the_first_reading_only() {
        let (db, id) = conn_with_task();
        db.with_conn(|c| {
            let job = ensure(c, &new_job(&id))?;
            assert_eq!(job.credit_ceiling(), None);
            set_credit_start(c, &id, 100.0)?;
            set_credit_start(c, &id, 500.0)?;
            assert_eq!(get(c, &id)?.unwrap().credit_ceiling(), Some(150.0));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_new_attempt_clears_the_last_stop() {
        let (db, id) = conn_with_task();
        db.with_conn(|c| {
            ensure(c, &new_job(&id))?;
            set_stop_reason(c, &id, STOP_BUDGET)?;
            begin_attempt(c, &id)?;
            let job = get(c, &id)?.unwrap();
            assert_eq!(job.stop_reason, None);
            assert_eq!((job.attempts, job.phase.as_str()), (1, PHASE_WORKING));
            Ok(())
        })
        .unwrap();
    }
}
