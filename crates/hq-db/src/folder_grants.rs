//! Folders the owner approved for sessions a bounded caller starts (`sql/101_session_folder_grants.sql`).
//! A grant is written only by the code that spends the owner's signed approval of a request, and
//! revoked only through the owner's web routes; no MCP tool reaches either.

use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FolderGrant {
    pub id: i64,
    pub host: String,
    pub path: String,
    pub requested_by: String,
    pub approved_at: i64,
}

/// Records a grant. Granting a folder that already has a live grant keeps the existing one.
pub fn grant(db: &Database, host: &str, path: &str, requested_by: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO session_folder_grants (host, path, requested_by, approved_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(host, path) WHERE revoked_at IS NULL DO NOTHING",
            params![host, path, requested_by, Utc::now().timestamp()],
        )?;
        Ok(())
    })
}

/// The live grants' folders on one host.
pub fn folders_for_host(db: &Database, host: &str) -> Result<Vec<String>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT path FROM session_folder_grants WHERE host = ?1 AND revoked_at IS NULL ORDER BY path",
        )?;
        let rows = stmt.query_map(params![host], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    })
}

/// Every live grant, newest first, for the owner's list.
pub fn list(db: &Database) -> Result<Vec<FolderGrant>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT id, host, path, requested_by, approved_at FROM session_folder_grants \
             WHERE revoked_at IS NULL ORDER BY approved_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(FolderGrant {
                id: r.get(0)?,
                host: r.get(1)?,
                path: r.get(2)?,
                requested_by: r.get(3)?,
                approved_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Revokes a grant. Returns whether it was live.
pub fn revoke(db: &Database, id: i64) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE session_folder_grants SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            params![id, Utc::now().timestamp()],
        )?;
        Ok(n == 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("t.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn a_grant_is_per_host_kept_once_and_gone_when_revoked() {
        let (_d, db) = db();
        grant(&db, "laptop", "/work/app", "claude").unwrap();
        grant(&db, "laptop", "/work/app", "claude").unwrap();
        assert_eq!(folders_for_host(&db, "laptop").unwrap(), vec!["/work/app"]);
        assert!(folders_for_host(&db, "server").unwrap().is_empty());
        let id = list(&db).unwrap()[0].id;
        assert!(revoke(&db, id).unwrap());
        assert!(!revoke(&db, id).unwrap());
        assert!(folders_for_host(&db, "laptop").unwrap().is_empty());
        grant(&db, "laptop", "/work/app", "claude").unwrap();
        assert_eq!(
            list(&db).unwrap().len(),
            1,
            "a revoked folder can be granted again"
        );
    }
}
