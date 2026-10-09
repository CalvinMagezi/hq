//! Approximate Copilot credits per step: the account-wide counter read around each step.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use hq_core::types::SessionEvent;

use super::AgentSession;

/// Which sessions are in a Copilot run right now. The credit counter is account-wide, so a step's
/// delta is only that step's own when no other session ran at the same time.
pub(super) struct CopilotRuns {
    active: std::sync::Mutex<Vec<String>>,
    /// Bumped whenever any run starts or ends, so a step can tell the picture changed under it.
    generation: std::sync::atomic::AtomicU64,
}

impl CopilotRuns {
    pub(super) const fn new() -> Self {
        Self {
            active: std::sync::Mutex::new(Vec::new()),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, Vec<String>> {
        self.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn started(&self, session: &str) -> u64 {
        let mut active = self.guard();
        if !active.iter().any(|s| s == session) {
            active.push(session.to_string());
        }
        drop(active);
        self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    pub(super) fn ended(&self, session: &str) {
        let mut active = self.guard();
        let before = active.len();
        active.retain(|s| s != session);
        let removed = active.len() != before;
        drop(active);
        if removed {
            self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether a step that began at generation `since` ran with no other Copilot session in play.
    pub(super) fn step_was_alone(&self, since: u64) -> bool {
        self.generation() == since && self.guard().len() <= 1
    }
}

pub(super) static COPILOT_RUNS: CopilotRuns = CopilotRuns::new();

/// Backend name of the in-process Copilot provider.
pub(super) const COPILOT_BACKEND: &str = "copilot";
const READ_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) type CreditReader =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<f64>> + Send>> + Send + Sync>;

/// The step before this one ended at this reading.
enum Baseline {
    Pending(JoinHandle<Option<f64>>),
    Known(Option<f64>),
}

pub(super) struct CreditTracker {
    reader: CreditReader,
    baseline: Baseline,
    /// [`CopilotRuns::generation`] when the step in progress began.
    generation: u64,
}

pub(super) struct StepUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub model: String,
}

impl CreditTracker {
    pub(super) fn with_reader(reader: CreditReader) -> Self {
        Self {
            reader,
            baseline: Baseline::Known(None),
            generation: 0,
        }
    }

    pub(super) fn live() -> Self {
        Self::with_reader(Arc::new(|| {
            Box::pin(async {
                let quota = hq_llm::copilot_usage::fetch_quota().await.ok()?;
                quota.is_metered().then_some(quota.credits_used)
            })
        }))
    }

    /// Bounded, so a slow or failing endpoint costs the step nothing.
    fn spawn_read(&self) -> JoinHandle<Option<f64>> {
        let read = (self.reader)();
        tokio::spawn(async move {
            tokio::time::timeout(READ_TIMEOUT, read)
                .await
                .ok()
                .flatten()
        })
    }
}

/// Credits between two readings; None unless both exist, never negative.
pub(super) fn credit_delta(before: Option<f64>, after: Option<f64>) -> Option<f64> {
    Some((after? - before?).max(0.0))
}

impl AgentSession {
    /// At the start of a run: read the baseline while the first step runs.
    pub(super) fn begin_credit_baseline(&mut self) {
        let copilot = self.backend.name() == COPILOT_BACKEND;
        let Some(tracker) = self.step_credits.as_mut() else {
            return;
        };
        tracker.baseline = if copilot {
            tracker.generation = COPILOT_RUNS.started(&self.session_id);
            Baseline::Pending(tracker.spawn_read())
        } else {
            Baseline::Known(None)
        };
    }

    /// Right after a step ends: start the closing reading, if Copilot served it.
    pub(super) fn begin_credit_read(
        &self,
        active_backend: &str,
    ) -> Option<JoinHandle<Option<f64>>> {
        let tracker = self.step_credits.as_ref()?;
        (active_backend == COPILOT_BACKEND).then(|| tracker.spawn_read())
    }

    /// Emit `StepCredits` for the step that just ended; its closing reading is the next baseline.
    pub(super) async fn emit_step_credits(
        &mut self,
        turn: u32,
        usage: StepUsage,
        after: Option<JoinHandle<Option<f64>>>,
    ) {
        let Some(after) = after else { return };
        let Some(tracker) = self.step_credits.as_mut() else {
            return;
        };
        let after = after.await.ok().flatten();
        let before = match std::mem::replace(&mut tracker.baseline, Baseline::Known(after)) {
            Baseline::Known(v) => v,
            Baseline::Pending(handle) => handle.await.ok().flatten(),
        };
        // The counter is shared by every Copilot session on the seat, so the delta is withheld when
        // another one ran during this step rather than shown as this step's.
        let alone = COPILOT_RUNS.step_was_alone(tracker.generation);
        tracker.generation = COPILOT_RUNS.generation();
        self.emit(SessionEvent::StepCredits {
            turn,
            credits_used_before: before,
            credits_used_after: after,
            delta: credit_delta(before, after).filter(|_| alone),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            model: usage.model,
            approximate: true,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_is_trusted_only_when_no_other_copilot_session_was_in_play() {
        let runs = CopilotRuns::new();
        let mine = runs.started("a");
        assert!(runs.step_was_alone(mine));

        // Another session starts during my step: the picture changed and two are active.
        runs.started("b");
        assert!(!runs.step_was_alone(mine));

        // Even after it ends, the step that overlapped it stays untrusted.
        runs.ended("b");
        assert!(!runs.step_was_alone(mine));
        // The next step, measured from now, is mine alone again.
        assert!(runs.step_was_alone(runs.generation()));
    }

    #[test]
    fn two_sessions_that_stay_active_never_trust_each_other() {
        let runs = CopilotRuns::new();
        runs.started("a");
        runs.started("b");
        assert!(!runs.step_was_alone(runs.generation()));
        runs.ended("a");
        assert!(runs.step_was_alone(runs.generation()));
    }

    #[test]
    fn delta_needs_both_readings_and_never_goes_negative() {
        assert_eq!(credit_delta(Some(10.0), Some(12.5)), Some(2.5));
        assert_eq!(credit_delta(Some(10.0), Some(9.0)), Some(0.0));
        assert_eq!(credit_delta(None, Some(9.0)), None);
        assert_eq!(credit_delta(Some(1.0), None), None);
    }
}
