//! Browsers subscribed to Web Push (`docs/WEB_PUSH.md`): one row per `PushSubscription`, keyed
//! by its endpoint. The keys are the browser's public half and auth secret, base64url.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub last_ok_at: Option<String>,
    pub failures: i64,
    pub last_error: Option<String>,
}

/// Saves a subscription, replacing the keys of one with the same endpoint (a browser
/// re-sending its subscription after a key rotation).
pub fn upsert(
    db: &Database,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    user_agent: Option<&str>,
) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO push_subscriptions (endpoint, p256dh, auth, user_agent, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth, \
             user_agent = excluded.user_agent, failures = 0, last_error = NULL",
            params![endpoint, p256dh, auth, user_agent, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })
}

/// Removes a subscription. Returns whether there was one.
pub fn delete(db: &Database, endpoint: &str) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "DELETE FROM push_subscriptions WHERE endpoint = ?1",
            params![endpoint],
        )?;
        Ok(n > 0)
    })
}

pub fn list(db: &Database) -> Result<Vec<PushSubscription>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT endpoint, p256dh, auth, user_agent, created_at, last_ok_at, failures, last_error \
             FROM push_subscriptions ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PushSubscription {
                    endpoint: r.get(0)?,
                    p256dh: r.get(1)?,
                    auth: r.get(2)?,
                    user_agent: r.get(3)?,
                    created_at: r.get(4)?,
                    last_ok_at: r.get(5)?,
                    failures: r.get(6)?,
                    last_error: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

pub fn get(db: &Database, endpoint: &str) -> Result<Option<PushSubscription>> {
    Ok(list(db)?.into_iter().find(|s| s.endpoint == endpoint))
}

pub fn record_success(db: &Database, endpoint: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE push_subscriptions SET last_ok_at = ?2, failures = 0, last_error = NULL \
             WHERE endpoint = ?1",
            params![endpoint, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })
}

/// Records a failed push. Returns the failures in a row, counting this one.
pub fn record_failure(db: &Database, endpoint: &str, error: &str) -> Result<i64> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE push_subscriptions SET failures = failures + 1, last_error = ?2 \
             WHERE endpoint = ?1",
            params![endpoint, error],
        )?;
        let n: i64 = c
            .query_row(
                "SELECT failures FROM push_subscriptions WHERE endpoint = ?1",
                params![endpoint],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_replaces_keys_and_clears_failures() {
        let db = Database::open_memory().unwrap();
        let ep = "https://fcm.googleapis.com/fcm/send/abc";
        upsert(&db, ep, "k1", "a1", Some("Firefox")).unwrap();
        assert_eq!(record_failure(&db, ep, "500").unwrap(), 1);
        assert_eq!(record_failure(&db, ep, "500").unwrap(), 2);
        upsert(&db, ep, "k2", "a2", None).unwrap();

        let all = list(&db).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].p256dh, "k2");
        assert_eq!(all[0].failures, 0);
        assert_eq!(all[0].last_error, None);

        record_success(&db, ep).unwrap();
        assert!(get(&db, ep).unwrap().unwrap().last_ok_at.is_some());
        assert!(delete(&db, ep).unwrap());
        assert!(!delete(&db, ep).unwrap());
        assert!(list(&db).unwrap().is_empty());
    }
}
