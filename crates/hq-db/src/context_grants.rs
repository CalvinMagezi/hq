//! What a running session may read through the context gateway: which task it works on
//! (from its open work lease, never from arguments) and which vault folders that task opens.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// A harness session as the gateway needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub running: bool,
    pub mission_id: Option<String>,
}

pub fn session(conn: &Connection, session_id: &str) -> Result<Option<SessionRow>> {
    Ok(conn
        .query_row(
            "SELECT status = 'running', mission_id FROM harness_sessions WHERE id = ?1",
            params![session_id],
            |row| {
                Ok(SessionRow {
                    running: row.get(0)?,
                    mission_id: row.get(1)?,
                })
            },
        )
        .optional()?)
}

/// The task this session holds an open work lease on. A spawned session has at most one.
pub fn open_task_for_session(conn: &Connection, session_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT task_id FROM task_work_sessions
             WHERE harness_session_id = ?1 AND ended_at IS NULL",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?)
}

/// The folders a task opens to its sessions, or `None` when the task was never marked.
pub fn allow_prefixes(conn: &Connection, task_id: &str) -> Result<Option<Vec<String>>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT allow_prefixes FROM task_context_scope WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|json| Ok(serde_json::from_str(&json)?)).transpose()
}

pub fn set_allow_prefixes(conn: &Connection, task_id: &str, prefixes: &[String]) -> Result<()> {
    conn.execute(
        "INSERT INTO task_context_scope (task_id, allow_prefixes) VALUES (?1, ?2)
         ON CONFLICT(task_id) DO UPDATE SET allow_prefixes = excluded.allow_prefixes,
                                            updated_at = datetime('now')",
        params![task_id, serde_json::to_string(prefixes)?],
    )?;
    Ok(())
}
