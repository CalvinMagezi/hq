//! The context gateway's audit trail: counts only, never what was asked or returned.

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, params};

pub struct AuditRow<'a> {
    pub session_id: &'a str,
    pub task_id: &'a str,
    pub mission_id: Option<&'a str>,
    pub op: &'a str,
    pub returned: usize,
    pub denied: usize,
}

pub fn record(conn: &Connection, row: &AuditRow<'_>, now: DateTime<Utc>) -> Result<()> {
    conn.execute(
        "INSERT INTO context_gateway_audit
         (session_id, task_id, mission_id, op, returned, denied, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            row.session_id,
            row.task_id,
            row.mission_id,
            row.op,
            row.returned as i64,
            row.denied as i64,
            now.to_rfc3339_opts(SecondsFormat::Secs, true),
        ],
    )?;
    Ok(())
}

/// `(op, returned, denied)` for a task, oldest first.
pub fn counts_for_task(conn: &Connection, task_id: &str) -> Result<Vec<(String, usize, usize)>> {
    let mut stmt = conn.prepare(
        "SELECT op, returned, denied FROM context_gateway_audit WHERE task_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([task_id], |r| {
        Ok((
            r.get(0)?,
            r.get::<_, i64>(1)? as usize,
            r.get::<_, i64>(2)? as usize,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
