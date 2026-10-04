//! `TaskOutcomeSink` implementation backed by `hq_db::task_outcomes`.
//!
//! Translates the provider-agnostic `OutcomeEvent` emitted by the router into
//! a `TaskOutcome` row. Writes run on a blocking pool (`spawn_blocking`) so
//! SQLite contention never stalls the Tokio runtime.

use std::sync::Arc;

use async_trait::async_trait;
use hq_db::Database;
use hq_db::task_outcomes::{self, TaskOutcome};
use hq_llm::{OutcomeEvent, TaskOutcomeSink};
use tracing::warn;

/// Sink backed by the shared SQLite pool.
pub struct DbOutcomeSink {
    db: Arc<Database>,
}

impl DbOutcomeSink {
    pub fn new(db: Arc<Database>) -> Arc<Self> {
        Arc::new(Self { db })
    }
}

#[async_trait]
impl TaskOutcomeSink for DbOutcomeSink {
    async fn record(&self, event: OutcomeEvent) {
        let db = self.db.clone();
        let outcome = TaskOutcome {
            session_id: event.session_id,
            turn_idx: event.turn_idx,
            model: event.model,
            provider: event.provider,
            task_hint: event.task_hint.to_string(),
            latency_ms: event.latency_ms,
            input_tokens: event.input_tokens,
            output_tokens: event.output_tokens,
            cost_usd: event.cost_usd,
            success: event.success,
            error_class: event.error_class,
            quality_score: None,
            tool_calls_issued: 0,
            tool_calls_succeeded: 0,
            recorded_at: current_epoch(),
        };

        // Thread-safety note: r2d2 hands out one connection per thread, and
        // `spawn_blocking` runs the closure on its own blocking thread. SQLite
        // WAL mode serializes writers at the filesystem level, so our use of
        // `unchecked_transaction` inside `task_outcomes::insert` is safe: the
        // conn is single-owned for the duration of the closure.
        let res = tokio::task::spawn_blocking(move || {
            db.with_conn(|conn| task_outcomes::insert(conn, &outcome).map(|_| ()))
        })
        .await;

        match res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => warn!(error = %e, "failed to persist task outcome"),
            Err(e) => warn!(error = %e, "task outcome sink join error"),
        }
    }
}

fn current_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
