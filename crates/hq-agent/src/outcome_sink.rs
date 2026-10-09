//! `TaskOutcomeSink` implementation backed by `hq_db::task_outcomes`.
//!
//! Translates the provider-agnostic `OutcomeEvent` emitted by the router into
//! a `TaskOutcome` row. Writes run on a blocking pool (`spawn_blocking`) so
//! SQLite contention never stalls the Tokio runtime.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use hq_db::Database;
use hq_db::task_outcomes::{self, TaskOutcome};
use hq_llm::{OutcomeEvent, TaskOutcomeSink};
use tracing::warn;

/// One more attempt after a short pause covers a writer that held the lock a moment too long.
const RETRY_DELAY: Duration = Duration::from_millis(250);

static DROPPED: AtomicU64 = AtomicU64::new(0);

/// Outcomes that could not be written even after a retry since the process started.
pub fn dropped_outcomes() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

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
            cache_read_tokens: event.cache_read_tokens,
            cache_write_tokens: event.cache_write_tokens,
            reasoning_tokens: event.reasoning_tokens,
            cost_usd: event.cost_usd,
            provider_cost_usd: event.provider_cost_usd,
            cost_source: event.cost_source.to_string(),
            origin: event.origin.to_string(),
            success: event.success,
            error_class: event.error_class,
            quality_score: None,
            tool_calls_issued: 0,
            tool_calls_succeeded: 0,
            recorded_at: current_epoch(),
        };

        if write_blocking(&db, &outcome).await.is_ok() {
            return;
        }
        tokio::time::sleep(RETRY_DELAY).await;
        if let Err(e) = write_blocking(&db, &outcome).await {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            warn!(error = %e, model = %outcome.model, "LLM outcome lost: could not be written to the ledger");
        }
    }
}

/// Runs on the blocking pool so SQLite contention never stalls the Tokio runtime. WAL mode
/// serializes writers, so the single-owned connection inside the closure is safe.
async fn write_blocking(db: &Arc<Database>, outcome: &TaskOutcome) -> anyhow::Result<()> {
    let db = db.clone();
    let outcome = outcome.clone();
    tokio::task::spawn_blocking(move || {
        db.with_conn(|conn| task_outcomes::insert(conn, &outcome).map(|_| ()))
    })
    .await?
}

fn current_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
