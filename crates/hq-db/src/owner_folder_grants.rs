//! Folders the owner allowed for one chat (`sql/137_owner_folder_grants.sql`). Inside a live
//! grant, that chat's file edits and shell commands run without an approval each. Written only
//! by the owner's answer route and revoked only through the owner's web routes; no tool a model
//! can call reaches either. Callers pass the folder already canonical; this module stores it.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use crate::Database;

/// How long a grant lasts when the owner does not say otherwise.
pub const DEFAULT_TTL_SECS: i64 = 8 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OwnerFolderGrant {
    pub id: i64,
    pub root: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub uses: i64,
    pub last_used_at: Option<i64>,
}

/// Allows `root` for chat `scope` until `now + ttl_secs`. A second grant of a live folder
/// restarts its clock instead of adding a row.
pub fn grant(db: &Database, scope: &str, root: &str, ttl_secs: i64) -> Result<()> {
    grant_at(db, scope, root, ttl_secs, Utc::now().timestamp())
}

fn grant_at(db: &Database, scope: &str, root: &str, ttl_secs: i64, now: i64) -> Result<()> {
    db.with_conn(|c| {
        // A lapsed grant still holds the unique slot until it is revoked: retire it first.
        c.execute(
            "UPDATE owner_folder_grants SET revoked_at = ?3
             WHERE scope = ?1 AND root = ?2 AND revoked_at IS NULL AND expires_at <= ?3",
            params![scope, root, now],
        )?;
        c.execute(
            "INSERT INTO owner_folder_grants (scope, root, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(scope, root) WHERE revoked_at IS NULL
             DO UPDATE SET expires_at = excluded.expires_at",
            params![scope, root, now, now + ttl_secs],
        )?;
        Ok(())
    })
}

/// The folders allowed for `scope` right now.
pub fn live_roots(db: &Database, scope: &str) -> Result<Vec<String>> {
    live_roots_at(db, scope, Utc::now().timestamp())
}

fn live_roots_at(db: &Database, scope: &str, now: i64) -> Result<Vec<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT root FROM owner_folder_grants
             WHERE scope = ?1 AND revoked_at IS NULL AND expires_at > ?2 ORDER BY root",
        )?;
        let rows = stmt.query_map(params![scope, now], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    })
}

/// The live grants of `scope`, for the chat's Details panel.
pub fn list(db: &Database, scope: &str) -> Result<Vec<OwnerFolderGrant>> {
    let now = Utc::now().timestamp();
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT id, root, created_at, expires_at, uses, last_used_at FROM owner_folder_grants
             WHERE scope = ?1 AND revoked_at IS NULL AND expires_at > ?2 ORDER BY root",
        )?;
        let rows = stmt.query_map(params![scope, now], |r| {
            Ok(OwnerFolderGrant {
                id: r.get(0)?,
                root: r.get(1)?,
                created_at: r.get(2)?,
                expires_at: r.get(3)?,
                uses: r.get(4)?,
                last_used_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Revokes grant `id`, but only if it belongs to `scope`. True when one was revoked.
pub fn revoke(db: &Database, scope: &str, id: i64) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE owner_folder_grants SET revoked_at = ?3
             WHERE id = ?1 AND scope = ?2 AND revoked_at IS NULL",
            params![id, scope, Utc::now().timestamp()],
        )?;
        Ok(n > 0)
    })
}

/// Revokes every grant of `scope` (the chat was archived or deleted).
pub fn revoke_all(db: &Database, scope: &str) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE owner_folder_grants SET revoked_at = ?2 WHERE scope = ?1 AND revoked_at IS NULL",
            params![scope, Utc::now().timestamp()],
        )?)
    })
}

/// Counts one call a grant of `scope` covering `root` let through.
pub fn record_use(db: &Database, scope: &str, root: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE owner_folder_grants SET uses = uses + 1, last_used_at = ?3
             WHERE scope = ?1 AND root = ?2 AND revoked_at IS NULL",
            params![scope, root, Utc::now().timestamp()],
        )?;
        Ok(())
    })
}

/// Owner-approval requests this chat raised for `tools` in the last `window_secs`. A retry of
/// the same call is the same request, so it does not count twice.
pub fn recent_requests(
    db: &Database,
    key_prefix: &str,
    tools: &[&str],
    window_secs: i64,
) -> Result<usize> {
    let floor = (Utc::now() - chrono::Duration::seconds(window_secs))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT o.tool FROM owner_approvals o JOIN value_items v ON v.id = o.item_id
             WHERE substr(v.dedup_key, 1, length(?1)) = ?1 AND o.created_at >= ?2",
        )?;
        let rows = stmt.query_map(params![key_prefix, floor], |r| r.get::<_, String>(0))?;
        let mut n = 0;
        for tool in rows {
            if tools.contains(&tool?.as_str()) {
                n += 1;
            }
        }
        Ok(n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Database {
        Database::open_memory().unwrap()
    }

    #[test]
    fn a_grant_is_live_until_it_expires_and_only_for_its_chat() {
        let db = db();
        grant_at(&db, "web:t1", "/work/app", 100, 1_000).unwrap();
        assert_eq!(live_roots_at(&db, "web:t1", 1_050).unwrap(), ["/work/app"]);
        assert!(live_roots_at(&db, "web:t1", 1_100).unwrap().is_empty());
        assert!(live_roots_at(&db, "web:t2", 1_050).unwrap().is_empty());
    }

    #[test]
    fn granting_again_restarts_the_clock_and_a_lapsed_grant_can_be_granted_anew() {
        let db = db();
        grant_at(&db, "web:t1", "/work/app", 100, 1_000).unwrap();
        grant_at(&db, "web:t1", "/work/app", 100, 1_080).unwrap();
        assert_eq!(live_roots_at(&db, "web:t1", 1_150).unwrap().len(), 1);
        grant_at(&db, "web:t1", "/work/app", 100, 5_000).unwrap();
        assert_eq!(live_roots_at(&db, "web:t1", 5_050).unwrap().len(), 1);
    }

    #[test]
    fn revoke_needs_the_owning_chat() {
        let db = db();
        grant(&db, "web:t1", "/work/app", DEFAULT_TTL_SECS).unwrap();
        let id = list(&db, "web:t1").unwrap()[0].id;
        assert!(!revoke(&db, "web:t2", id).unwrap());
        assert!(revoke(&db, "web:t1", id).unwrap());
        assert!(live_roots(&db, "web:t1").unwrap().is_empty());
    }

    #[test]
    fn uses_are_counted_per_grant() {
        let db = db();
        grant(&db, "web:t1", "/work/app", DEFAULT_TTL_SECS).unwrap();
        record_use(&db, "web:t1", "/work/app").unwrap();
        record_use(&db, "web:t1", "/work/app").unwrap();
        assert_eq!(list(&db, "web:t1").unwrap()[0].uses, 2);
    }

    #[test]
    fn revoke_all_clears_a_chat() {
        let db = db();
        grant(&db, "web:t1", "/a/x", DEFAULT_TTL_SECS).unwrap();
        grant(&db, "web:t1", "/a/y", DEFAULT_TTL_SECS).unwrap();
        assert_eq!(revoke_all(&db, "web:t1").unwrap(), 2);
    }
}
