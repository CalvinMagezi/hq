//! Task-outcome telemetry plumbing for the router.
//!
//! The router is sync-facing; recording outcomes to SQLite runs in a spawned
//! task so completion-site latency is unaffected. Session identity flows from
//! `hq-agent` via a `tokio::task_local!` scope so the router doesn't need to
//! know about sessions directly — if no context is set the outcome is still
//! recorded with a synthetic `session_id = "unattributed"`.

use std::sync::Arc;

/// Minimal context the router needs to attribute an outcome to a session.
///
/// `hq-agent` sets this via `SESSION_CONTEXT.scope(ctx, async { ... }).await`
/// at the start of every LLM call. The router reads it with `current()`.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub session_id: String,
    /// Monotonic turn counter within the session. Incremented by the session
    /// loop; the router just reads whatever is current.
    pub turn_idx: i64,
}

impl SessionContext {
    pub fn unattributed() -> Self {
        Self {
            session_id: "unattributed".into(),
            turn_idx: 0,
        }
    }
}

tokio::task_local! {
    pub static SESSION_CONTEXT: SessionContext;
}

/// Return the current session context, or `unattributed` if none is set.
pub fn current_context() -> SessionContext {
    SESSION_CONTEXT
        .try_with(|c| c.clone())
        .unwrap_or_else(|_| SessionContext::unattributed())
}

/// One completed LLM call, ready for the sink to persist.
///
/// Intentionally lean — just the dimensions the router already has.
/// The sink is responsible for translation to whatever storage layout it uses.
#[derive(Debug, Clone)]
pub struct OutcomeEvent {
    pub session_id: String,
    pub turn_idx: i64,
    pub model: String,
    pub provider: String,
    pub task_hint: &'static str,
    pub latency_ms: i64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: f64,
    pub success: bool,
    pub error_class: Option<String>,
}

/// Sink trait implemented by `hq-agent` with a `Database` handle.
///
/// Implementations should not block — the router dispatches via
/// `tokio::spawn` so slow sinks don't stall routing.
#[async_trait::async_trait]
pub trait TaskOutcomeSink: Send + Sync {
    async fn record(&self, event: OutcomeEvent);
}

/// Convenience alias for shared sink handles.
pub type SharedSink = Arc<dyn TaskOutcomeSink>;
