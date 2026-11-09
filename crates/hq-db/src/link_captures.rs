//! The queue of shared links waiting to be summarised into the note made for them.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};
use std::collections::HashSet;

const STATE_PENDING: &str = "pending";
const PURGE_AFTER_DAYS: i64 = 30;

/// How a capture ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Done,
    /// The owner changed the note first, so it was left alone.
    Skipped,
    Failed,
}

impl Finish {
    fn as_str(self) -> &'static str {
        match self {
            Finish::Done => "done",
            Finish::Skipped => "skipped",
            Finish::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueCapture {
    pub path: String,
    pub url: String,
    pub base_hash: String,
    pub attempts: i64,
}

/// Queues a note's link. A note already waiting stays one entry; a new note at the path of a
/// finished capture starts fresh.
pub fn enqueue(conn: &Connection, path: &str, url: &str, base_hash: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO link_captures (path, url, base_hash, next_try_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(path) DO UPDATE SET url = excluded.url, base_hash = excluded.base_hash,
           state = ?5, attempts = 0, reason = NULL, last_try_at = NULL,
           next_try_at = excluded.next_try_at, created_at = excluded.created_at
         WHERE state != ?5",
        params![path, url, base_hash, now, STATE_PENDING],
    )?;
    Ok(())
}

/// The oldest captures ready for a try.
pub fn due(conn: &Connection, now: DateTime<Utc>, limit: usize) -> Result<Vec<DueCapture>> {
    let mut stmt = conn.prepare(
        "SELECT path, url, base_hash, attempts FROM link_captures
         WHERE state = ?1 AND next_try_at <= ?2 ORDER BY created_at LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![STATE_PENDING, now.to_rfc3339(), limit as i64],
        |r| {
            Ok(DueCapture {
                path: r.get(0)?,
                url: r.get(1)?,
                base_hash: r.get(2)?,
                attempts: r.get(3)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Tries started in the day before `now`, the number the daily cap counts.
pub fn tries_in_last_day(conn: &Connection, now: DateTime<Utc>) -> Result<usize> {
    let since = (now - Duration::days(1)).to_rfc3339();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM link_captures WHERE last_try_at >= ?1",
        [since],
        |r| r.get(0),
    )?;
    Ok(count as usize)
}

pub fn retry_at(
    conn: &Connection,
    path: &str,
    next_try_at: DateTime<Utc>,
    reason: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE link_captures SET attempts = attempts + 1, next_try_at = ?2, reason = ?3,
         last_try_at = ?4 WHERE path = ?1",
        params![
            path,
            next_try_at.to_rfc3339(),
            reason,
            Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

pub fn finish(conn: &Connection, path: &str, how: Finish, reason: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE link_captures SET state = ?2, reason = ?3, attempts = attempts + 1,
         last_try_at = ?4 WHERE path = ?1",
        params![path, how.as_str(), reason, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Forgets finished captures past the retention window. Returns how many.
pub fn purge_finished(conn: &Connection, now: DateTime<Utc>) -> Result<usize> {
    let cutoff = (now - Duration::days(PURGE_AFTER_DAYS)).to_rfc3339();
    Ok(conn.execute(
        "DELETE FROM link_captures WHERE state != ?1 AND created_at < ?2",
        params![STATE_PENDING, cutoff],
    )?)
}

/// Saved notes already brought up, so the weekly tip does not repeat one.
pub fn tipped_paths(conn: &Connection) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare("SELECT path FROM collection_tips")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<HashSet<String>>>()?)
}

pub fn record_tip(conn: &Connection, path: &str, now: DateTime<Utc>) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO collection_tips (path, surfaced_at) VALUES (?1, ?2)",
        params![path, now.to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn a_capture_is_queued_once_tried_later_and_leaves_when_finished() {
        let conn = db();
        enqueue(&conn, "Notebooks/Inbox/a.md", "https://example.com/", "h").unwrap();
        enqueue(
            &conn,
            "Notebooks/Inbox/a.md",
            "https://example.com/other",
            "h2",
        )
        .unwrap();
        let now = Utc::now();
        let found = due(&conn, now, 5).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://example.com/");

        retry_at(
            &conn,
            "Notebooks/Inbox/a.md",
            now + Duration::minutes(15),
            "down",
        )
        .unwrap();
        assert!(due(&conn, now, 5).unwrap().is_empty());
        assert_eq!(tries_in_last_day(&conn, now).unwrap(), 1);
        let later = now + Duration::minutes(16);
        assert_eq!(due(&conn, later, 5).unwrap()[0].attempts, 1);

        finish(&conn, "Notebooks/Inbox/a.md", Finish::Done, None).unwrap();
        assert!(due(&conn, later, 5).unwrap().is_empty());

        enqueue(
            &conn,
            "Notebooks/Inbox/a.md",
            "https://example.com/new",
            "h3",
        )
        .unwrap();
        let fresh = due(&conn, Utc::now(), 5).unwrap();
        assert_eq!(fresh[0].url, "https://example.com/new");
        assert_eq!(fresh[0].attempts, 0);
        finish(&conn, "Notebooks/Inbox/a.md", Finish::Done, None).unwrap();
        assert_eq!(
            purge_finished(&conn, now + Duration::days(PURGE_AFTER_DAYS + 1)).unwrap(),
            1
        );
    }
}
