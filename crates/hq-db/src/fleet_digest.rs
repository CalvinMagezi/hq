//! The fleet digest: one rolling inbox item per local day that counts what the helpers did
//! ("7 helpers finished today: 5 ready to check, 2 stopped"), instead of one row per helper.
//!
//! The counts come from the session event log (`harness_session_events`) and the linked task's
//! status, never from screen text. The item is rewritten in place under one dedup key, and its
//! source is device-only (`FLEET_DIGEST_SOURCE`), so it never reaches web push.

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};
use hq_core::types::{FLEET_DIGEST_SOURCE, ValueItem, ValueKind};
use rusqlite::{Connection, params};

use crate::Database;

/// SQLite's own `datetime('now')` shape, which is how the event log stamps rows.
const SQLITE_TIME: &str = "%Y-%m-%d %H:%M:%S";
/// The page a digest opens.
const DIGEST_LINK: &str = "/helpers";

/// What the helpers did in one local day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Helpers that finished a turn.
    pub finished: usize,
    /// Of those, the ones whose linked task now waits for review.
    pub ready_for_review: usize,
    /// Helpers whose process ended normally.
    pub stopped: usize,
    /// Helpers whose process ended with a failing exit code.
    pub failed: usize,
}

impl Counts {
    pub fn is_empty(&self) -> bool {
        *self == Counts::default()
    }
}

/// The dedup key of the digest for `day`.
pub fn key_for(day: NaiveDate) -> String {
    format!("fleet-digest-{day}")
}

fn day_bounds(day: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = |d: NaiveDate| {
        Local
            .from_local_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight exists"))
            .earliest()
            .map_or_else(
                || d.and_hms_opt(0, 0, 0).expect("midnight exists").and_utc(),
                |t| t.with_timezone(&Utc),
            )
    };
    (start(day), start(day + Duration::days(1)))
}

/// The day's counts, read from the event log.
pub fn counts_for(conn: &Connection, day: NaiveDate) -> Result<Counts> {
    let (from, to) = day_bounds(day);
    let (from, to) = (
        from.format(SQLITE_TIME).to_string(),
        to.format(SQLITE_TIME).to_string(),
    );
    let finished: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT session_id) FROM harness_session_events
         WHERE kind = 'finished' AND created_at >= ?1 AND created_at < ?2",
        params![from, to],
        |r| r.get(0),
    )?;
    let ready: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT e.session_id) FROM harness_session_events e
         JOIN harness_sessions s ON s.id = e.session_id
         JOIN tasks t ON t.id = s.mission_id
         WHERE e.kind = 'finished' AND t.status = 'ready_for_review'
           AND e.created_at >= ?1 AND e.created_at < ?2",
        params![from, to],
        |r| r.get(0),
    )?;
    let details: Vec<Option<String>> = conn
        .prepare(
            "SELECT detail FROM harness_session_events
             WHERE kind = 'exited' AND created_at >= ?1 AND created_at < ?2",
        )?
        .query_map(params![from, to], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let failed = details
        .iter()
        .filter(|d| {
            d.as_deref()
                .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
                .and_then(|v| v["exit_code"].as_i64())
                .is_some_and(|code| code != 0)
        })
        .count();
    Ok(Counts {
        finished: finished as usize,
        ready_for_review: ready as usize,
        stopped: details.len() - failed,
        failed,
    })
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The title and body for `counts`: plain words, numbers only.
pub fn wording(counts: &Counts) -> (String, String) {
    let title = if counts.finished > 0 {
        format!(
            "{} finished today",
            plural(counts.finished, "helper", "helpers")
        )
    } else {
        format!(
            "{} stopped today",
            plural(counts.stopped + counts.failed, "helper", "helpers")
        )
    };
    let mut parts = Vec::new();
    if counts.ready_for_review > 0 {
        parts.push(format!("{} ready to check", counts.ready_for_review));
    }
    if counts.stopped > 0 && counts.finished > 0 {
        parts.push(format!("{} stopped", counts.stopped));
    }
    if counts.failed > 0 {
        parts.push(format!("{} failed", counts.failed));
    }
    let body = if parts.is_empty() {
        "Nothing needs you. Open Helpers to see each one.".to_string()
    } else {
        format!("{}. Open Helpers to see each one.", parts.join(", "))
    };
    (title, body)
}

/// Writes the digest for the local day of `now`: inserts it the first time, rewrites it after.
/// Does nothing on a day with no helper activity. Returns the counts it wrote.
pub fn refresh(db: &Database, now: DateTime<Local>) -> Result<Counts> {
    let day = now.date_naive();
    let counts = db.with_conn(|c| counts_for(c, day))?;
    if counts.is_empty() {
        return Ok(counts);
    }
    let (title, body) = wording(&counts);
    let key = key_for(day);
    // A digest the owner dismissed stays dismissed, and one the tidy expired stays expired.
    let updated = db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE value_items SET title = ?2, body = ?3,
                    read_at = CASE WHEN title = ?2 AND body = ?3 THEN read_at ELSE NULL END
             WHERE dedup_key = ?1 AND source_task = ?4 AND state IN ('pending','routed','delivered')",
            params![key, title, body, FLEET_DIGEST_SOURCE],
        )?)
    })?;
    if updated == 0 && !crate::value_items::any_with_dedup(db, &key)? {
        let item = ValueItem::new(FLEET_DIGEST_SOURCE, ValueKind::Fyi, title, body)
            .with_dedup_key(key)
            .with_link(DIGEST_LINK);
        crate::value_items::insert(db, &item)?;
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness_sessions_registry as registry;

    fn session(db: &Database, id: &str) {
        db.with_conn(|c| {
            registry::insert(
                c,
                &registry::NewSession {
                    id,
                    harness: "claude",
                    label: "",
                    cwd: "/tmp/example",
                    mission_id: None,
                    placement: registry::Placement {
                        host: "local",
                        agent_name: id,
                        workspace_id: "w",
                        pane_id: "p",
                    },
                    purpose: None,
                },
            )
        })
        .unwrap();
    }

    fn event(db: &Database, id: &str, kind: &str, detail: Option<&str>) {
        db.with_conn(|c| registry::record_event(c, id, kind, "supervisor", detail))
            .unwrap();
    }

    fn digests(db: &Database) -> Vec<(String, String, String)> {
        db.with_conn(|c| {
            Ok(
                c.prepare("SELECT title, body, state FROM value_items WHERE source_task = ?1")?
                    .query_map([FLEET_DIGEST_SOURCE], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })?
                    .collect::<rusqlite::Result<_>>()?,
            )
        })
        .unwrap()
    }

    #[test]
    fn many_finishes_make_one_item_that_is_rewritten_in_place() {
        let db = Database::open_memory().unwrap();
        for n in 0..5 {
            let id = format!("hs-{n}");
            session(&db, &id);
            event(&db, &id, "finished", None);
            refresh(&db, Local::now()).unwrap();
        }
        let rows = digests(&db);
        assert_eq!(rows.len(), 1, "one item for five finishes");
        assert_eq!(rows[0].0, "5 helpers finished today");

        session(&db, "hs-x");
        event(&db, "hs-x", "exited", Some(r#"{"exit_code":0}"#));
        session(&db, "hs-y");
        event(&db, "hs-y", "exited", Some(r#"{"exit_code":2}"#));
        refresh(&db, Local::now()).unwrap();
        let rows = digests(&db);
        assert_eq!(rows.len(), 1, "still one item");
        assert_eq!(
            rows[0].1,
            "1 stopped, 1 failed. Open Helpers to see each one."
        );
    }

    #[test]
    fn a_quiet_day_makes_no_item_and_the_same_session_counts_once() {
        let db = Database::open_memory().unwrap();
        assert!(refresh(&db, Local::now()).unwrap().is_empty());
        assert!(digests(&db).is_empty());
        session(&db, "hs-1");
        event(&db, "hs-1", "finished", None);
        event(&db, "hs-1", "finished", None);
        assert_eq!(refresh(&db, Local::now()).unwrap().finished, 1);
        assert_eq!(digests(&db)[0].0, "1 helper finished today");
    }

    #[test]
    fn a_changed_digest_shows_unread_and_a_dismissed_one_is_left_alone() {
        let db = Database::open_memory().unwrap();
        session(&db, "hs-1");
        event(&db, "hs-1", "finished", None);
        refresh(&db, Local::now()).unwrap();
        let read = |db: &Database| -> bool {
            db.with_conn(|c| {
                Ok(c.query_row(
                    "SELECT read_at IS NOT NULL FROM value_items WHERE source_task = ?1",
                    [FLEET_DIGEST_SOURCE],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
        };
        db.with_conn(|c| {
            c.execute("UPDATE value_items SET read_at = datetime('now')", [])?;
            Ok(())
        })
        .unwrap();
        refresh(&db, Local::now()).unwrap();
        assert!(read(&db), "same counts: still read");
        session(&db, "hs-2");
        event(&db, "hs-2", "finished", None);
        refresh(&db, Local::now()).unwrap();
        assert!(!read(&db), "new counts: unread again");

        db.with_conn(|c| {
            c.execute("UPDATE value_items SET state = 'dismissed'", [])?;
            Ok(())
        })
        .unwrap();
        session(&db, "hs-3");
        event(&db, "hs-3", "finished", None);
        refresh(&db, Local::now()).unwrap();
        let rows = digests(&db);
        assert_eq!(rows.len(), 1, "no second item");
        assert_eq!(rows[0].0, "2 helpers finished today", "not rewritten");
    }

    #[test]
    fn the_digest_is_device_only_so_it_is_never_pushed() {
        let item = ValueItem::new(FLEET_DIGEST_SOURCE, ValueKind::Fyi, "t", "b");
        assert!(item.is_device_only());
    }
}
