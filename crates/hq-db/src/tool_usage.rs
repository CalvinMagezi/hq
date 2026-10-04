//! Database operations for per-tool usage telemetry.
//!
//! Called from the PostToolUse hook to record every tool invocation. Provides
//! aggregate queries for the model-card UI (call counts, error rates, last-used).

use anyhow::Result;
use rusqlite::{Connection, params};

/// One row in the `tool_usage` table.
#[derive(Debug, Clone)]
pub struct ToolUsageRow {
    pub id: i64,
    pub tool_name: String,
    pub agent_name: String,
    pub session_id: String,
    pub timestamp: i64,
    pub success: bool,
    pub error_msg: Option<String>,
}

/// Record one tool call. Pass `success = false` and an `error_msg` on failure.
pub fn record_tool_call(
    conn: &Connection,
    tool_name: &str,
    agent_name: &str,
    session_id: &str,
    success: bool,
    error_msg: Option<&str>,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT INTO tool_usage (tool_name, agent_name, session_id, timestamp, success, error_msg)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            tool_name,
            agent_name,
            session_id,
            now,
            success as i64,
            error_msg
        ],
    )?;
    Ok(())
}

/// Total call count for a tool across all agents and sessions.
pub fn total_count(conn: &Connection, tool_name: &str) -> Result<u64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tool_usage WHERE tool_name = ?1",
        params![tool_name],
        |row| row.get(0),
    )?;
    Ok(count as u64)
}

/// Per-agent breakdown: call counts sorted descending.
pub fn by_agent(conn: &Connection, tool_name: &str) -> Result<Vec<(String, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT agent_name, COUNT(*) AS cnt
           FROM tool_usage
          WHERE tool_name = ?1
          GROUP BY agent_name
          ORDER BY cnt DESC",
    )?;
    let rows = stmt.query_map(params![tool_name], |row| {
        let count: i64 = row.get(1)?;
        Ok((row.get::<_, String>(0)?, count as u64))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Unix timestamp of the most recent call, or `None` if the tool has never been used.
pub fn last_used(conn: &Connection, tool_name: &str) -> Result<Option<i64>> {
    let ts: Option<i64> = conn.query_row(
        "SELECT MAX(timestamp) FROM tool_usage WHERE tool_name = ?1",
        params![tool_name],
        |row| row.get(0),
    )?;
    Ok(ts)
}

/// Fraction of calls that failed (0.0 to 1.0). Returns 0.0 when there are no calls.
pub fn error_rate(conn: &Connection, tool_name: &str) -> Result<f64> {
    let (total, failures): (u64, u64) = conn.query_row(
        "SELECT COUNT(*), SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END)
           FROM tool_usage
          WHERE tool_name = ?1",
        params![tool_name],
        |row| Ok((row.get(0)?, row.get::<_, Option<u64>>(1)?.unwrap_or(0))),
    )?;
    if total == 0 {
        return Ok(0.0);
    }
    Ok(failures as f64 / total as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn seed(conn: &Connection) -> Result<()> {
        // Two success calls for "tool-a" from "agent-1"
        record_tool_call(conn, "tool-a", "agent-1", "sess-1", true, None)?;
        record_tool_call(conn, "tool-a", "agent-1", "sess-1", true, None)?;
        // One failure for "tool-a" from "agent-2"
        record_tool_call(conn, "tool-a", "agent-2", "sess-2", false, Some("timeout"))?;
        // One success for "tool-b"
        record_tool_call(conn, "tool-b", "agent-1", "sess-1", true, None)?;
        Ok(())
    }

    #[test]
    fn record_and_total_count() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            seed(conn)?;
            assert_eq!(total_count(conn, "tool-a")?, 3);
            assert_eq!(total_count(conn, "tool-b")?, 1);
            assert_eq!(total_count(conn, "tool-unknown")?, 0);
            Ok(())
        })
    }

    #[test]
    fn by_agent_sorted_desc() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            seed(conn)?;
            let breakdown = by_agent(conn, "tool-a")?;
            assert_eq!(breakdown.len(), 2);
            // agent-1 has 2 calls, agent-2 has 1 — highest first
            assert_eq!(breakdown[0].0, "agent-1");
            assert_eq!(breakdown[0].1, 2);
            assert_eq!(breakdown[1].0, "agent-2");
            assert_eq!(breakdown[1].1, 1);
            Ok(())
        })
    }

    #[test]
    fn error_rate_calculation() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            seed(conn)?;
            // tool-a: 1 failure out of 3 total
            let rate = error_rate(conn, "tool-a")?;
            assert!((rate - 1.0 / 3.0).abs() < 1e-9, "rate was {rate}");
            // tool-b: 0 failures
            let rate_b = error_rate(conn, "tool-b")?;
            assert_eq!(rate_b, 0.0);
            // unknown tool: 0.0 (no division by zero)
            let rate_u = error_rate(conn, "never-called")?;
            assert_eq!(rate_u, 0.0);
            Ok(())
        })
    }

    #[test]
    fn last_used_some_and_none() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            seed(conn)?;
            assert!(last_used(conn, "tool-a")?.is_some());
            assert!(last_used(conn, "never-called")?.is_none());
            Ok(())
        })
    }

}
