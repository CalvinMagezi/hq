use std::collections::HashSet;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Soft alert thresholds used when a budget names none.
const DEFAULT_SOFT_PCT: &[u8] = &[80];
const MAX_PCT: u8 = 100;

/// What a budget counts: every call, or the calls of one provider, model or origin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BudgetScope {
    Global,
    /// A configured backend name (`haiku`) or provider name (`openrouter`).
    Provider(String),
    Model(String),
    /// What kind of work the call served: `chat`, `memory`, `subagent`, `background`...
    Origin(String),
}

impl FromStr for BudgetScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s == "global" {
            return Ok(BudgetScope::Global);
        }
        let (kind, value) = s
            .split_once(':')
            .map(|(k, v)| (k.trim(), v.trim()))
            .filter(|(_, v)| !v.is_empty())
            .ok_or_else(|| {
                format!("scope '{s}' must be 'global' or 'provider:<name>', 'model:<id>', 'origin:<name>'")
            })?;
        match kind {
            "provider" => Ok(BudgetScope::Provider(value.to_string())),
            "model" => Ok(BudgetScope::Model(value.to_string())),
            "origin" => Ok(BudgetScope::Origin(value.to_string())),
            other => Err(format!("unknown budget scope kind '{other}'")),
        }
    }
}

impl std::fmt::Display for BudgetScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetScope::Global => write!(f, "global"),
            BudgetScope::Provider(v) => write!(f, "provider:{v}"),
            BudgetScope::Model(v) => write!(f, "model:{v}"),
            BudgetScope::Origin(v) => write!(f, "origin:{v}"),
        }
    }
}

impl Serialize for BudgetScope {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for BudgetScope {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    Day,
    Week,
    Month,
}

/// What happens to a call that would push spend past the limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAction {
    /// Refuse the call with a clear error.
    #[default]
    Block,
    /// Send the call to `downgrade_model` instead.
    Downgrade,
    /// Let the call through and raise the alert.
    Notify,
}

/// One spending limit. Nothing is enforced until the owner configures at least one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub name: String,
    pub scope: BudgetScope,
    pub period: BudgetPeriod,
    pub limit_usd: f64,
    /// Percentages of the limit at which an alert is raised, once per period each.
    #[serde(default = "default_soft_pct")]
    pub soft_pct: Vec<u8>,
    #[serde(default)]
    pub action: BudgetAction,
    /// Required when `action` is `downgrade`: the cheaper model that takes over.
    #[serde(default)]
    pub downgrade_model: Option<String>,
}

fn default_soft_pct() -> Vec<u8> {
    DEFAULT_SOFT_PCT.to_vec()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BudgetsConfig {
    #[serde(default)]
    pub budgets: Vec<Budget>,
    /// Ceiling in USD for one background or watch run, and so for the sub-agents it starts, lowering
    /// any higher cap the run has, so a runaway loop cannot drain a month. Unset means no ceiling.
    #[serde(default)]
    pub background_run_usd: Option<f64>,
    /// Add a one-line budget note to the system prompt once a budget is nearly used up, so the model
    /// can spend less. Off by default: whether it lowers cost without hurting results is unmeasured.
    #[serde(default)]
    pub guidance: bool,
    /// Models allowed to run under a blocking budget although HQ has no price for them.
    #[serde(default)]
    pub allow_unpriced_models: Vec<String>,
}

impl BudgetsConfig {
    /// The budgets that are safe to enforce: a malformed one (non-positive or non-finite limit,
    /// empty or repeated name, downgrade without a target) is left out rather than enforced wrongly.
    pub fn enforceable(&self) -> Vec<&Budget> {
        let mut seen = HashSet::new();
        self.budgets
            .iter()
            .filter(|b| {
                let has_model = b.downgrade_model.as_deref().is_some_and(|m| !m.trim().is_empty());
                !b.name.trim().is_empty()
                    && seen.insert(b.name.clone())
                    && b.limit_usd.is_finite()
                    && b.limit_usd > 0.0
                    && b.soft_pct.iter().all(|p| (1..=MAX_PCT).contains(p))
                    && (b.action != BudgetAction::Downgrade || has_model)
            })
            .collect()
    }

    /// Every problem with the configuration, so the owner sees all of them at once.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for b in &self.budgets {
            let name = &b.name;
            if name.trim().is_empty() {
                out.push("a budget has no name".to_string());
            } else if !seen.insert(name.clone()) {
                out.push(format!("budget '{name}' is defined more than once"));
            }
            if !b.limit_usd.is_finite() || b.limit_usd <= 0.0 {
                out.push(format!("budget '{name}': limit_usd must be a positive number"));
            }
            if b.soft_pct.iter().any(|p| *p == 0 || *p > MAX_PCT) {
                out.push(format!("budget '{name}': soft_pct values must be 1 to {MAX_PCT}"));
            }
            let has_model = b.downgrade_model.as_deref().is_some_and(|m| !m.trim().is_empty());
            if b.action == BudgetAction::Downgrade && !has_model {
                out.push(format!("budget '{name}': action downgrade needs downgrade_model"));
            }
        }
        if let Some(v) = self.background_run_usd
            && (!v.is_finite() || v <= 0.0)
        {
            out.push("background_run_usd must be a positive number".to_string());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> BudgetsConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn nothing_is_configured_by_default() {
        let c = parse("{}");
        assert!(c.budgets.is_empty() && c.background_run_usd.is_none());
        assert!(c.problems().is_empty());
    }

    #[test]
    fn scopes_round_trip_through_their_string_form() {
        for s in ["global", "provider:haiku", "model:anthropic/claude-haiku-5.5", "origin:memory"] {
            assert_eq!(s.parse::<BudgetScope>().unwrap().to_string(), s);
        }
        assert!("provider:".parse::<BudgetScope>().is_err());
        assert!("team:x".parse::<BudgetScope>().is_err());
    }

    #[test]
    fn a_budget_defaults_to_blocking_with_an_eighty_percent_alert() {
        let c = parse(
            "budgets:\n  - name: month\n    scope: global\n    period: month\n    limit_usd: 20\n",
        );
        assert_eq!(c.budgets[0].action, BudgetAction::Block);
        assert_eq!(c.budgets[0].soft_pct, vec![80]);
        assert!(c.problems().is_empty());
    }

    #[test]
    fn a_malformed_budget_is_left_out_of_enforcement() {
        let c = parse(
            "budgets:\n  - {name: bad, scope: global, period: day, limit_usd: 0}\n  \
             - {name: ok, scope: global, period: day, limit_usd: 5}\n  \
             - {name: ok, scope: global, period: day, limit_usd: 9}\n",
        );
        let names: Vec<&str> = c.enforceable().iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["ok"]);
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let c = parse(
            "budgets:\n  - {name: a, scope: global, period: day, limit_usd: 0}\n  \
             - {name: a, scope: global, period: day, limit_usd: 5, soft_pct: [0, 150]}\n  \
             - {name: b, scope: global, period: day, limit_usd: 5, action: downgrade}\n\
             background_run_usd: -1\n",
        );
        let problems = c.problems().join("\n");
        for needle in [
            "limit_usd must be a positive number",
            "defined more than once",
            "soft_pct values must be 1 to 100",
            "needs downgrade_model",
            "background_run_usd must be a positive number",
        ] {
            assert!(problems.contains(needle), "{needle}: {problems}");
        }
    }
}
