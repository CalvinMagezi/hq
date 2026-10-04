//! Tracks HQ's checkpointed self-update runs: branch, base revision, rollback
//! binary snapshot, and lifecycle status from `open` through `installed` or
//! `rolled_back`.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

pub const STATUS_OPEN: &str = "open";
pub const STATUS_CHECKED: &str = "checked";
/// The owner approved this exact tree and the applier may swap the binary.
pub const STATUS_APPROVED: &str = "approved";
pub const STATUS_INSTALLED: &str = "installed";
pub const STATUS_ROLLED_BACK: &str = "rolled_back";
pub const STATUS_ABANDONED: &str = "abandoned";
pub const STATUS_FAILED: &str = "failed";

#[derive(Debug, Clone, Serialize)]
pub struct SelfUpdateRun {
    pub id: i64,
    pub description: String,
    pub branch: String,
    pub base_rev: String,
    pub prev_binary_path: Option<String>,
    pub status: String,
    pub test_output_tail: Option<String>,
    pub created_at: String,
    pub installed_at: Option<String>,
    /// SHA-256 of the binary the owner approved, with its signature.
    pub approved_binary_sha256: Option<String>,
    pub approval_mac: Option<String>,
}

fn row_to_run(row: &rusqlite::Row) -> rusqlite::Result<SelfUpdateRun> {
    Ok(SelfUpdateRun {
        id: row.get(0)?,
        description: row.get(1)?,
        branch: row.get(2)?,
        base_rev: row.get(3)?,
        prev_binary_path: row.get(4)?,
        status: row.get(5)?,
        test_output_tail: row.get(6)?,
        created_at: row.get(7)?,
        installed_at: row.get(8)?,
        approved_binary_sha256: row.get(9)?,
        approval_mac: row.get(10)?,
    })
}

const COLS: &str = "id, description, branch, base_rev, prev_binary_path, status, test_output_tail, created_at, installed_at, approved_binary_sha256, approval_mac";

pub fn insert(
    conn: &Connection,
    description: &str,
    branch: &str,
    base_rev: &str,
    prev_binary_path: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO self_update_runs (description, branch, base_rev, prev_binary_path)
         VALUES (?1, ?2, ?3, ?4)",
        params![description, branch, base_rev, prev_binary_path],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<SelfUpdateRun>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM self_update_runs WHERE id = ?1"),
            params![id],
            row_to_run,
        )
        .optional()?)
}

/// The single run currently in `open`, `checked` or `approved` state, if any.
/// Only one self-update may be in flight at a time.
pub fn get_active(conn: &Connection) -> Result<Option<SelfUpdateRun>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM self_update_runs
                 WHERE status IN ('open', 'checked', 'approved')
                 ORDER BY id DESC LIMIT 1"
            ),
            [],
            row_to_run,
        )
        .optional()?)
}

pub fn set_status(conn: &Connection, id: i64, status: &str) -> Result<()> {
    let installed_at_sql = if status == STATUS_INSTALLED {
        ", installed_at = datetime('now')"
    } else {
        ""
    };
    conn.execute(
        &format!("UPDATE self_update_runs SET status = ?1{installed_at_sql} WHERE id = ?2"),
        params![status, id],
    )?;
    Ok(())
}

/// Marks the run approved for exactly this binary hash, signed with the install key.
pub fn set_approved(conn: &Connection, id: i64, binary_sha256: &str, mac: &str) -> Result<()> {
    conn.execute(
        "UPDATE self_update_runs SET status = ?1, approved_binary_sha256 = ?2, approval_mac = ?3 WHERE id = ?4",
        params![STATUS_APPROVED, binary_sha256, mac, id],
    )?;
    Ok(())
}

pub fn set_check_output(conn: &Connection, id: i64, tail: &str) -> Result<()> {
    conn.execute(
        "UPDATE self_update_runs SET test_output_tail = ?1 WHERE id = ?2",
        params![tail, id],
    )?;
    Ok(())
}

pub fn list_recent(conn: &Connection, limit: usize) -> Result<Vec<SelfUpdateRun>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM self_update_runs ORDER BY id DESC LIMIT ?1"
    ))?;
    let runs = stmt
        .query_map(params![limit as i64], row_to_run)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::Database;

    #[test]
    fn lifecycle_roundtrip() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            assert!(get_active(c)?.is_none());
            let id = insert(
                c,
                "add feature",
                "self/20260725-add-feature",
                "abc123",
                Some("/tmp/hq-abc123"),
            )?;
            let active = get_active(c)?.unwrap();
            assert_eq!(active.id, id);
            assert_eq!(active.status, STATUS_OPEN);

            set_check_output(c, id, "test result: ok")?;
            set_status(c, id, STATUS_CHECKED)?;
            let run = get(c, id)?.unwrap();
            assert_eq!(run.status, STATUS_CHECKED);
            assert_eq!(run.test_output_tail.as_deref(), Some("test result: ok"));
            assert!(get_active(c)?.is_some());

            set_status(c, id, STATUS_INSTALLED)?;
            let run = get(c, id)?.unwrap();
            assert!(run.installed_at.is_some());
            assert!(get_active(c)?.is_none());
            Ok(())
        })
        .unwrap();
    }
}
