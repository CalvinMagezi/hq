//! Approximate Copilot credits per step: the account-wide counter read around each step.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use hq_core::types::SessionEvent;

use super::AgentSession;

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
        self.emit(SessionEvent::StepCredits {
            turn,
            credits_used_before: before,
            credits_used_after: after,
            delta: credit_delta(before, after),
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
    fn delta_needs_both_readings_and_never_goes_negative() {
        assert_eq!(credit_delta(Some(10.0), Some(12.5)), Some(2.5));
        assert_eq!(credit_delta(Some(10.0), Some(9.0)), Some(0.0));
        assert_eq!(credit_delta(None, Some(9.0)), None);
        assert_eq!(credit_delta(Some(1.0), None), None);
    }
}
