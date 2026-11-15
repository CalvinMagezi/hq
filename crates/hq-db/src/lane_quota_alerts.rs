//! The lane quota alerts already raised, one row per (host, lane, window, level, reset time).

use anyhow::Result;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{Connection, params};

/// Alert rows are only needed while their window can still be open; a month covers the longest.
const RETENTION_DAYS: i64 = 60;

fn fmt(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Records that `level` was raised for this window. True when it was new, so the caller raises
/// the alert; false when the same window already alerted at that level.
pub fn claim(
    conn: &Connection,
    (host, lane, window): (&str, &str, &str),
    level: &str,
    resets_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<bool> {
    let added = conn.execute(
        "INSERT OR IGNORE INTO lane_quota_alerts (host, lane, window, level, resets_at, raised_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![host, lane, window, level, fmt(resets_at), fmt(now)],
    )?;
    Ok(added > 0)
}

/// Takes a claim back, when the alert it stood for could not be raised.
pub fn release(
    conn: &Connection,
    (host, lane, window): (&str, &str, &str),
    level: &str,
    resets_at: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        "DELETE FROM lane_quota_alerts
          WHERE host = ?1 AND lane = ?2 AND window = ?3 AND level = ?4 AND resets_at = ?5",
        params![host, lane, window, level, fmt(resets_at)],
    )?;
    Ok(())
}

/// Deletes rows raised before the retention window; returns rows removed.
pub fn prune(conn: &Connection, now: DateTime<Utc>) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM lane_quota_alerts WHERE raised_at < ?1",
        params![fmt(now - Duration::days(RETENTION_DAYS))],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[test]
    fn a_window_alerts_once_per_level_and_again_after_it_resets() {
        let db = Database::open_memory().unwrap();
        let now = Utc::now();
        let reset = now + Duration::days(2);
        let key = ("h", "claude", "week");
        db.with_conn(|c| {
            assert!(claim(c, key, "warn", reset, now)?);
            assert!(!claim(c, key, "warn", reset, now)?);
            assert!(claim(c, key, "block", reset, now)?);
            assert!(claim(c, key, "warn", reset + Duration::days(7), now)?);
            release(c, key, "warn", reset)?;
            assert!(claim(c, key, "warn", reset, now)?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn prunes_old_rows() {
        let db = Database::open_memory().unwrap();
        let now = Utc::now();
        db.with_conn(|c| {
            claim(
                c,
                ("h", "l", "w"),
                "warn",
                now,
                now - Duration::days(RETENTION_DAYS + 1),
            )?;
            assert_eq!(prune(c, now)?, 1);
            Ok(())
        })
        .unwrap();
    }
}
