//! The veto window on tier 2 work (docs/plans/task-verification.md, "Risk tiers"): a verified
//! deploy, migration, payment or deletion does not complete at once. It waits, in the owner's
//! exceptions list, for a window in which one command reverses it, and HQ completes it when the
//! window passes. Dependents are not told it is done until then.

use super::*;
use serde_json::Value;

/// A tier 2 task that passed verification and is inside its window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VetoHold {
    pub task_id: String,
    pub verdict_id: String,
    pub submission: Option<i64>,
    /// Unix seconds.
    pub release_at: i64,
    /// What the verdict was about, to be compared again before completing.
    pub currency: Value,
    pub created_at: String,
}

const HOLD_COLS: &str = "task_id, verdict_id, submission, release_at, currency, created_at";

fn row_to_hold(r: &rusqlite::Row) -> rusqlite::Result<VetoHold> {
    let currency: String = r.get(4)?;
    Ok(VetoHold {
        task_id: r.get(0)?,
        verdict_id: r.get(1)?,
        submission: r.get(2)?,
        release_at: r.get(3)?,
        currency: serde_json::from_str(&currency).unwrap_or(Value::Null),
        created_at: r.get(5)?,
    })
}

/// Opens the window on a task whose pass `verdict_id` approved.
pub fn create_hold(
    conn: &Connection,
    task_id: &str,
    verdict_id: &str,
    release_at: i64,
    currency: &Value,
) -> Result<VetoHold> {
    let marker = review_marker(conn, task_id)?;
    conn.execute(
        "INSERT INTO task_veto_holds (task_id, verdict_id, submission, release_at, currency) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(task_id) DO UPDATE SET verdict_id = excluded.verdict_id, \
           submission = excluded.submission, release_at = excluded.release_at, \
           currency = excluded.currency, created_at = datetime('now')",
        params![
            task_id,
            verdict_id,
            marker,
            release_at,
            serde_json::to_string(currency)?
        ],
    )?;
    get_hold(conn, task_id)?.ok_or_else(|| anyhow::anyhow!("the veto window was not recorded"))
}

/// The window open on a task, if any.
pub fn get_hold(conn: &Connection, task_id: &str) -> Result<Option<VetoHold>> {
    Ok(conn
        .query_row(
            &format!("SELECT {HOLD_COLS} FROM task_veto_holds WHERE task_id = ?1"),
            params![task_id],
            row_to_hold,
        )
        .optional()?)
}

/// Every open window, soonest to close first.
pub fn list_holds(conn: &Connection) -> Result<Vec<VetoHold>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {HOLD_COLS} FROM task_veto_holds ORDER BY release_at, task_id"
    ))?;
    let rows = stmt
        .query_map([], row_to_hold)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Windows that have closed by `now`.
pub fn due_holds(conn: &Connection, now: i64) -> Result<Vec<VetoHold>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {HOLD_COLS} FROM task_veto_holds WHERE release_at <= ?1 ORDER BY release_at, task_id"
    ))?;
    let rows = stmt
        .query_map(params![now], row_to_hold)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Closes a window without completing the task.
pub fn clear_hold(conn: &Connection, task_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM task_veto_holds WHERE task_id = ?1",
        params![task_id],
    )?;
    Ok(())
}

/// The owner reverses a verdict inside its window: the task goes back to `to_do` with the reason
/// as its checkpoint, and the veto counts against the verifier that passed it.
pub fn veto_hold(conn: &Connection, task_ref: &str, note: &str, by: &str) -> Result<()> {
    in_write_tx(conn, |conn| {
        let task =
            get_task(conn, task_ref)?.ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
        let hold = get_hold(conn, &task.id)?
            .ok_or_else(|| anyhow::anyhow!("{} has no veto window open", task.display_id))?;
        if task.status != STATUS_READY_FOR_REVIEW {
            anyhow::bail!(
                "{} is {}, not waiting for the end of its window",
                task.display_id,
                task.status
            );
        }
        let verdict = get_verdict(conn, &hold.verdict_id)?
            .ok_or_else(|| anyhow::anyhow!("the verdict behind the window is gone"))?;
        clear_hold(conn, &task.id)?;
        let note: String = note.chars().take(1000).collect();
        let ctx = WriteCtx {
            actor: Some(by),
            ..Default::default()
        };
        update_task_as(
            conn,
            &task.id,
            &TaskPatch {
                status: Some(STATUS_TO_DO.to_string()),
                ..Default::default()
            },
            Some(STATUS_READY_FOR_REVIEW),
            &ctx,
        )?;
        let body = if note.trim().is_empty() {
            "The owner vetoed this inside its veto window. It is back in to_do.".to_string()
        } else {
            format!(
                "The owner vetoed this inside its veto window, and it is back in to_do:\n> {note}"
            )
        };
        add_comment(conn, &task.id, "audit", &body, None)?;
        add_checkpoint(
            conn,
            &task.id,
            None,
            "owner",
            &Checkpoint {
                summary: format!("The owner vetoed the verified result: {note}"),
                next_step: "Address what the owner objected to, then release the task as \
                            ready_for_review again."
                    .to_string(),
                ..Default::default()
            },
        )?;
        let id = format!("au-{}", uuid::Uuid::new_v4().simple());
        conn.execute(
            "INSERT INTO task_audits (id, task_id, verdict_id, source, tier, verifier_config, \
             answer, note, answered_at, answered_by) \
             VALUES (?1, ?2, ?3, 'veto', ?4, ?5, 'disagree', ?6, datetime('now'), ?7)",
            params![
                id,
                task.id,
                verdict.id,
                verdict.tier,
                verdict.verifier_config,
                note,
                by
            ],
        )?;
        Ok(())
    })
}
