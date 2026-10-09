//! Daily ledger upkeep: fold old per-call rows into daily totals, then check the ledger against
//! what the provider says it billed and say so in the log when they drift apart.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::usage_ledger::rollup_and_prune;
use hq_llm::reconcile::Verdict;
use tracing::{info, warn};

use crate::commands::usage::drift_against_openrouter;

pub async fn run_usage_ledger(db: &Database, config: &HqConfig) -> Result<()> {
    let retain = config.usage_ledger.effective_retain_days();
    let now = chrono::Utc::now().timestamp();
    let folded = db.with_conn(|conn| rollup_and_prune(conn, now, retain))?;
    if folded > 0 {
        info!(
            rows = folded,
            retain_days = retain,
            "usage ledger: folded old rows into daily totals"
        );
    }
    let (dropped, unscoped) = (
        hq_agent::dropped_outcomes(),
        hq_llm::unscoped_calls(),
    );
    if dropped > 0 || unscoped > 0 {
        warn!(
            dropped,
            unscoped, "usage ledger: calls were lost or recorded without an origin since start"
        );
    }
    match drift_against_openrouter(config, db, now).await {
        Ok(Some(windows)) => {
            for w in windows
                .iter()
                .filter(|w| matches!(w.verdict, Verdict::LedgerLow | Verdict::LedgerHigh))
            {
                warn!(
                    window = %w.window,
                    ledger_usd = w.ledger_usd,
                    billed_usd = w.provider_usd,
                    drift_pct = w.drift_pct,
                    "usage ledger disagrees with OpenRouter billing; run `hq usage reconcile`"
                );
            }
        }
        Ok(None) => {}
        Err(e) => warn!(error = %e, "usage ledger: could not read OpenRouter billing"),
    }
    Ok(())
}
