//! Database operations for per-tool usage telemetry.
//!
//! Called from the PostToolUse hook to record every tool invocation. The weekly
//! failure digest (`crate::tool_failures`) reads what is recorded here.

use anyhow::Result;
use rusqlite::{Connection, params};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[test]
    fn records_successes_and_failures() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            record_tool_call(conn, "tool-a", "agent-1", "sess-1", true, None)?;
            record_tool_call(conn, "tool-a", "agent-2", "sess-2", false, Some("timeout"))?;
            let (total, failed, error): (i64, i64, Option<String>) = conn.query_row(
                "SELECT COUNT(*), SUM(success = 0), MAX(error_msg) FROM tool_usage
                  WHERE tool_name = 'tool-a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!((total, failed), (2, 1));
            assert_eq!(error.as_deref(), Some("timeout"));
            Ok(())
        })
    }
}
