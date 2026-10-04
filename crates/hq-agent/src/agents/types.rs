//! Core request/plan/outcome types for the unified [`AgentService`](super::AgentService).
//!
//! These consolidate the previously separate vocabularies of `SpawnSubagentTool`
//! (single child, role + model) and `CoordinatorSession` (`TaskSpec` +
//! dependency graph). A [`ChildRequest`] is the single child unit; a
//! [`ChildPlan`] wraps one or more of them with an execution [`ChildMode`]; a
//! [`ChildOutcome`] is the structured result the parent reads and synthesizes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hq_core::types::{ChatMessage, SessionEventEnvelope, SubagentType};
use serde::{Deserialize, Serialize};

use super::model_select::{EffectiveModel, OnModelUnavailable};

/// A cloneable sink the service pushes correlated envelopes into. The parent
/// wires this to its own envelope stream so `ChildStarted`/`ChildFinished`
/// markers and forwarded child events interleave with the parent run.
pub type EnvelopeSink = Arc<dyn Fn(SessionEventEnvelope) + Send + Sync>;

/// One child execution unit.
///
/// Field semantics deliberately mirror `TaskSpec`/`SpawnSubagentTool` so the
/// surface-migration todo can translate the old tool schemas one-to-one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildRequest {
    /// Stable identifier, unique within a [`ChildPlan`]. Referenced by
    /// [`depends_on`](Self::depends_on) of other children.
    pub id: String,
    /// The goal/prompt handed to the child.
    pub goal: String,
    /// Extra context folded into the child's system prompt.
    #[serde(default)]
    pub context: Option<String>,
    /// Explicit backend/harness preference. `None` = auto-select by policy;
    /// `Some("hq")` (or any value equal to the in-process label) = in-process
    /// HQ child; any other name resolves against the [`BackendRegistry`].
    #[serde(default)]
    pub backend: Option<String>,
    /// Role/agent type. Reuses `SubagentType` semantics (general/explorer/
    /// planner/verifier/coder) for model routing and tool policy.
    #[serde(default)]
    pub agent_type: SubagentType,
    /// Explicit model override for an in-process child (e.g. a pre-bound
    /// escalation model like `moonshotai/kimi-k2`). Mirrors the legacy
    /// `SpawnSubagentTool` `model` argument: when set and the child resolves to
    /// the in-process executor, it takes precedence over role aliasing.
    /// Ignored for external backends, which choose their own model.
    #[serde(default)]
    pub model: Option<String>,
    /// What to do when `model` is unsupported or unavailable. Default rejects;
    /// `inherit` runs on the parent's model and records the substitution.
    #[serde(default)]
    pub on_model_unavailable: OnModelUnavailable,
    /// Task ids that must complete before this child runs (graph mode).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Optional path allowlist override. Every entry must resolve within the
    /// service's own allowlist or the child is rejected by governance.
    #[serde(default)]
    pub allowed_paths: Option<Vec<PathBuf>>,
    /// Files the child should focus on (advisory, threaded into the prompt).
    #[serde(default)]
    pub files_in_scope: Vec<String>,
    /// Measurable success criteria (advisory, threaded into the prompt).
    #[serde(default)]
    pub success_criteria: Vec<String>,
    /// Per-child USD budget cap. `None` = derive a fraction of the parent's.
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    /// Wall-clock timeout in seconds. `None` = service default.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Vault context this child needs. HQ writes it per subtask; the packet is
    /// retrieved when the child starts, so a queued child never runs on stale
    /// retrieval.
    #[serde(default)]
    pub context_need: Option<hq_memory::context_packet::ContextNeed>,
    /// A packet HQ built earlier. Time-sensitive entries are refreshed at start
    /// when it has aged past the retrieval window.
    #[serde(default)]
    pub context_packet: Option<hq_memory::context_packet::ContextPacket>,
    /// Tools the child must have to do this work. A missing one blocks the
    /// child before dispatch instead of letting it report success without
    /// the deliverable.
    #[serde(default)]
    pub required_tools: Vec<String>,
    /// Native HQ task this child's work belongs to, when there is one.
    #[serde(default)]
    pub task_id: Option<String>,
}

impl ChildRequest {
    /// Minimal constructor used by callers and tests.
    pub fn new(id: impl Into<String>, goal: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            goal: goal.into(),
            context: None,
            backend: None,
            agent_type: SubagentType::General,
            model: None,
            on_model_unavailable: OnModelUnavailable::default(),
            depends_on: Vec::new(),
            allowed_paths: None,
            files_in_scope: Vec::new(),
            success_criteria: Vec::new(),
            max_budget_usd: None,
            timeout_secs: None,
            context_need: None,
            context_packet: None,
            required_tools: Vec::new(),
            task_id: None,
        }
    }

    /// Lowercase role string matching the sub-agent role vocabulary.
    pub fn role(&self) -> &'static str {
        match self.agent_type {
            SubagentType::Coder => "coder",
            SubagentType::Explorer => "explorer",
            SubagentType::Planner => "planner",
            SubagentType::Verifier => "verifier",
            _ => "general",
        }
    }
}

/// How a [`ChildPlan`]'s children are executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildMode {
    /// Exactly one child, run to completion.
    Single,
    /// Independent children, bounded by `max_concurrent`, isolated failures.
    Parallel,
    /// All children race; first success wins, the rest are cancelled.
    Race,
    /// Dependency-aware topological scheduling honoring `depends_on`.
    Graph,
}

/// A resolved execution plan: the mode plus its children and concurrency bound.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildPlan {
    pub mode: ChildMode,
    pub children: Vec<ChildRequest>,
    /// Concurrency bound for [`Parallel`](ChildMode::Parallel) and
    /// [`Graph`](ChildMode::Graph). Ignored by `Single`; `Race` runs all at once.
    pub max_concurrent: usize,
    /// `Some(false)` runs the children detached from the parent turn: the
    /// service returns immediately and each child's completion is delivered
    /// via [`ChildExecContext::completion_sink`] as a [`ChildCompletionEvent`]
    /// instead of being returned inline. `None`/`Some(true)` is the classic
    /// blocking behavior.
    #[serde(default)]
    pub blocking: Option<bool>,
}

impl ChildPlan {
    /// A single-child plan.
    pub fn single(child: ChildRequest) -> Self {
        Self {
            mode: ChildMode::Single,
            children: vec![child],
            max_concurrent: 1,
            blocking: None,
        }
    }

    /// A parallel plan with the given concurrency bound.
    pub fn parallel(children: Vec<ChildRequest>, max_concurrent: usize) -> Self {
        Self {
            mode: ChildMode::Parallel,
            children,
            max_concurrent: max_concurrent.max(1),
            blocking: None,
        }
    }

    /// A race plan (first success wins).
    pub fn race(children: Vec<ChildRequest>) -> Self {
        let n = children.len().max(1);
        Self {
            mode: ChildMode::Race,
            children,
            max_concurrent: n,
            blocking: None,
        }
    }

    /// A dependency-graph plan with the given concurrency bound.
    pub fn graph(children: Vec<ChildRequest>, max_concurrent: usize) -> Self {
        Self {
            mode: ChildMode::Graph,
            children,
            max_concurrent: max_concurrent.max(1),
            blocking: None,
        }
    }

    /// Whether this plan runs inline (`true`, the default) or detached.
    pub fn is_blocking(&self) -> bool {
        self.blocking.unwrap_or(true)
    }
}

/// Terminal status of a child, preserving `SessionResult` distinctions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildStatus {
    /// Ran to a clean completion.
    Completed,
    /// Ran but ended in a terminal error (possibly after partial output).
    Failed,
    /// Exceeded its wall-clock timeout.
    TimedOut,
    /// A dependency failed, so this child never ran.
    Blocked,
    /// Governance denied the request before it ran (bad path/backend combo,
    /// recursion depth). Never silently downgraded.
    Rejected,
    /// Lost a race (a sibling succeeded first) and was cancelled.
    Cancelled,
}

impl ChildStatus {
    /// Whether this is the single clean-success terminal state.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }

    /// Single-character glyph for compact summaries.
    pub fn glyph(&self) -> char {
        match self {
            Self::Completed => '+',
            Self::Failed => 'x',
            Self::TimedOut => '!',
            Self::Blocked => '-',
            Self::Rejected => '/',
            Self::Cancelled => '~',
        }
    }
}

/// Structured result for one child.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildOutcome {
    /// The [`ChildRequest::id`] this corresponds to.
    pub id: String,
    /// Terminal status.
    pub status: ChildStatus,
    /// Final (or partial, on failure) output text.
    pub output: String,
    /// The backend/harness actually used ("hq" for in-process).
    pub resolved_backend: String,
    /// True when the service auto-fell-back to in-process because the policy
    /// preferred an external backend but none was available. Always visible,
    /// never silent.
    pub fallback_used: bool,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Error description when the child failed/timed out/was rejected.
    #[serde(default)]
    pub error: Option<String>,
    /// The child's correlated run id, when it actually ran.
    #[serde(default)]
    pub run_id: Option<String>,
    /// The model/backend the child was dispatched on and whether it was
    /// inherited or overridden. `None` when it never got that far.
    #[serde(default)]
    pub effective_model: Option<EffectiveModel>,
    /// Citations in the child's answer checked against the context packet it
    /// was given. `None` when the child had no packet.
    #[serde(default)]
    pub evidence_check: Option<hq_memory::context_packet::CitationReport>,
    /// Durable run record and acceptance verdict, set once the service has
    /// settled the child. `None` only for outcomes built outside the service.
    #[serde(default)]
    pub run: Option<RunInfo>,
}

/// What the registry knows about a settled child beyond how it exited.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInfo {
    /// Key into the durable run registry.
    pub run_id: String,
    /// `unverified`, `partial` or `blocked` from the service; only the parent
    /// reviewing the result may raise it to `accepted`.
    pub accept_status: String,
    #[serde(default)]
    pub missing_deliverables: Vec<String>,
    #[serde(default)]
    pub blocker: Option<String>,
    #[serde(default)]
    pub next_action: Option<String>,
}

impl ChildOutcome {
    /// Build a rejected outcome (governance denial) that never ran.
    pub fn rejected(id: impl Into<String>, reason: impl Into<String>) -> Self {
        let reason = hq_core::redact::redact_secrets(&reason.into());
        Self {
            id: id.into(),
            status: ChildStatus::Rejected,
            output: reason.clone(),
            resolved_backend: String::new(),
            fallback_used: false,
            duration_ms: 0,
            error: Some(reason),
            run_id: None,
            effective_model: None,
            evidence_check: None,
            run: None,
        }
    }

    /// Build a blocked outcome (a dependency failed, so this never ran).
    pub fn blocked(id: impl Into<String>, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            id: id.into(),
            status: ChildStatus::Blocked,
            output: reason.clone(),
            resolved_backend: String::new(),
            fallback_used: false,
            duration_ms: 0,
            error: Some(reason),
            run_id: None,
            effective_model: None,
            evidence_check: None,
            run: None,
        }
    }

    /// Build a failed outcome for a child task that panicked or was otherwise
    /// lost by its `JoinHandle` before it could produce a real outcome. This
    /// must always be surfaced explicitly — silently dropping the id would
    /// make `Parallel` mode lose results and would make `Graph` mode treat the
    /// task as still-unresolved and reschedule it forever.
    pub fn panicked(id: impl Into<String>, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            id: id.into(),
            status: ChildStatus::Failed,
            output: reason.clone(),
            resolved_backend: String::new(),
            fallback_used: false,
            duration_ms: 0,
            error: Some(reason),
            run_id: None,
            effective_model: None,
            evidence_check: None,
            run: None,
        }
    }
}

/// Per-execute context threaded from the parent run.
#[derive(Clone, Default)]
pub struct ChildExecContext {
    /// The parent run's id. Forwarded child envelopes carry this as their
    /// `parent_run_id`, and the `ChildStarted`/`ChildFinished` markers carry it
    /// as their own `run_id`.
    pub parent_run_id: Option<String>,
    /// Where correlated envelopes are pushed. `None` = no streaming (outcomes
    /// are still returned).
    pub envelope_sink: Option<EnvelopeSink>,
    /// Parent conversation messages shared into children (fork context) when the
    /// request's context policy allows it.
    pub parent_messages: Option<Vec<ChatMessage>>,
    /// The originating chat turn id (e.g. a `background_turns` row). Copied
    /// into every [`ChildCompletionEvent`] so a consumer can route async child
    /// results back to the turn that spawned them.
    pub parent_turn_id: Option<String>,
    /// Completion callback invoked once per child when a non-blocking plan
    /// ([`ChildPlan::blocking`] = `Some(false)`) finishes a child. `None` =
    /// events are dropped (detached children still run).
    pub completion_sink: Option<CompletionSink>,
    /// Progress callback fired when the agent volunteers a mid-turn progress
    /// note via the `report_progress` tool. `None` = the tool is a graceful
    /// no-op. The turn id is carried by [`ChildExecContext::parent_turn_id`].
    pub progress_sink: Option<crate::native_hq::ProgressSink>,
    /// The chat this delegation belongs to. Scopes run inspection and routes
    /// the automatic parent follow-up. `None` = runs are recorded unrouted.
    pub origin: Option<hq_db::subagent_runs::Origin>,
    /// Whether HQ will resume the parent conversation on its own when a
    /// background child settles. Drives what the dispatch acknowledgement
    /// is allowed to promise.
    pub auto_followup: bool,
}

impl std::fmt::Debug for ChildExecContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildExecContext")
            .field("parent_run_id", &self.parent_run_id)
            .field("has_sink", &self.envelope_sink.is_some())
            .field(
                "parent_messages",
                &self.parent_messages.as_ref().map(|m| m.len()),
            )
            .field("parent_turn_id", &self.parent_turn_id)
            .field("has_completion_sink", &self.completion_sink.is_some())
            .field("has_progress_sink", &self.progress_sink.is_some())
            .field("origin", &self.origin)
            .field("auto_followup", &self.auto_followup)
            .finish()
    }
}

/// A cloneable sink the service pushes child-completion events into when a
/// plan runs non-blocking. Mirrors the `DetachedTurnSink` precedent from the
/// native-hq runtime: the caller owns routing (Task 7 wires this to the bus).
pub type CompletionSink = Arc<dyn Fn(ChildCompletionEvent) + Send + Sync>;

/// One child's terminal result, emitted asynchronously for non-blocking plans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildCompletionEvent {
    /// The originating chat turn, when the parent supplied one.
    pub parent_turn_id: Option<String>,
    /// The [`ChildRequest::id`] this completion corresponds to.
    pub task_id: String,
    /// Lowercase role string ([`ChildRequest::role`]).
    pub role: String,
    /// `true` only for a clean [`ChildStatus::Completed`].
    pub success: bool,
    /// Final (or partial/error) output text, truncated to a preview length.
    pub summary: String,
    /// Registry key for the full result. `None` when no registry is attached.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Terminal status name (`completed`, `failed`, `timed_out`, ...).
    #[serde(default)]
    pub status: Option<String>,
    /// `unverified`, `partial` or `blocked`.
    #[serde(default)]
    pub accept_status: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The timeout to apply to a child: request override or the service default.
pub(crate) fn resolve_timeout(req: &ChildRequest, default: Duration) -> Duration {
    req.timeout_secs
        .map(|s| Duration::from_secs(s.clamp(1, 3600)))
        .unwrap_or(default)
}
