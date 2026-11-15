//! Stored lane quota readings, one row per (host, lane, observed_at). Timestamps are UTC RFC 3339
//! with second precision, so string order is time order. The windows are kept as JSON because a
//! lane may report any number of them.

use anyhow::Result;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hq_core::quota::{Confidence, LaneQuota, QuotaSource, QuotaWindow};
use rusqlite::{Connection, params};

pub const RETENTION_DAYS: i64 = 35;

/// A reading and the host that gave it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub host: String,
    pub quota: LaneQuota,
}

fn fmt(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn name_of<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn from_name<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

/// Stores a reading. A reading seen twice (a host's cached answer) is stored once. Returns
/// whether a row was added.
pub fn insert(conn: &Connection, host: &str, q: &LaneQuota) -> Result<bool> {
    let added = conn.execute(
        "INSERT OR IGNORE INTO lane_quota_samples
           (host, lane, observed_at, account, source, confidence, windows, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            host,
            q.lane,
            fmt(q.observed_at),
            q.account,
            name_of(&q.source),
            name_of(&q.confidence),
            serde_json::to_string(&q.windows)?,
            q.reason
        ],
    )?;
    Ok(added > 0)
}

type Raw = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
);

fn to_sample(raw: Raw) -> Option<Sample> {
    let (host, lane, observed_at, account, source, confidence, windows, reason) = raw;
    let windows: Vec<QuotaWindow> = serde_json::from_str(&windows).ok()?;
    Some(Sample {
        host,
        quota: LaneQuota {
            lane,
            account,
            windows,
            source: from_name::<QuotaSource>(&source)?,
            observed_at: DateTime::parse_from_rfc3339(&observed_at)
                .ok()?
                .with_timezone(&Utc),
            confidence: from_name::<Confidence>(&confidence)?,
            reason,
        },
    })
}

/// The newest reading of every (host, lane), ordered by host then lane.
pub fn latest_per_lane(conn: &Connection) -> Result<Vec<Sample>> {
    let mut stmt = conn.prepare(
        "SELECT s.host, s.lane, s.observed_at, s.account, s.source, s.confidence, s.windows, s.reason
           FROM lane_quota_samples s
          WHERE s.id = (SELECT id FROM lane_quota_samples
                         WHERE host = s.host AND lane = s.lane
                         ORDER BY observed_at DESC, id DESC LIMIT 1)
          ORDER BY s.host, s.lane",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.extend(to_sample(row?));
    }
    Ok(out)
}

/// Deletes readings older than the retention window; returns rows removed.
pub fn prune(conn: &Connection, now: DateTime<Utc>) -> Result<usize> {
    let cutoff = fmt(now - Duration::days(RETENTION_DAYS));
    Ok(conn.execute(
        "DELETE FROM lane_quota_samples WHERE observed_at < ?1",
        params![cutoff],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn reading(lane: &str, at: DateTime<Utc>, used: f64) -> LaneQuota {
        LaneQuota {
            lane: lane.into(),
            account: Some("seat".into()),
            windows: vec![QuotaWindow {
                label: "week".into(),
                kind: hq_core::quota::WindowKind::Weekly,
                used_percent: Some(used),
                remaining: None,
                resets_at: Some(at + Duration::days(2)),
            }],
            source: QuotaSource::Api,
            observed_at: at,
            confidence: Confidence::Live,
            reason: None,
        }
    }

    #[test]
    fn keeps_the_newest_reading_per_host_and_lane_and_dedupes() {
        let db = Database::open_memory().unwrap();
        let now = Utc::now();
        db.with_conn(|c| {
            assert!(insert(
                c,
                "a",
                &reading("claude", now - Duration::hours(1), 10.0)
            )?);
            assert!(insert(c, "a", &reading("claude", now, 20.0))?);
            assert!(!insert(c, "a", &reading("claude", now, 20.0))?);
            assert!(insert(
                c,
                "b",
                &reading("claude", now - Duration::hours(3), 5.0)
            )?);
            assert!(insert(c, "a", &reading("codex", now, 1.0))?);
            let latest = latest_per_lane(c)?;
            let keys: Vec<_> = latest
                .iter()
                .map(|s| (s.host.as_str(), s.quota.lane.as_str()))
                .collect();
            assert_eq!(keys, [("a", "claude"), ("a", "codex"), ("b", "claude")]);
            assert_eq!(latest[0].quota.windows[0].used_percent, Some(20.0));
            assert_eq!(latest[0].quota.source, QuotaSource::Api);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn prunes_past_retention() {
        let db = Database::open_memory().unwrap();
        let now = Utc::now();
        db.with_conn(|c| {
            insert(
                c,
                "a",
                &reading("claude", now - Duration::days(RETENTION_DAYS + 1), 1.0),
            )?;
            insert(c, "a", &reading("codex", now, 1.0))?;
            assert_eq!(prune(c, now)?, 1);
            assert_eq!(latest_per_lane(c)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
