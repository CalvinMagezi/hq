//! Who a task in review is waiting on (docs/plans/task-verification.md, "Weekend bursts").
//!
//! Every task that is not done carries a `waiting_for`, and only `owner` counts against the
//! owner. A verdict that is not a pass names it on the verdict. A task that could not be decided
//! yet (checks still running, no reviewer reachable, the ceiling spent) has no verdict, so a
//! wait row says what it is waiting on and when to look again.

use super::verification::{review_marker, task_tier, verification_policy};
use super::*;
use hq_core::config::VerificationMode;

/// Everyone a waiting task can be waiting on, in the order the plan lists them.
pub const WAITING_FOR: [&str; 5] = ["worker", "ci", "verifier", "budget", "owner"];
pub const WAIT_WORKER: &str = "worker";
pub const WAIT_CI: &str = "ci";
pub const WAIT_VERIFIER: &str = "verifier";
pub const WAIT_BUDGET: &str = "budget";
pub const WAIT_OWNER: &str = "owner";

/// Longest reason kept.
const MAX_REASON_CHARS: usize = 500;

/// A task that was looked at and could not be decided yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Wait {
    pub task_id: String,
    pub submission: Option<i64>,
    pub waiting_for: String,
    pub reason: String,
    /// Looks at this submission that ended in a wait.
    pub tries: i64,
    /// Unix seconds before which it is not looked at again.
    pub retry_at: i64,
}

fn row_to_wait(r: &rusqlite::Row) -> rusqlite::Result<Wait> {
    Ok(Wait {
        task_id: r.get(0)?,
        submission: r.get(1)?,
        waiting_for: r.get(2)?,
        reason: r.get(3)?,
        tries: r.get(4)?,
        retry_at: r.get(5)?,
    })
}

const WAIT_COLS: &str = "task_id, submission, waiting_for, reason, tries, retry_at";

/// Records that the task's current submission could not be decided and should be looked at again
/// `retry_after_secs` after `now`. Looks at the same submission count up; a new submission starts
/// over.
pub fn record_wait(
    conn: &Connection,
    task_id: &str,
    waiting_for: &str,
    reason: &str,
    retry_after_secs: u64,
    now: i64,
) -> Result<Wait> {
    if !WAITING_FOR.contains(&waiting_for) {
        anyhow::bail!("waiting_for must be one of {}", WAITING_FOR.join(", "));
    }
    in_write_tx(conn, |conn| {
        let marker = review_marker(conn, task_id)?;
        let reason: String = reason.chars().take(MAX_REASON_CHARS).collect();
        let retry_at = now.saturating_add(i64::try_from(retry_after_secs).unwrap_or(i64::MAX));
        conn.execute(
            "INSERT INTO task_verification_waits \
             (task_id, submission, waiting_for, reason, tries, retry_at) \
             VALUES (?1, ?2, ?3, ?4, 1, ?5) \
             ON CONFLICT(task_id) DO UPDATE SET \
               tries = CASE WHEN task_verification_waits.submission IS excluded.submission \
                            THEN task_verification_waits.tries + 1 ELSE 1 END, \
               submission = excluded.submission, waiting_for = excluded.waiting_for, \
               reason = excluded.reason, retry_at = excluded.retry_at, \
               updated_at = datetime('now')",
            params![task_id, marker, waiting_for, reason, retry_at],
        )?;
        Ok(conn.query_row(
            &format!("SELECT {WAIT_COLS} FROM task_verification_waits WHERE task_id = ?1"),
            params![task_id],
            row_to_wait,
        )?)
    })
}

/// Forgets the wait on a task, once it has a verdict or has moved on.
pub fn clear_wait(conn: &Connection, task_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM task_verification_waits WHERE task_id = ?1",
        params![task_id],
    )?;
    Ok(())
}

/// The wait on a task's current submission, if it has one. A wait recorded for an earlier
/// submission is not a wait for this one.
pub fn current_wait(conn: &Connection, task_id: &str) -> Result<Option<Wait>> {
    let marker = review_marker(conn, task_id)?;
    Ok(conn
        .query_row(
            &format!(
                "SELECT {WAIT_COLS} FROM task_verification_waits \
                 WHERE task_id = ?1 AND submission IS ?2"
            ),
            params![task_id, marker],
            row_to_wait,
        )
        .optional()?)
}

/// Who a task is waiting on and why, when the answer is not simply "its worker, working".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WaitingState {
    pub who: String,
    pub reason: String,
}

impl WaitingState {
    fn new(who: &str, reason: impl Into<String>) -> Option<Self> {
        Some(Self {
            who: who.to_string(),
            reason: reason.into(),
        })
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

/// Who `task` waits on under the policy in force. A task in review waits on HQ's verifier when
/// its tier is verified, and on the owner when it is not; a task a verification sent back waits
/// on its worker; a task blocked by verification waits on the owner. `None` for any other task.
pub fn waiting_state(conn: &Connection, task: &Task) -> Result<Option<WaitingState>> {
    let cfg = verification_policy(conn)?;
    match task.status.as_str() {
        STATUS_READY_FOR_REVIEW => {
            let tier = task_tier(conn, task, &cfg)?;
            if !(cfg.mode == VerificationMode::Enforce && cfg.enforces(tier)) {
                return Ok(WaitingState::new(
                    WAIT_OWNER,
                    "waiting for you to review it",
                ));
            }
            if let Some(hold) = get_hold(conn, &task.id)? {
                return Ok(WaitingState::new(
                    WAIT_OWNER,
                    format!(
                        "tier 2: it completes at {} UTC unless you veto it",
                        chrono::DateTime::from_timestamp(hold.release_at, 0)
                            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_default()
                    ),
                ));
            }
            if let Some(wait) = current_wait(conn, &task.id)? {
                return Ok(WaitingState::new(&wait.waiting_for, wait.reason));
            }
            let marker = review_marker(conn, &task.id)?;
            let latest: Option<(Option<String>, String)> = conn
                .query_row(
                    "SELECT waiting_for, reasons FROM task_verdicts \
                     WHERE task_id = ?1 AND canary = 0 AND submission IS ?2 \
                     ORDER BY created_at DESC, rowid DESC LIMIT 1",
                    params![task.id, marker],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            Ok(match latest {
                Some((Some(who), reasons)) => WaitingState::new(&who, first_line(&reasons)),
                Some((None, _)) => None,
                None => WaitingState::new(WAIT_VERIFIER, "queued for verification"),
            })
        }
        STATUS_TO_DO | STATUS_BLOCKED => {
            let marker = review_marker(conn, &task.id)?;
            let latest: Option<(Option<String>, String)> = conn
                .query_row(
                    "SELECT waiting_for, reasons FROM task_verdicts \
                     WHERE task_id = ?1 AND canary = 0 AND submission IS ?2 AND verdict <> 'pass' \
                     ORDER BY created_at DESC, rowid DESC LIMIT 1",
                    params![task.id, marker],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let owner_blocked = task.status == STATUS_BLOCKED
                && current_wait(conn, &task.id)?.is_some_and(|w| w.waiting_for == WAIT_OWNER);
            Ok(match latest {
                _ if owner_blocked => {
                    WaitingState::new(WAIT_OWNER, task.blocked_reason.clone().unwrap_or_default())
                }
                Some((Some(who), reasons)) if task.status == STATUS_TO_DO => {
                    WaitingState::new(&who, first_line(&reasons))
                }
                _ => None,
            })
        }
        _ => Ok(None),
    }
}

/// How many tasks wait on each party right now: every task in review, and every task a
/// verification touched that is still open. Only `owner` counts against the owner.
pub fn waiting_summary(conn: &Connection) -> Result<std::collections::BTreeMap<String, i64>> {
    let mut stmt = conn.prepare(
        "SELECT t.id FROM tasks t WHERE t.archived_at IS NULL AND ( \
             t.status = 'ready_for_review' \
          OR (t.status IN ('to_do', 'blocked') \
              AND EXISTS (SELECT 1 FROM task_verdicts v WHERE v.task_id = t.id AND v.canary = 0)))",
    )?;
    let ids: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = std::collections::BTreeMap::new();
    for id in ids {
        let Some(task) = get_task(conn, &id)? else {
            continue;
        };
        if let Some(state) = waiting_state(conn, &task)? {
            *out.entry(state.who).or_insert(0) += 1;
        }
    }
    Ok(out)
}
