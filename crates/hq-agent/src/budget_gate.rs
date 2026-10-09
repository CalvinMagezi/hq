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

type ConfigLoader = Arc<dyn Fn() -> BudgetsConfig + Send + Sync>;

struct Cache {
    config: Option<(Instant, BudgetsConfig)>,
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
                status: HashMap::new(),
                alerted: HashSet::new(),
            }),
        })
    }

    fn budgets(&self) -> BudgetsConfig {
        let mut cache = self.cache.lock().unwrap();
        if let Some((at, cfg)) = &cache.config
            && at.elapsed() < CONFIG_TTL
        {
            return cfg.clone();
        }
        let cfg = (self.load)();
        for problem in cfg.problems() {
            warn!(%problem, "budget configuration problem; the budget is not enforced as written");
        }
        cache.config = Some((Instant::now(), cfg.clone()));
        cfg
    }

    fn status(&self, budget: &Budget, now: i64) -> Option<BudgetStatus> {
        let key = format!("{}|{}", budget.name, budget.scope);
        if let Some((at, s)) = self.cache.lock().unwrap().status.get(&key)
            && at.elapsed() < SPEND_TTL
            && s.period_start <= now
            && now < s.resets_at
        {
            return Some(s.clone());
        }
        let status = self
            .db
            .with_conn(|conn| budget_status(conn, budget, now))
            .map_err(|e| warn!(error = %e, budget = %budget.name, "budget spend could not be read"))
            .ok()?;
        self.cache
            .lock()
            .unwrap()
            .status
            .insert(key, (Instant::now(), status.clone()));
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
            if let Err(e) = hq_db::value_items::emit(&self.db, &item) {
                warn!(error = %e, "budget alert could not be recorded");
            }
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
        for budget in cfg.budgets.iter().filter(|b| covers(&b.scope, req)) {
            let Some(status) = self.status(budget, now) else {
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
        LedgerBudgetGate::new(db, Arc::new(move || cfg.clone()))
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
        let items = hq_db::value_items::list_by_state(&db, hq_core::types::ValueState::Pending).unwrap();
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles.len(), 1, "{titles:?}");
        assert!(titles[0].contains("80%"));
    }
}
