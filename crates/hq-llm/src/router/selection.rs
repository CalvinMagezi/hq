use std::sync::Arc;

use rand::Rng;

use crate::provider::LlmProvider;

use super::LlmRouter;
use super::health::ProviderHealth;
use super::types::{CostTier, RouteEntry, TaskHint};

/// A scored candidate for routing.
#[derive(Clone)]
pub(super) struct ScoredCandidate {
    pub provider: Arc<dyn LlmProvider>,
    pub model_id: String,
    pub provider_name: String,
    pub score: f64,
    pub cost_tier: CostTier,
    pub is_local: bool,
    /// Whether this candidate matched via a wildcard route (e.g. "cerebras/*").
    pub is_wildcard: bool,
}

/// Score carried by a lone candidate: with nothing to rank against, it is never scored.
pub(super) const UNSCORED: f64 = 0.0;

/// Jitter half-width that keeps equally scored providers sharing load.
const SCORE_JITTER: f64 = 2.5;

/// Find ALL matching routes for a given model string, scored by cost and health.
/// A single match (the whole `backends:` setup) skips scoring entirely.
pub(super) fn resolve_scored(
    model: &str,
    task: TaskHint,
    providers: &[(String, Arc<dyn LlmProvider>)],
    routes: &[RouteEntry],
    health: &[(String, ProviderHealth)],
    pressure: &dyn Fn(&str) -> f64,
) -> Vec<ScoredCandidate> {
    let mut candidates = route_candidates(model, providers, routes, health);
    if candidates.is_empty() {
        candidates.extend(prefix_candidate(model, providers, routes));
    }
    if candidates.len() > 1 {
        rank(&mut candidates, task, health, pressure);
    }
    candidates
}

/// Unscored candidates from explicit and wildcard routes. A cooling-down
/// provider is excluded outright; demoting it by score would still retry it
/// on every request for the whole cooldown window.
fn route_candidates(
    model: &str,
    providers: &[(String, Arc<dyn LlmProvider>)],
    routes: &[RouteEntry],
    health: &[(String, ProviderHealth)],
) -> Vec<ScoredCandidate> {
    routes
        .iter()
        .filter(|route| route_matches(&route.pattern, model))
        .filter(|route| !health_of(health, &route.provider).is_some_and(|h| h.is_cooling_down()))
        .filter_map(|route| {
            let (_, provider) = providers.iter().find(|(n, _)| n == &route.provider)?;
            Some(ScoredCandidate {
                provider: provider.clone(),
                model_id: route.model_id.clone(),
                provider_name: route.provider.clone(),
                score: UNSCORED,
                cost_tier: route.cost_tier,
                is_local: route.is_local,
                is_wildcard: route.pattern.ends_with('*'),
            })
        })
        .collect()
}

/// `provider/model` with no matching route: the named provider, with the
/// tier of its `provider/*` route when one is registered.
fn prefix_candidate(
    model: &str,
    providers: &[(String, Arc<dyn LlmProvider>)],
    routes: &[RouteEntry],
) -> Option<ScoredCandidate> {
    let (provider_name, model_id) = model.split_once('/')?;
    let (_, provider) = providers.iter().find(|(n, _)| n == provider_name)?;
    let wildcard_pattern = format!("{provider_name}/*");
    let (cost_tier, is_local) = routes
        .iter()
        .find(|r| r.pattern == wildcard_pattern)
        .map(|r| (r.cost_tier, r.is_local))
        .unwrap_or((CostTier::Budget, false));
    Some(ScoredCandidate {
        provider: provider.clone(),
        model_id: model_id.to_string(),
        provider_name: provider_name.to_string(),
        score: UNSCORED,
        cost_tier,
        is_local,
        is_wildcard: false,
    })
}

fn route_matches(pattern: &str, model: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => model.starts_with(prefix),
        None => model == pattern,
    }
}

fn health_of<'a>(health: &'a [(String, ProviderHealth)], name: &str) -> Option<&'a ProviderHealth> {
    health.iter().find(|(n, _)| n == name).map(|(_, h)| h)
}

/// Score with jitter, then order best first with explicit routes ahead of
/// wildcards. For tool, coding and planning work, Free candidates go last
/// whenever a paid one exists (free models give shallow multi-turn tool
/// chains); that ordering replaces the wildcard rule, as it always has.
fn rank(
    candidates: &mut [ScoredCandidate],
    task: TaskHint,
    health: &[(String, ProviderHealth)],
    pressure: &dyn Fn(&str) -> f64,
) {
    let mut rng = rand::thread_rng();
    for c in candidates.iter_mut() {
        let base = compute_score_pressured(
            c.cost_tier,
            c.is_local,
            health_of(health, &c.provider_name),
            task,
            pressure(&c.provider_name),
        );
        c.score = (base + rng.gen_range(-SCORE_JITTER..SCORE_JITTER)).max(0.0);
    }
    let demote_free = matches!(
        task,
        TaskHint::ToolUse | TaskHint::Coding | TaskHint::Planning
    ) && candidates.iter().any(|c| !is_free(c));
    candidates.sort_by(|a, b| {
        let first = if demote_free {
            is_free(a).cmp(&is_free(b))
        } else {
            a.is_wildcard.cmp(&b.is_wildcard)
        };
        first.then(
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
}

fn is_free(c: &ScoredCandidate) -> bool {
    matches!(c.cost_tier, CostTier::Free)
}

/// Compute a composite score for a provider candidate.
/// Higher = better. Range roughly 0-100.
///
/// Formula (task-dependent weights):
///   ToolUse/Coding: cost=0.10 health=0.30 reliability=0.40 speed=0.20
///   Planning:       cost=0.15 health=0.25 reliability=0.35 speed=0.25
///   Simple/Bulk:    cost=0.30 health=0.30 reliability=0.20 speed=0.20
///
/// - cost_score: Free=100, Budget=60, Standard=30, Premium=10 (local Free=50)
///   Penalized when daily token budget >80% consumed.
/// - health_score: healthy=100, cooling_down=0, consecutive_failures penalized
/// - reliability_score: task-type success rate (sliding window) * 100
/// - speed_score: 100 for <500ms avg, decreasing for slower providers
pub(super) fn compute_score(
    cost_tier: CostTier,
    is_local: bool,
    health: Option<&ProviderHealth>,
    task: TaskHint,
) -> f64 {
    compute_score_pressured(cost_tier, is_local, health, task, 0.0)
}

/// How much more the cost score counts when a budget is nearly used up, at full pressure: the
/// cost weight is multiplied by one plus this, and the other weights shrink to keep the sum at one.
const PRESSURE_COST_BOOST: f64 = 6.0;

/// [`compute_score`] with budget pressure in `[0, 1]`: 0 leaves the weights alone, 1 makes the price
/// of a provider matter far more, so spend drifts to cheaper providers as a budget drains instead
/// of hitting a wall.
pub(super) fn compute_score_pressured(
    cost_tier: CostTier,
    is_local: bool,
    health: Option<&ProviderHealth>,
    task: TaskHint,
    pressure: f64,
) -> f64 {
    // Local free providers score lower than cloud free providers.
    let mut cost_score = if is_local {
        50.0 // Between Budget(60) and Standard(30), below cloud Free(100)
    } else {
        match cost_tier {
            CostTier::Free => 100.0,
            CostTier::Budget => 60.0,
            CostTier::Standard => 30.0,
            CostTier::Premium => 10.0,
        }
    };

    let (health_score, reliability_score, speed_score) = match health {
        Some(h) => {
            // Daily budget penalty: approaching the limit reduces cost advantage
            let budget_ratio = h.daily_budget_ratio();
            if budget_ratio > 0.8 {
                // Linear penalty from 80% to 100%: score drops by up to 50%
                let penalty = ((budget_ratio - 0.8) / 0.2).min(1.0) * 0.5;
                cost_score *= 1.0 - penalty;
            }

            let hs = if h.is_cooling_down() {
                0.0
            } else if h.consecutive_failures > 5 {
                10.0
            } else if h.consecutive_failures > 0 {
                100.0 - (h.consecutive_failures as f64 * 15.0)
            } else {
                100.0
            };

            let rs = h.task_success_rate(task) * 100.0;

            // Speed score: fast providers score higher
            // <500ms = 100, 1s = 80, 5s = 40, 30s = 10, >60s = 0
            let ss = if h.avg_latency_ms == 0.0 {
                50.0 // No data yet, neutral
            } else if h.avg_latency_ms < 500.0 {
                100.0
            } else if h.avg_latency_ms < 1000.0 {
                80.0
            } else if h.avg_latency_ms < 5000.0 {
                40.0
            } else if h.avg_latency_ms < 30000.0 {
                10.0
            } else {
                0.0
            };

            (hs, rs, ss)
        }
        None => {
            // No health data: strong reliability edge for paid providers on ToolUse.
            // Free providers are unreliable for multi-turn tool chains (429s, shallow).
            let reliability_bootstrap = match (task, cost_tier) {
                (TaskHint::ToolUse | TaskHint::Coding, CostTier::Budget) => 80.0,
                (TaskHint::ToolUse | TaskHint::Coding, CostTier::Standard | CostTier::Premium) => {
                    85.0
                }
                (TaskHint::Planning, CostTier::Budget) => 75.0,
                (TaskHint::Planning, CostTier::Standard | CostTier::Premium) => 80.0,
                (TaskHint::ToolUse | TaskHint::Coding | TaskHint::Planning, CostTier::Free) => 35.0,
                _ => 50.0, // Simple/Bulk: neutral
            };
            (50.0, reliability_bootstrap, 50.0)
        }
    };

    let (w_cost, w_health, w_reliability, w_speed) = match task {
        TaskHint::ToolUse | TaskHint::Coding => (0.05, 0.25, 0.55, 0.15),
        TaskHint::Planning => (0.10, 0.25, 0.45, 0.20),
        TaskHint::Notification => (0.60, 0.20, 0.10, 0.10),
        _ => (0.30, 0.30, 0.20, 0.20),
    };
    let pressure = pressure.clamp(0.0, 1.0);
    let boosted = (w_cost * (1.0 + PRESSURE_COST_BOOST * pressure)).min(1.0);
    let rest = if w_cost < 1.0 { (1.0 - boosted) / (1.0 - w_cost) } else { 1.0 };
    cost_score * boosted
        + health_score * w_health * rest
        + reliability_score * w_reliability * rest
        + speed_score * w_speed * rest
}

impl LlmRouter {
    #[cfg(test)]
    pub(super) fn resolve_scored(&self, model: &str, task: TaskHint) -> Vec<ScoredCandidate> {
        let health = self.health.lock().unwrap();
        resolve_scored(model, task, &self.providers, &self.routes, &health, &|p| {
            self.instruments.pressure_for(p)
        })
    }

    pub fn compute_score(
        cost_tier: CostTier,
        is_local: bool,
        health: Option<&ProviderHealth>,
        task: TaskHint,
    ) -> f64 {
        compute_score(cost_tier, is_local, health, task)
    }
}
