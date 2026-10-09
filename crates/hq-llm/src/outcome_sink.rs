//! Task-outcome telemetry plumbing for the router.
//!
//! The router is sync-facing; recording outcomes to SQLite runs in a spawned
//! task so completion-site latency is unaffected. Session identity flows from
//! `hq-agent` via a `tokio::task_local!` scope so the router doesn't need to
//! know about sessions directly — if no context is set the outcome is still
//! recorded with a synthetic `session_id = "unattributed"`.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// What kind of work an LLM call served. Stored in `task_outcomes.origin`.
pub mod origin {
    pub const CHAT: &str = "chat";
    pub const BACKGROUND: &str = "background";
    pub const SUBAGENT: &str = "subagent";
    pub const MEMORY: &str = "memory";
    pub const SKILL_REVIEW: &str = "skill_review";
    pub const COMPACTION: &str = "compaction";
    pub const EMBEDDINGS: &str = "embeddings";
    pub const SUPERVISOR: &str = "supervisor";
    pub const SETUP: &str = "setup";
    pub const CLI: &str = "cli";
    /// A call nobody scoped. Should stay at zero; `unscoped_calls()` counts it.
    pub const UNKNOWN: &str = "unknown";
}

static UNSCOPED_CALLS: AtomicU64 = AtomicU64::new(0);

/// LLM calls recorded without any origin scope since the process started.
pub fn unscoped_calls() -> u64 {
    UNSCOPED_CALLS.load(Ordering::Relaxed)
}

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
    pub origin: &'static str,
}

impl SessionContext {
    pub fn unattributed() -> Self {
        Self {
            session_id: "unattributed".into(),
            turn_idx: 0,
            origin: origin::UNKNOWN,
        }
    }
}

/// Like [`with_origin`], but an origin an outer caller already chose is left alone.
pub async fn with_default_origin<F: Future>(origin: &'static str, fut: F) -> F::Output {
    let ctx = current_context();
    if ctx.origin == origin::UNKNOWN {
        with_origin(origin, fut).await
    } else {
        fut.await
    }
}

/// Run `fut` with every LLM call inside it tagged `origin`. An enclosing session keeps its id.
pub async fn with_origin<F: Future>(origin: &'static str, fut: F) -> F::Output {
    let mut ctx = current_context();
    ctx.origin = origin;
    SESSION_CONTEXT.scope(ctx, fut).await
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

/// The context to attribute a finished call to, counting the ones nobody scoped.
pub(crate) fn context_for_record() -> SessionContext {
    let ctx = current_context();
    if ctx.origin == origin::UNKNOWN {
        UNSCOPED_CALLS.fetch_add(1, Ordering::Relaxed);
    }
    ctx
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
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost_usd: f64,
    pub cost_source: &'static str,
    pub origin: &'static str,
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
