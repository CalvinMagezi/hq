//! Enforces the owner's spending limits against the ledger, and raises an alert the first time
//! each limit crosses a threshold in a period.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hq_core::config::{Budget, BudgetAction, BudgetScope, BudgetsConfig};
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::usage_ledger::{BudgetStatus, budget_status};
use hq_llm::budget::{Admission, BudgetBlocked, BudgetGate, GateRequest};
use tracing::warn;

/// Limits are re-read this often, so editing them takes effect without a restart.
const CONFIG_TTL: Duration = Duration::from_secs(30);
/// A burst of calls shares one ledger read per budget.
const SPEND_TTL: Duration = Duration::from_secs(5);
const FULL_PCT: u8 = 100;

/// `None` when the configuration could not be read, so the last good one stays in force.
type ConfigLoader = Arc<dyn Fn() -> Option<BudgetsConfig> + Send + Sync>;

struct Cache {
    config: Option<(Instant, BudgetsConfig)>,
    /// The last configuration that was read successfully, kept when a later read fails.
    last_good: Option<BudgetsConfig>,
    status: HashMap<String, (Instant, BudgetStatus)>,
    alerted: HashSet<(String, i64, u8)>,
}

pub struct LedgerBudgetGate {
    db: Arc<Database>,
    load: ConfigLoader,
    cache: Mutex<Cache>,
}

impl LedgerBudgetGate {
    pub fn new(db: Arc<Database>, load: ConfigLoader) -> Arc<Self> {
        Arc::new(Self {
            db,
            load,
            cache: Mutex::new(Cache {
                config: None,
                last_good: None,
                status: HashMap::new(),
                alerted: HashSet::new(),
            }),
        })
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        // A panic elsewhere must not turn every later LLM call into a panic.
        self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn budgets(&self) -> BudgetsConfig {
        if let Some((at, cfg)) = &self.cache().config
            && at.elapsed() < CONFIG_TTL
        {
            return cfg.clone();
        }
        // Read outside the lock: it is file I/O.
        let loaded = (self.load)();
        let mut cache = self.cache();
        let cfg = match loaded {
            Some(cfg) => {
                for problem in cfg.problems() {
                    warn!(%problem, "budget configuration problem; that budget is not enforced");
                }
                cache.last_good = Some(cfg.clone());
                cfg
            }
            None => cache.last_good.clone().unwrap_or_default(),
        };
        cache.config = Some((Instant::now(), cfg.clone()));
        cfg
    }

    async fn status(&self, budget: &Budget, now: i64) -> Option<BudgetStatus> {
        let key = format!(
            "{}|{}|{:?}|{}",
            budget.name, budget.scope, budget.period, budget.limit_usd
        );
        if let Some((at, s)) = self.cache().status.get(&key)
            && at.elapsed() < SPEND_TTL
            && s.period_start <= now
            && now < s.resets_at
        {
            return Some(s.clone());
        }
        let (db, b) = (self.db.clone(), budget.clone());
        let status = tokio::task::spawn_blocking(move || db.with_conn(|c| budget_status(c, &b, now)))
            .await
            .map_err(|e| warn!(error = %e, "budget spend read was cancelled"))
            .ok()?
            .map_err(|e| warn!(error = %e, budget = %budget.name, "budget spend could not be read"))
            .ok()?;
        self.cache().status.insert(key, (Instant::now(), status.clone()));
        Some(status)
    }

    /// Alert once per period for each threshold the spend has reached.
    fn alert(&self, budget: &Budget, status: &BudgetStatus) {
        let thresholds = budget.soft_pct.iter().copied().chain([FULL_PCT]);
        for t in thresholds.filter(|t| status.pct >= f64::from(*t)) {
            let first = self
                .cache
                .lock()
                .unwrap()
                .alerted
                .insert((budget.name.clone(), status.period_start, t));
            if !first {
                continue;
            }
            let item = ValueItem::new(
                "budget",
                ValueKind::Fyi,
                format!("Budget '{}' is at {t}% of ${:.2}", budget.name, budget.limit_usd),
                format!(
                    "${:.2} of the ${:.2} {} limit on {} is spent. Action at the limit: {:?}.",
                    status.spent_usd, budget.limit_usd, period_word(budget), budget.scope, budget.action
                ),
            )
            .with_dedup_key(format!("budget:{}:{}:{t}", budget.name, status.period_start));
            let db = self.db.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = hq_db::value_items::emit(&db, &item) {
                    warn!(error = %e, "budget alert could not be recorded");
                }
            });
        }
    }
}

fn period_word(b: &Budget) -> &'static str {
    match b.period {
        hq_core::config::BudgetPeriod::Day => "daily",
        hq_core::config::BudgetPeriod::Week => "weekly",
        hq_core::config::BudgetPeriod::Month => "monthly",
    }
}

fn covers(scope: &BudgetScope, req: &GateRequest<'_>) -> bool {
    match scope {
        BudgetScope::Global => true,
        BudgetScope::Provider(p) => p == req.provider,
        BudgetScope::Model(m) => m == req.model,
        BudgetScope::Origin(o) => o == req.origin,
    }
}

fn blocked(budget: &Budget, spent: f64, why: String) -> Admission {
    Admission::Deny(BudgetBlocked {
        budget: budget.name.clone(),
        spent_usd: spent,
        limit_usd: budget.limit_usd,
        message: format!("Budget '{}' blocks this call: {why}", budget.name),
    })
}

#[async_trait]
impl BudgetGate for LedgerBudgetGate {
    async fn admit(&self, req: &GateRequest<'_>) -> Admission {
        let cfg = self.budgets();
        let now = chrono::Utc::now().timestamp();
        let mut verdict = Admission::Allow;
        for budget in cfg.enforceable().into_iter().filter(|b| covers(&b.scope, req)) {
            let Some(status) = self.status(budget, now).await else {
                continue;
            };
            self.alert(budget, &status);
            // A call that costs nothing (local, subscription) never adds to spend.
            if req.estimate_usd == Some(0.0) {
                continue;
            }
            let projected = status.spent_usd + req.estimate_usd.unwrap_or(0.0);
            let over = status.spent_usd >= budget.limit_usd || projected > budget.limit_usd;
            match budget.action {
                BudgetAction::Block if req.estimate_usd.is_none() => {
                    if !cfg.allow_unpriced_models.iter().any(|m| m == req.model) {
                        return blocked(
                            budget,
                            status.spent_usd,
                            format!(
                                "HQ has no price for model '{}', so it cannot be counted. Add it to budgets.allow_unpriced_models to allow it.",
                                req.model
                            ),
                        );
                    }
                }
                BudgetAction::Block if over => {
                    return blocked(
                        budget,
                        status.spent_usd,
                        format!(
                            "${:.2} of ${:.2} is spent and this call could cost about ${:.4} more.",
                            status.spent_usd,
                            budget.limit_usd,
                            req.estimate_usd.unwrap_or(0.0)
                        ),
                    );
                }
                BudgetAction::Downgrade if over => {
                    if let Some(model) = budget.downgrade_model.as_deref()
                        && model != req.model
                    {
                        verdict = Admission::Downgrade {
                            model: model.to_string(),
                        };
                    }
                }
                _ => {}
            }
        }
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::task_outcomes::{TaskOutcome, insert};
    use hq_llm::cost::ProviderClass;

    const MODEL: &str = "anthropic/claude-haiku-5.5";

    fn db_with_spend(origin: &str, usd: f64) -> Arc<Database> {
        let db = Arc::new(Database::open_memory().unwrap());
        let mut o = TaskOutcome::now("s", 0, MODEL, "haiku", "chat");
        o.cost_usd = usd;
        o.origin = origin.into();
        db.with_conn(|c| insert(c, &o)).unwrap();
        db
    }

    fn gate(db: Arc<Database>, yaml: &str) -> Arc<LedgerBudgetGate> {
        let cfg: BudgetsConfig = serde_yaml::from_str(yaml).unwrap();
        LedgerBudgetGate::new(db, Arc::new(move || Some(cfg.clone())))
    }

    fn req<'a>(provider: &'a str, class: ProviderClass, est: Option<f64>) -> GateRequest<'a> {
        GateRequest {
            provider,
            class,
            model: MODEL,
            origin: "chat",
            estimate_usd: est,
        }
    }

    const MONTH_BLOCK: &str =
        "budgets:\n  - {name: month, scope: global, period: month, limit_usd: 10}\n";

    #[tokio::test]
    async fn under_the_limit_a_call_goes_through() {
        let g = gate(db_with_spend("chat", 1.0), MONTH_BLOCK);
        assert!(matches!(g.admit(&req("haiku", ProviderClass::Metered, Some(0.01))).await, Admission::Allow));
    }

    #[tokio::test]
    async fn a_call_that_would_cross_the_limit_is_blocked_with_the_numbers() {
        let g = gate(db_with_spend("chat", 9.995), MONTH_BLOCK);
        let Admission::Deny(b) = g.admit(&req("haiku", ProviderClass::Metered, Some(0.02))).await else {
            panic!("expected a refusal");
        };
        assert_eq!((b.budget.as_str(), b.limit_usd), ("month", 10.0));
        assert!(b.message.contains("$10.00"), "{}", b.message);
    }

    #[tokio::test]
    async fn a_call_that_costs_nothing_is_allowed_even_when_the_budget_is_used_up() {
        let g = gate(db_with_spend("chat", 50.0), MONTH_BLOCK);
        assert!(matches!(g.admit(&req("ollama", ProviderClass::Local, Some(0.0))).await, Admission::Allow));
    }

    #[tokio::test]
    async fn an_unpriced_model_is_refused_under_a_blocking_budget_unless_allow_listed() {
        let g = gate(db_with_spend("chat", 0.0), MONTH_BLOCK);
        assert!(matches!(g.admit(&req("haiku", ProviderClass::Metered, None)).await, Admission::Deny(_)));
        let allowed = gate(
            db_with_spend("chat", 0.0),
            &format!("{MONTH_BLOCK}allow_unpriced_models: ['{MODEL}']\n"),
        );
        assert!(matches!(allowed.admit(&req("haiku", ProviderClass::Metered, None)).await, Admission::Allow));
    }

    #[tokio::test]
    async fn a_budget_only_binds_the_calls_in_its_scope() {
        let yaml = "budgets:\n  - {name: mem, scope: 'origin:memory', period: day, limit_usd: 0.5}\n";
        let g = gate(db_with_spend("memory", 5.0), yaml);
        // Origin 'chat' is outside the memory budget even though memory is over its limit.
        assert!(matches!(g.admit(&req("haiku", ProviderClass::Metered, Some(0.01))).await, Admission::Allow));
    }

    #[tokio::test]
    async fn a_downgrade_budget_moves_the_call_to_the_cheaper_model() {
        let yaml = "budgets:\n  - {name: d, scope: global, period: day, limit_usd: 1, action: downgrade, downgrade_model: cheap/model}\n";
        let g = gate(db_with_spend("chat", 2.0), yaml);
        match g.admit(&req("haiku", ProviderClass::Metered, Some(0.01))).await {
            Admission::Downgrade { model } => assert_eq!(model, "cheap/model"),
            _ => panic!("expected a downgrade"),
        }
    }

    #[tokio::test]
    async fn each_threshold_alerts_once_per_period() {
        let db = db_with_spend("chat", 9.0);
        let g = gate(db.clone(), MONTH_BLOCK);
        for _ in 0..3 {
            g.admit(&req("haiku", ProviderClass::Metered, Some(0.01))).await;
        }
        // The alert is written on the blocking pool; give it a moment to land.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let items = hq_db::value_items::list_by_state(&db, hq_core::types::ValueState::Pending).unwrap();
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles.len(), 1, "{titles:?}");
        assert!(titles[0].contains("80%"));
    }

    #[tokio::test]
    async fn an_unreadable_config_keeps_the_last_good_budgets_in_force() {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = reads.clone();
        let cfg: BudgetsConfig = serde_yaml::from_str(MONTH_BLOCK).unwrap();
        let g = LedgerBudgetGate::new(
            db_with_spend("chat", 50.0),
            Arc::new(move || {
                // The first read works; every later one fails.
                (counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0).then(|| cfg.clone())
            }),
        );
        assert!(matches!(g.admit(&req("haiku", ProviderClass::Metered, Some(0.01))).await, Admission::Deny(_)));
        // Force the next call to reload.
        g.cache().config = None;
        assert!(matches!(g.admit(&req("haiku", ProviderClass::Metered, Some(0.01))).await, Admission::Deny(_)));
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
