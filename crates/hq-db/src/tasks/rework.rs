//! What happens to a task whose verification found something for its worker to fix
//! (docs/plans/task-verification.md, "Workflow"): it goes back to `to_do` keeping its assignees,
//! with the findings as the checkpoint the next claim resumes from. After `max_attempts` such
//! returns it goes to the owner as one exception instead of being bounced again.

use super::verification::{VERDICT_FAIL, VERDICT_INCONCLUSIVE};
use super::waits::{WAIT_OWNER, WAIT_WORKER, clear_wait, record_wait};
use super::*;

/// What came of sending a task's findings back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reworked {
    /// Back in `to_do` for its worker; this was return number `rejections`.
    Returned { rejections: i64 },
    /// Blocked on the owner: it has been returned `rejections` times without passing.
    Escalated { rejections: i64 },
}

/// Times a task's verification has waited on its worker, counting all submissions: what "reopen
/// count" means.
pub fn rejections(conn: &Connection, task_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM task_verdicts \
         WHERE task_id = ?1 AND canary = 0 AND waiting_for = ?2 AND verdict IN (?3, ?4)",
        params![task_id, WAIT_WORKER, VERDICT_FAIL, VERDICT_INCONCLUSIVE],
        |r| r.get(0),
    )?)
}

/// The lines of `text`, each quoted, so they cannot pass for a line HQ wrote.
fn quoted(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| format!("> {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Moves a task in review whose newest verdict waits on its worker: back to `to_do` with the
/// findings as a checkpoint, or to the owner once it has failed `max_attempts` times. The verdict
/// must already be recorded, because it is what is counted. `actor` is the verifier's identity.
pub fn send_back(
    conn: &Connection,
    task_id: &str,
    actor: &str,
    findings: &str,
    max_attempts: u32,
) -> Result<Reworked> {
    in_write_tx(conn, |conn| {
        let task =
            get_task(conn, task_id)?.ok_or_else(|| anyhow::anyhow!("no task '{task_id}'"))?;
        if task.status != STATUS_READY_FOR_REVIEW {
            anyhow::bail!("{} is not waiting for review", task.display_id);
        }
        let count = rejections(conn, &task.id)?;
        let limit = i64::from(max_attempts.max(1));
        let ctx = WriteCtx {
            actor: Some(actor),
            ..Default::default()
        };
        clear_wait(conn, &task.id)?;
        if count >= limit {
            let reason = format!(
                "Verification found it not done {count} times; it needs a decision from the owner: \
                 rework it differently, change what it should deliver, or complete it by hand."
            );
            let patch = TaskPatch {
                status: Some(STATUS_BLOCKED.to_string()),
                blocked_reason: Some(Some(reason.clone())),
                waiting_on: Some(Some("owner".to_string())),
                ..Default::default()
            };
            update_task_as(conn, &task.id, &patch, Some(STATUS_READY_FOR_REVIEW), &ctx)?;
            add_comment(
                conn,
                &task.id,
                "verifier",
                &format!("{reason}\n{}", quoted(findings)),
                None,
            )?;
            record_wait(conn, &task.id, WAIT_OWNER, &reason, 0, now_secs())?;
            return Ok(Reworked::Escalated { rejections: count });
        }
        let patch = TaskPatch {
            status: Some(STATUS_TO_DO.to_string()),
            ..Default::default()
        };
        update_task_as(conn, &task.id, &patch, Some(STATUS_READY_FOR_REVIEW), &ctx)?;
        let head = format!(
            "Verification could not pass this (return {count} of {limit}); it is back in to_do for \
             {}.",
            if task.assignees.is_empty() {
                "whoever claims it".to_string()
            } else {
                task.assignees.join(", ")
            }
        );
        add_comment(
            conn,
            &task.id,
            "verifier",
            &format!("{head}\n{}", quoted(findings)),
            None,
        )?;
        add_checkpoint(
            conn,
            &task.id,
            None,
            "verifier",
            &Checkpoint {
                summary: format!(
                    "Verification did not pass (return {count} of {limit}): {findings}"
                ),
                next_step: "Fix what is listed, make each criterion demonstrable (link the pull \
                            request or note that shows it), then release the task as \
                            ready_for_review again."
                    .to_string(),
                ..Default::default()
            },
        )?;
        Ok(Reworked::Returned { rejections: count })
    })
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}
