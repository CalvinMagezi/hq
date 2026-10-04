//! Stored Copilot credit snapshots. Timestamps are UTC RFC 3339 with second precision, so string
//! order is time order.

use anyhow::Result;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{Connection, params};

pub const RETENTION_DAYS: i64 = 45;

#[derive(Debug, Clone, PartialEq)]
pub struct UsageSample {
    pub ts: DateTime<Utc>,
    pub login: Option<String>,
    pub credits_used: f64,
    pub remaining: f64,
    pub entitlement: f64,
    pub reset_at: Option<DateTime<Utc>>,
}

fn fmt(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

pub fn insert_sample(conn: &Connection, s: &UsageSample) -> Result<()> {
    conn.execute(
        "INSERT INTO copilot_usage_samples (ts, login, credits_used, remaining, entitlement, reset_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            fmt(s.ts),
            s.login,
            s.credits_used,
            s.remaining,
            s.entitlement,
            s.reset_at.map(fmt)
        ],
    )?;
    Ok(())
}

type RawRow = (String, Option<String>, f64, f64, f64, Option<String>);

fn to_sample(row: RawRow) -> Option<UsageSample> {
    let (ts, login, credits_used, remaining, entitlement, reset_at) = row;
    Some(UsageSample {
        ts: parse(&ts)?,
        login,
        credits_used,
        remaining,
        entitlement,
        reset_at: reset_at.as_deref().and_then(parse),
    })
}

/// Samples at or after `since`, oldest first.
pub fn list_samples_since(conn: &Connection, since: DateTime<Utc>) -> Result<Vec<UsageSample>> {
    let mut stmt = conn.prepare(
        "SELECT ts, login, credits_used, remaining, entitlement, reset_at
           FROM copilot_usage_samples WHERE ts >= ?1 ORDER BY ts ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![fmt(since)], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.extend(to_sample(row?));
    }
    Ok(out)
}

/// Delete samples older than the retention window; returns rows removed.
pub fn prune(conn: &Connection, now: DateTime<Utc>) -> Result<usize> {
    let cutoff = fmt(now - Duration::days(RETENTION_DAYS));
    Ok(conn.execute(
        "DELETE FROM copilot_usage_samples WHERE ts < ?1",
        params![cutoff],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn sample(ts: DateTime<Utc>, used: f64) -> UsageSample {
        UsageSample {
            ts,
            login: Some("u".into()),
            credits_used: used,
            remaining: 100.0 - used,
            entitlement: 100.0,
            reset_at: Some(ts + Duration::days(3)),
        }
    }

    #[test]
    fn stores_lists_in_order_and_prunes_old_rows() {
        let db = Database::open_memory().unwrap();
        let now = Utc::now();
        let old = now - Duration::days(RETENTION_DAYS + 1);
        let recent = now - Duration::hours(2);
        db.with_conn(|c| {
            insert_sample(c, &sample(now, 30.0))?;
            insert_sample(c, &sample(old, 1.0))?;
            insert_sample(c, &sample(recent, 20.0))?;
            let since = list_samples_since(c, now - Duration::hours(3))?;
            let used: Vec<f64> = since.iter().map(|s| s.credits_used).collect();
            assert_eq!(used, vec![20.0, 30.0]);
            assert_eq!(prune(c, now)?, 1);
            assert_eq!(list_samples_since(c, old - Duration::days(1))?.len(), 2);
            Ok(())
        })
        .unwrap();
    }
}
