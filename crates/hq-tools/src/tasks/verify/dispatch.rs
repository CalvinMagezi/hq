//! The loop behind `verify-dispatch`: take the tasks waiting for review in priority order and
//! verify them a few at a time (docs/plans/task-verification.md, "Verifier").
//!
//! One pass is bounded: at most [`MAX_PER_PASS`] tasks, [`Env::cfg`]'s concurrency at once. A task
//! that cannot be decided yet (checks running, the reviewer's ceiling reached) is remembered with
//! a retry time, so a long queue of such tasks does not take every pass. The retry time is held in
//! memory and is keyed by the submission, so a restart only costs one extra look and a resubmitted
//! task is looked at again at once.

use super::pipeline::{Env, Finished, verify_task};
use super::reviewer::Reviewer;
use anyhow::Result;
use futures::future::join_all;
use hq_core::config::VerificationMode;
use hq_db::tasks as t;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Tasks looked at in one pass, so one pass ends in bounded time however long the queue is.
pub const MAX_PER_PASS: usize = 40;

/// When tasks were last deferred, and until when to leave them alone.
#[derive(Default)]
pub struct Backoff {
    until: Mutex<HashMap<String, Instant>>,
}

impl Backoff {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(c: &t::Candidate) -> String {
        format!("{}@{}", c.task.id, c.marker.unwrap_or_default())
    }

    fn waiting(&self, c: &t::Candidate, now: Instant) -> bool {
        let map = self.until.lock().unwrap_or_else(|p| p.into_inner());
        map.get(&Self::key(c)).is_some_and(|until| *until > now)
    }

    fn defer(&self, c: &t::Candidate, secs: u64, now: Instant) {
        let mut map = self.until.lock().unwrap_or_else(|p| p.into_inner());
        map.insert(Self::key(c), now + Duration::from_secs(secs));
    }

    /// Forgets everything that is no longer in the queue, so the map cannot grow without bound.
    fn retain_only(&self, queue: &[t::Candidate]) {
        let live: std::collections::HashSet<String> = queue.iter().map(Self::key).collect();
        let mut map = self.until.lock().unwrap_or_else(|p| p.into_inner());
        map.retain(|k, _| live.contains(k));
    }

    pub fn len(&self) -> usize {
        self.until.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What one pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DispatchReport {
    /// Tasks waiting for a verdict when the pass began.
    pub queued: usize,
    /// Of those, left alone this pass because they are in a retry wait.
    pub backed_off: usize,
    /// Verifications started.
    pub attempted: usize,
    /// Verdicts recorded.
    pub recorded: usize,
    /// Tasks a pass completed.
    pub completed: usize,
    /// Verifications that could not be decided and will be tried again.
    pub deferred: usize,
    /// Verdicts dropped because the work changed while it was checked.
    pub voided: usize,
    /// Verifications that failed to run at all.
    pub errors: Vec<String>,
}

/// Runs one pass. Does nothing when verification is off.
pub async fn dispatch(
    env: &Env<'_>,
    reviewer: &dyn Reviewer,
    backoff: &Backoff,
) -> Result<DispatchReport> {
    let mut report = DispatchReport::default();
    if env.cfg.mode == VerificationMode::Off {
        return Ok(report);
    }
    let cfg = env.cfg.clone();
    let queue = env
        .db
        .with_conn(move |c| t::verification_candidates(c, &cfg))?;
    backoff.retain_only(&queue);
    report.queued = queue.len();

    let now = Instant::now();
    let mut ready: Vec<&t::Candidate> = Vec::new();
    for c in &queue {
        if backoff.waiting(c, now) {
            report.backed_off += 1;
        } else if ready.len() < MAX_PER_PASS {
            ready.push(c);
        }
    }

    for batch in ready.chunks(env.cfg.parallelism()) {
        let allow_review = review_allowed(env)?;
        let runs = batch
            .iter()
            .map(|c| async move { (*c, verify_task(env, reviewer, &c.task, allow_review).await) });
        for (c, result) in join_all(runs).await {
            report.attempted += 1;
            match result {
                Ok(Finished::Recorded { completed, .. }) => {
                    report.recorded += 1;
                    if completed {
                        report.completed += 1;
                    }
                }
                Ok(Finished::Deferred {
                    retry_after_secs, ..
                }) => {
                    report.deferred += 1;
                    backoff.defer(c, retry_after_secs, Instant::now());
                }
                Ok(Finished::Voided { reason }) => {
                    report.voided += 1;
                    tracing::info!(task = %c.task.display_id, %reason, "verify-dispatch: verdict voided");
                }
                Err(e) => {
                    // A task that cannot be read is retried after a while, not every minute.
                    backoff.defer(c, ERROR_RETRY_SECS, Instant::now());
                    report.errors.push(format!("{}: {e:#}", c.task.display_id));
                }
            }
        }
    }
    Ok(report)
}

/// How long to leave a task whose verification failed to run.
const ERROR_RETRY_SECS: u64 = 300;

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
