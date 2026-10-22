//! A second opinion on one task: the owner asks for a fresh verifier to judge it again, ideally a
//! different model, and a disagreement with the first verdict counts as an audit datum against it
//! (docs/plans/task-verification.md, "Verify and audit are different jobs"). The request is a row
//! the daemon answers, so a web request never waits for a model.

use super::*;

pub const REVERIFY_PENDING: &str = "pending";
pub const REVERIFY_AGREED: &str = "agreed";
pub const REVERIFY_DISAGREED: &str = "disagreed";
pub const REVERIFY_FAILED: &str = "failed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReverifyRequest {
    pub task_id: String,
    pub display_id: String,
    pub requested_at: String,
    pub state: String,
    pub detail: String,
    pub answered_at: Option<String>,
}

fn row_to_request(r: &rusqlite::Row) -> rusqlite::Result<ReverifyRequest> {
    Ok(ReverifyRequest {
        task_id: r.get(0)?,
        display_id: r.get(1)?,
        requested_at: r.get(2)?,
        state: r.get(3)?,
        detail: r.get(4)?,
        answered_at: r.get(5)?,
    })
}

const COLS: &str = "q.task_id, t.display_id, q.requested_at, q.state, q.detail, q.answered_at";

/// Asks for a second opinion on a task. Asking again after it was answered asks again.
pub fn request_reverify(conn: &Connection, task_ref: &str) -> Result<ReverifyRequest> {
    let task = get_task(conn, task_ref)?.ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
    if latest_verdict(conn, &task.id)?.is_none() {
        anyhow::bail!(
            "{} has no verdict yet, so there is nothing to give a second opinion on",
            task.display_id
        );
    }
    conn.execute(
        "INSERT INTO task_reverify_requests (task_id, state) VALUES (?1, 'pending') \
         ON CONFLICT(task_id) DO UPDATE SET state = 'pending', detail = '', \
           requested_at = datetime('now'), answered_at = NULL",
        params![task.id],
    )?;
    get_reverify(conn, &task.id)?.ok_or_else(|| anyhow::anyhow!("the request was lost"))
}

pub fn get_reverify(conn: &Connection, task_id: &str) -> Result<Option<ReverifyRequest>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM task_reverify_requests q JOIN tasks t ON t.id = q.task_id \
                 WHERE q.task_id = ?1"
            ),
            params![task_id],
            row_to_request,
        )
        .optional()?)
}

pub fn pending_reverifies(conn: &Connection, limit: usize) -> Result<Vec<ReverifyRequest>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM task_reverify_requests q JOIN tasks t ON t.id = q.task_id \
         WHERE q.state = 'pending' ORDER BY q.requested_at, q.rowid LIMIT {}",
        limit.clamp(1, 50)
    ))?;
    let rows = stmt
        .query_map([], row_to_request)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Records how a second opinion came out.
pub fn answer_reverify(conn: &Connection, task_id: &str, state: &str, detail: &str) -> Result<()> {
    conn.execute(
        "UPDATE task_reverify_requests SET state = ?2, detail = ?3, answered_at = datetime('now') \
         WHERE task_id = ?1",
        params![task_id, state, detail.chars().take(500).collect::<String>()],
    )?;
    Ok(())
}
