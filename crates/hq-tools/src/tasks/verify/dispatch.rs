//! The loop behind `verify-dispatch`: take the tasks waiting for review in priority order and
//! verify them a few at a time (docs/plans/task-verification.md, "Verifier").
//!
//! One pass is bounded: at most [`MAX_PER_PASS`] tasks, [`Env::cfg`]'s concurrency at once. A task
//! that cannot be decided yet (checks running, the reviewer's ceiling reached) has a wait stored
//! in the database with its retry time, so a long queue of such tasks does not take every pass,
//! a restart forgets nothing, and the web server can say what each task is waiting on.

use super::pipeline::{Env, Finished, Moved, verify_task};
use super::reviewer::Reviewer;
use anyhow::Result;
use futures::future::join_all;
use hq_core::config::VerificationMode;
use hq_db::tasks as t;

/// Tasks looked at in one pass, so one pass ends in bounded time however long the queue is.
pub const MAX_PER_PASS: usize = 40;

/// Seconds to leave a task whose verification failed to run at all.
const ERROR_RETRY_SECS: u64 = 300;

/// What one pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DispatchReport {
    /// Tasks due for a verdict when the pass began.
    pub queued: usize,
    /// Verifications started.
    pub attempted: usize,
    /// Verdicts recorded.
    pub recorded: usize,
    /// Tasks a pass completed.
    pub completed: usize,
    /// Tasks sent back to their worker with the findings.
    pub returned: usize,
    /// Tasks that went to the owner as an exception.
    pub escalated: usize,
    /// Verifications that could not be decided yet; their wait is stored.
    pub deferred: usize,
    /// Verdicts dropped because the work changed while it was checked.
    pub voided: usize,
    /// Verifications that failed to run at all.
    pub errors: Vec<String>,
}

/// Runs one pass. Does nothing when verification is off.
pub async fn dispatch(env: &Env<'_>, reviewer: &dyn Reviewer) -> Result<DispatchReport> {
    let mut report = DispatchReport::default();
    if env.cfg.mode == VerificationMode::Off {
        return Ok(report);
    }
    let cfg = env.cfg.clone();
    let now = chrono::Utc::now().timestamp();
    let queue = env
        .db
        .with_conn(move |c| t::verification_candidates(c, &cfg, now))?;
    report.queued = queue.len();

    let ready: Vec<&t::Candidate> = queue.iter().take(MAX_PER_PASS).collect();
    for batch in ready.chunks(env.cfg.parallelism()) {
        let allow_review = review_allowed(env)?;
        let runs = batch
            .iter()
            .map(|c| async move { (*c, verify_task(env, reviewer, &c.task, allow_review).await) });
        for (c, result) in join_all(runs).await {
            report.attempted += 1;
            match result {
                Ok(Finished::Recorded { moved, .. }) => {
                    report.recorded += 1;
                    match moved {
                        Moved::Completed => report.completed += 1,
                        Moved::Returned => report.returned += 1,
                        Moved::Escalated => report.escalated += 1,
                        Moved::Nothing => {}
                    }
                }
                Ok(Finished::Deferred { .. }) => report.deferred += 1,
                Ok(Finished::Voided { reason }) => {
                    report.voided += 1;
                    tracing::info!(task = %c.task.display_id, %reason, "verify-dispatch: verdict voided");
                }
                Err(e) => {
                    report.errors.push(format!("{}: {e:#}", c.task.display_id));
                    // A task that cannot be read is retried after a while, not every minute.
                    let (id, reason) = (c.task.id.clone(), format!("could not be verified: {e:#}"));
                    let stored = env.db.with_conn(move |conn| {
                        t::record_wait(
                            conn,
                            &id,
                            t::WAIT_VERIFIER,
                            &reason,
                            ERROR_RETRY_SECS,
                            chrono::Utc::now().timestamp(),
                        )
                        .map(|_| ())
                    });
                    if let Err(e) = stored {
                        tracing::warn!(error = %e, "verify-dispatch: could not store the wait");
                    }
                }
            }
        }
    }
    Ok(report)
}

/// Whether a reviewer may be called for the next batch. In enforce mode always (the monthly
/// ceiling is what limits it). In shadow mode only until the sample is used, so a try of E2 prices
/// the rest without paying for all of it. A batch can overshoot the sample by less than its size.
fn review_allowed(env: &Env<'_>) -> Result<bool> {
    if env.cfg.mode != VerificationMode::Shadow {
        return Ok(true);
    }
    let recorded = env.db.with_conn(t::reviewer_runs_recorded)?;
    Ok(recorded < i64::from(env.cfg.shadow_e2_sample))
}
