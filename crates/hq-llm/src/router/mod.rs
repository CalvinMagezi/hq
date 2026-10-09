mod builder;
pub(crate) mod health;
mod selection;
mod strategy;
mod types;

#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod tests;

pub use health::ProviderHealth;
pub(crate) use health::classify_error_for_telemetry;
pub use types::{CostTier, RouteEntry, TaskHint};

use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use crate::provider::LlmProvider;

/// Self-observing, cost-aware LLM router.
///
/// Routing strategy:
/// 1. Resolve all matching routes for the requested model
/// 2. Filter out unhealthy/cooling-down providers
/// 3. Score each candidate: free first, then cheapest capable,
///    weighted by health and task-type reliability
/// 4. Try in score order, failover on error
/// 5. Track success/failure per provider per task type
pub struct LlmRouter {
    pub(super) providers: Vec<(String, Arc<dyn LlmProvider>)>,
    pub(super) routes: Vec<RouteEntry>,
    pub(super) round_robin: AtomicUsize,
    pub(super) health: Arc<Mutex<Vec<(String, ProviderHealth)>>>,
    pub(super) instruments: Arc<crate::instrument::Instruments>,
}

impl Default for LlmRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmRouter {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            routes: Vec::new(),
            round_robin: AtomicUsize::new(0),
            health: Arc::new(Mutex::new(Vec::new())),
            instruments: crate::instrument::Instruments::new(),
        }
    }
}
