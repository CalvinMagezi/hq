//! What shadow mode learns before anything is enforced (docs/plans/task-verification.md,
//! "Calibration": shadow first). In shadow mode HQ completes nothing, so there are no completed
//! tasks to audit yet. What there is: a verdict for each task in review, and then what someone
//! other than its worker did with the task. Comparing the two gives the first accuracy numbers.
//!
//! A verdict is compared with the first thing that happened to its submission afterwards:
//!
//! - the task was completed, by someone who did not work it: the verifier was right to pass it,
//!   and wrong (a false reject) to fail it;
//! - the task was sent back (to `to_do`, `in_progress` or `blocked`), by someone who did not work
//!   it: the verifier was right to fail it, and wrong (a false pass, the dangerous kind) to pass it.
//!
//! A move by the task's own worker or by HQ's verifier says nothing about whether the work was
//! done, so it is not counted.

use super::calibration::wilson_upper;
use super::verification::{LEVEL_OWNER, MAX_TIER, VERDICT_PASS};
use super::*;

/// How a tier's shadow verdicts compare with what independent people and agents did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShadowTier {
    pub tier: u8,
    /// Verdicts whose submission has an independent outcome.
    pub compared: i64,
    pub agreed: i64,
    /// The verifier would have completed work that was then sent back.
    pub false_passes: i64,
    /// The verifier would not have completed work that was then accepted.
    pub false_rejects: i64,
    /// Upper bound (95%) on the share of compared verdicts that were false passes. `None` until
    /// there is something compared.
    pub false_pass_upper: Option<f64>,
}

/// Who worked the submission: whoever held a lease on the task, and whoever moved it into progress
/// (not out of review, which is a decision and not work) or into review, up to that submission.
/// `task_workers` also counts a move out of review into `in_progress`, which would make the
/// person who sent the work back one of its workers.
fn workers_of(
    conn: &Connection,
    task_id: &str,
    submission: i64,
) -> Result<std::collections::HashSet<String>> {
    let mut workers = std::collections::HashSet::new();
    let mut stmt = conn.prepare(
        "SELECT actor FROM task_events \
         WHERE task_id = ?1 AND id <= ?2 AND actor IS NOT NULL \
           AND ((event_type = 'entered_in_progress' AND COALESCE(from_status, '') <> ?3) \
                OR event_type = 'entered_ready_for_review')",
    )?;
    for actor in stmt.query_map(params![task_id, submission, STATUS_READY_FOR_REVIEW], |r| {
        r.get::<_, String>(0)
    })? {
        workers.insert(actor?.trim().to_ascii_lowercase());
    }
    let mut stmt =
        conn.prepare("SELECT DISTINCT actor FROM task_work_sessions WHERE task_id = ?1")?;
    for actor in stmt.query_map(params![task_id], |r| r.get::<_, String>(0))? {
        workers.insert(actor?.trim().to_ascii_lowercase());
    }
    Ok(workers)
}

enum Outcome {
    Accepted,
    SentBack,
}

/// The first thing that happened to a submission after it entered review, if an independent
/// party did it.
fn independent_outcome(
    conn: &Connection,
    task_id: &str,
    submission: i64,
    workers: &std::collections::HashSet<String>,
) -> Result<Option<Outcome>> {
    let first: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT to_status, actor FROM task_events \
             WHERE task_id = ?1 AND id > ?2 AND from_status = ?3 ORDER BY id LIMIT 1",
            params![task_id, submission, STATUS_READY_FOR_REVIEW],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((to, Some(actor))) = first else {
        return Ok(None);
    };
    let actor = actor.to_ascii_lowercase();
    if actor.starts_with("verify:") || workers.contains(&actor) {
        return Ok(None);
    }
    Ok(match to.as_str() {
        STATUS_COMPLETE => Some(Outcome::Accepted),
        STATUS_TO_DO | STATUS_IN_PROGRESS | STATUS_BLOCKED => Some(Outcome::SentBack),
        _ => None,
    })
}

/// Shadow verdicts compared with outcomes, per tier. Only the newest verdict on each submission
/// counts, and verdicts HQ applied itself are left to the audits.
pub fn shadow_comparison(conn: &Connection) -> Result<Vec<ShadowTier>> {
    let mut stmt = conn.prepare(
        "SELECT v.task_id, v.submission, v.tier, v.verdict FROM task_verdicts v \
         WHERE v.canary = 0 AND v.applied = 0 AND v.level <> ?1 AND v.submission IS NOT NULL \
           AND v.rowid = (SELECT MAX(w.rowid) FROM task_verdicts w \
                          WHERE w.task_id = v.task_id AND w.submission = v.submission \
                            AND w.canary = 0 AND w.applied = 0 AND w.level <> ?1)",
    )?;
    let rows: Vec<(String, i64, i64, String)> = stmt
        .query_map(params![LEVEL_OWNER], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut tiers: Vec<ShadowTier> = (0..=MAX_TIER)
        .map(|tier| ShadowTier {
            tier,
            compared: 0,
            agreed: 0,
            false_passes: 0,
            false_rejects: 0,
            false_pass_upper: None,
        })
        .collect();
    for (task_id, submission, tier, verdict) in rows {
        let workers = workers_of(conn, &task_id, submission)?;
        let Some(outcome) = independent_outcome(conn, &task_id, submission, &workers)? else {
            continue;
        };
        let t = &mut tiers[tier.clamp(0, i64::from(MAX_TIER)) as usize];
        t.compared += 1;
        let would_complete = verdict == VERDICT_PASS;
        match (would_complete, outcome) {
            (true, Outcome::Accepted) | (false, Outcome::SentBack) => t.agreed += 1,
            (true, Outcome::SentBack) => t.false_passes += 1,
            (false, Outcome::Accepted) => t.false_rejects += 1,
        }
    }
    for t in &mut tiers {
        t.false_pass_upper = (t.compared > 0).then(|| wilson_upper(t.false_passes, t.compared));
    }
    Ok(tiers)
}
