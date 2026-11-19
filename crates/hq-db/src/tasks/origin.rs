//! Whether a task's text could have come from outside text a session read. Set when a tainted
//! session creates the task or edits its title or description; never cleared, so an injected page
//! cannot write a task and have a later clean session start a wide helper from it.

use anyhow::Result;
use rusqlite::{Connection, params};

pub fn mark_origin_tainted(conn: &Connection, task_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE tasks SET origin_tainted = 1 WHERE id = ?1 OR display_id = ?1",
        params![task_id],
    )?;
    Ok(())
}

/// False for a task that is not found.
pub fn is_origin_tainted(conn: &Connection, task: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT origin_tainted FROM tasks WHERE id = ?1 OR display_id = ?1",
            params![task],
            |r| r.get::<_, i64>(0),
        )
        .map(|v| v != 0)
        .unwrap_or(false))
}
