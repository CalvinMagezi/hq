// ─── Session Types ──────────────────────────────────────────────

/// Session event emitted during agent execution.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    TextDelta(String),
    /// Reasoning/thinking delta streamed before the final answer.
    Reasoning(String),
    TextDone(String),
    ToolStart {
        tool_name: String,
        tool_call_id: String,
        /// The call's arguments as the model sent them. Unredacted: a surface
        /// that shows or stores them must scrub secrets first.
        arguments: serde_json::Value,
    },
    /// Mid-execution progress update from a long-running tool.
    ///
    /// Distilled from claude-code's `Tool.onProgress` callback pattern.
    /// Tools emit this to report incremental status without blocking the session loop.
    ToolProgress {
        tool_name: String,
        tool_call_id: String,
        /// Short human-readable status line (e.g., "read 1024/4096 bytes").
        message: String,
    },
    ToolEnd {
        tool_name: String,
        tool_call_id: String,
        result: String,
    },
    TurnEnd {
        turn: u32,
    },
    Error(String),
    /// Approximate Copilot credits one step used: the account-wide counter read
    /// before and after the step. Only emitted while a Copilot backend serves the session.
    StepCredits {
        turn: u32,
        credits_used_before: Option<f64>,
        credits_used_after: Option<f64>,
        /// After minus before, floored at 0. None when either reading failed.
        delta: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
        model: String,
        /// Always true: the counter is shared with other use of the seat and lags.
        approximate: bool,
    },
    Compaction {
        old_messages: usize,
        new_messages: usize,
    },
    PreemptiveSummaryReady,
    /// LLM API call is being retried after a transient error.
    RetryAttempt {
        attempt: u32,
        max_retries: u32,
        delay_ms: u64,
        error: String,
    },
    /// Context overflow detected; compaction triggered automatically.
    ContextOverflowRecovery,
    /// Session entered Plan mode. Only read-only tools + plan file writes allowed.
    PlanModeEntered {
        plan_file: String,
    },
    /// Session exited Plan mode. Plan is ready for approval.
    PlanModeExited {
        plan_file: String,
    },
    /// A subagent completed and returned results to the parent session.
    SubagentCompleted {
        agent_type: String,
        harness: String,
        result_preview: String,
    },
    /// Session stopped because it exceeded its USD budget.
    BudgetExhausted {
        spent: f64,
        budget: f64,
    },
    /// Cumulative cost snapshot emitted after each LLM call.
    CostUpdate {
        total_usd: f64,
        input_tokens: u64,
        output_tokens: u64,
        model: String,
        /// True when a [`ProviderChain`](crate) fell back off the declared
        /// primary backend to produce this turn's output — i.e. `model` is
        /// not what the primary was configured to run.
        is_fallback: bool,
    },
}

/// Structured result from a completed session.
///
/// Replaces raw `Result<String>` so callers can distinguish between
/// clean completions, budget/time limits, and API failures.
#[derive(Debug, Clone)]
pub enum SessionResult {
    /// The model produced a final response with no outstanding tool calls.
    Complete(String),
    /// Cumulative cost exceeded `max_budget_usd`.
    BudgetExhausted(String),
    /// Wall-clock duration exceeded `max_duration_secs`.
    TimeLimitReached(String),
    /// Caller set the cancel flag; session stopped cleanly after current tool.
    Cancelled(String),
    /// The backend errored *after* committing partial output. This is **not** a
    /// clean completion: `partial` holds whatever text was streamed before the
    /// failure and `error` describes the failure. Surfacing it as a distinct
    /// variant (rather than `Complete`) keeps callers from treating a truncated,
    /// failed turn as a success while still preserving the partial text.
    Failed {
        /// Text committed before the error (may be empty).
        partial: String,
        /// Human-readable description of the terminal error.
        error: String,
    },
}

impl SessionResult {
    /// Extract the final text regardless of how the session ended.
    pub fn text(&self) -> &str {
        match self {
            Self::Complete(t)
            | Self::BudgetExhausted(t)
            | Self::TimeLimitReached(t)
            | Self::Cancelled(t) => t,
            Self::Failed { partial, .. } => partial,
        }
    }

    /// Whether the session completed normally.
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete(_))
    }

    /// Whether the session ended in a terminal error after partial output.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    /// The terminal error description when this is a [`Failed`](Self::Failed)
    /// outcome, else `None`. Pairs with [`text`](Self::text) (the preserved
    /// partial output) so callers can log or display the partial while still
    /// recognizing that the turn failed.
    pub fn failure_reason(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }

    /// Stable, lowercase label for the terminal outcome. Used to tag the
    /// [`EnvelopeKind::RunFinished`] correlation event.
    pub fn outcome_label(&self) -> &'static str {
        match self {
            Self::Complete(_) => "complete",
            Self::BudgetExhausted(_) => "budget_exhausted",
            Self::TimeLimitReached(_) => "time_limit",
            Self::Cancelled(_) => "cancelled",
            Self::Failed { .. } => "failed",
        }
    }
}

// ─── Correlated Event Envelope ──────────────────────────────────
//
// The session engine emits [`SessionEvent`]s to legacy `on_event` subscribers
// unchanged. It *also* wraps every event in a [`SessionEventEnvelope`] for
// subscribers that need correlation — a run identity, an ordering sequence, and
// the originating source. This is the seam the upcoming agent service builds on
// to interleave a root run with its children; this crate only defines the
// envelope shape (run/child lifecycle variants included), it does not implement
// child orchestration.

/// Where a correlated event originated.
///
/// Cloneable and comparable so consumers can filter a merged stream by source
/// (e.g. "only the CLI harness backend", "only child run X").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventSource {
    /// The session engine itself: limits, compaction, lifecycle bookkeeping.
    Session,
    /// A named execution backend (an API provider or a CLI harness).
    Backend(String),
    /// A child run (sub-agent), identified by its run id. Reserved for the
    /// agent service — the session engine does not emit this today.
    Child(String),
}

/// The payload carried by a [`SessionEventEnvelope`].
///
/// Most envelopes wrap a [`SessionEvent`] via [`EnvelopeKind::Session`]. The
/// run/child lifecycle variants exist so the agent service (a later task) can
/// correlate a root run with spawned children over the same envelope channel
/// without another type; the session engine currently emits only
/// [`Session`](Self::Session), [`RunStarted`](Self::RunStarted), and
/// [`RunFinished`](Self::RunFinished).
#[derive(Debug, Clone)]
pub enum EnvelopeKind {
    /// A standard session event.
    Session(SessionEvent),
    /// A run began (root or, later, a child).
    RunStarted {
        /// The model the run was started with.
        model: String,
    },
    /// A run finished, tagged with [`SessionResult::outcome_label`] (or
    /// `"error"` when the run returned `Err`).
    RunFinished {
        /// Terminal outcome label.
        outcome: String,
    },
    /// A child run was spawned. Reserved for the agent service.
    ChildStarted {
        /// The child's run id (matches its envelopes' `run_id`).
        child_run_id: String,
        /// Human-readable label (e.g. role or task summary).
        label: String,
    },
    /// A child run finished. Reserved for the agent service.
    ChildFinished {
        /// The child's run id.
        child_run_id: String,
        /// Terminal outcome label.
        outcome: String,
    },
}

/// A correlated, ordered envelope around a [`SessionEvent`] or run/child
/// lifecycle marker.
///
/// Cloneable rather than serializable: [`SessionEvent`] is not `Serialize`, and
/// the requirement is "serializable *or* cloneable". Consumers that need wire
/// formats project the fields they need.
#[derive(Debug, Clone)]
pub struct SessionEventEnvelope {
    /// Identifier of the run that produced this event.
    pub run_id: String,
    /// Parent run id, when this run was spawned by another (child runs).
    pub parent_run_id: Option<String>,
    /// Monotonic per-session sequence number for deterministic ordering.
    pub seq: u64,
    /// Where the event originated.
    pub source: EventSource,
    /// The event payload.
    pub kind: EnvelopeKind,
}

impl SessionEventEnvelope {
    /// Wrap a plain [`SessionEvent`] with correlation metadata.
    pub fn session(
        run_id: impl Into<String>,
        parent_run_id: Option<String>,
        seq: u64,
        source: EventSource,
        event: SessionEvent,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            parent_run_id,
            seq,
            source,
            kind: EnvelopeKind::Session(event),
        }
    }

    /// Build a lifecycle envelope (run/child markers).
    pub fn lifecycle(
        run_id: impl Into<String>,
        parent_run_id: Option<String>,
        seq: u64,
        source: EventSource,
        kind: EnvelopeKind,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            parent_run_id,
            seq,
            source,
            kind,
        }
    }

    /// The wrapped [`SessionEvent`], when this envelope carries one.
    pub fn as_session_event(&self) -> Option<&SessionEvent> {
        match &self.kind {
            EnvelopeKind::Session(event) => Some(event),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_result_text_covers_all_variants() {
        assert_eq!(SessionResult::Complete("a".into()).text(), "a");
        assert_eq!(SessionResult::BudgetExhausted("c".into()).text(), "c");
        assert_eq!(SessionResult::TimeLimitReached("d".into()).text(), "d");
        assert_eq!(SessionResult::Cancelled("e".into()).text(), "e");
        assert_eq!(
            SessionResult::Failed {
                partial: "f".into(),
                error: "boom".into()
            }
            .text(),
            "f"
        );
    }

    #[test]
    fn failed_result_is_not_complete_and_preserves_partial() {
        let result = SessionResult::Failed {
            partial: "streamed so far".into(),
            error: "stream reset".into(),
        };
        assert!(!result.is_complete());
        assert!(result.is_failed());
        assert_eq!(result.text(), "streamed so far");
        assert_eq!(result.outcome_label(), "failed");
        assert_eq!(result.failure_reason(), Some("stream reset"));
    }

    #[test]
    fn failure_reason_is_none_for_non_failed_outcomes() {
        assert!(
            SessionResult::Complete("x".into())
                .failure_reason()
                .is_none()
        );
        assert!(
            SessionResult::Cancelled("x".into())
                .failure_reason()
                .is_none()
        );
    }

    #[test]
    fn session_result_outcome_labels_are_stable() {
        assert_eq!(
            SessionResult::Complete("x".into()).outcome_label(),
            "complete"
        );
        assert_eq!(
            SessionResult::Cancelled("x".into()).outcome_label(),
            "cancelled"
        );
    }

    #[test]
    fn envelope_wraps_session_event_with_correlation() {
        let env = SessionEventEnvelope::session(
            "run-1",
            Some("parent-0".into()),
            7,
            EventSource::Backend("openrouter".into()),
            SessionEvent::TextDelta("hi".into()),
        );
        assert_eq!(env.run_id, "run-1");
        assert_eq!(env.parent_run_id.as_deref(), Some("parent-0"));
        assert_eq!(env.seq, 7);
        assert_eq!(env.source, EventSource::Backend("openrouter".into()));
        assert!(matches!(
            env.as_session_event(),
            Some(SessionEvent::TextDelta(t)) if t == "hi"
        ));
    }

    #[test]
    fn envelope_lifecycle_variants_carry_no_session_event() {
        let started = SessionEventEnvelope::lifecycle(
            "run-1",
            None,
            0,
            EventSource::Backend("chain".into()),
            EnvelopeKind::RunStarted { model: "m".into() },
        );
        assert!(started.as_session_event().is_none());
        assert!(matches!(started.kind, EnvelopeKind::RunStarted { .. }));

        let finished = SessionEventEnvelope::lifecycle(
            "run-1",
            None,
            9,
            EventSource::Session,
            EnvelopeKind::RunFinished {
                outcome: "complete".into(),
            },
        );
        assert!(matches!(
            finished.kind,
            EnvelopeKind::RunFinished { outcome } if outcome == "complete"
        ));
    }
}
