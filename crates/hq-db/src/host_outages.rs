//! Hosts with running sessions that stopped answering (`sql/103_host_outages.sql`). The
//! session supervisor opens an outage on the first failed poll, raises one alert once it has
//! lasted long enough, and ends it when the host answers again. The row outlives a daemon
//! restart, so the alert is neither lost nor repeated.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outage {
    pub host: String,
    /// Unix seconds of the first poll that failed.
    pub since: i64,
    /// The last poll's error.
    pub detail: String,
    /// The "not answering" inbox item, once raised.
    pub alert_item_id: Option<String>,
}

impl Outage {
    /// Whole minutes the host has been silent at `now`.
    pub fn minutes(&self, now: i64) -> i64 {
        (now - self.since).max(0) / 60
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Outage> {
    Ok(Outage {
        host: r.get(0)?,
        since: r.get(1)?,
        detail: r.get(2)?,
        alert_item_id: r.get(3)?,
    })
}

/// Records that `host` did not answer at `now`. The first failure opens the outage; later ones
/// only refresh its detail. Returns the outage as it stands.
pub fn note_silent(db: &Database, host: &str, detail: &str, now: i64) -> Result<Outage> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO host_outages (host, since, detail) VALUES (?1, ?2, ?3) \
             ON CONFLICT(host) DO UPDATE SET detail = excluded.detail",
            params![host, now, detail],
        )?;
        Ok(c.query_row(
            "SELECT host, since, detail, alert_item_id FROM host_outages WHERE host = ?1",
            params![host],
            row,
        )?)
    })
}

/// Claims the alert for the outage that began at `since`, naming the inbox item. True for
/// exactly one caller, so two overlapping sweeps raise one alert.
pub fn claim_alert(db: &Database, host: &str, since: i64, item_id: &str) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE host_outages SET alert_item_id = ?3 \
             WHERE host = ?1 AND since = ?2 AND alert_item_id IS NULL",
            params![host, since, item_id],
        )?;
        Ok(n == 1)
    })
}

/// Ends the outage of `host` and returns it, to exactly one caller. `None` when it had none.
pub fn end(db: &Database, host: &str) -> Result<Option<Outage>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "DELETE FROM host_outages WHERE host = ?1 \
             RETURNING host, since, detail, alert_item_id",
            params![host],
            row,
        )
        .optional()?)
    })
}

/// Every open outage.
pub fn list(db: &Database) -> Result<Vec<Outage>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT host, since, detail, alert_item_id FROM host_outages ORDER BY since",
        )?;
        let rows = stmt.query_map([], row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outage_opens_once_alerts_once_and_ends_once() {
        let db = Database::open_memory().unwrap();
        let first = note_silent(&db, "laptop", "connect timed out", 1_000).unwrap();
        assert_eq!(first.since, 1_000);
        let later = note_silent(&db, "laptop", "no route to host", 1_400).unwrap();
        assert_eq!(later.since, 1_000, "a later failure keeps the start");
        assert_eq!(later.detail, "no route to host");
        assert_eq!(later.minutes(1_400), 6);

        assert!(
            !claim_alert(&db, "laptop", 999, "item-0").unwrap(),
            "another outage's start"
        );
        assert!(claim_alert(&db, "laptop", 1_000, "item-1").unwrap());
        assert!(
            !claim_alert(&db, "laptop", 1_000, "item-2").unwrap(),
            "alerted once"
        );
        assert_eq!(list(&db).unwrap().len(), 1);

        let ended = end(&db, "laptop").unwrap().unwrap();
        assert_eq!(ended.alert_item_id.as_deref(), Some("item-1"));
        assert!(end(&db, "laptop").unwrap().is_none(), "ended once");
        assert!(list(&db).unwrap().is_empty());

        let again = note_silent(&db, "laptop", "timed out", 5_000).unwrap();
        assert_eq!(again.since, 5_000, "a new outage starts afresh");
        assert!(again.alert_item_id.is_none());
    }
}
