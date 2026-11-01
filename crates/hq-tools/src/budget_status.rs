//! Lets the agent see its own budget, burn rate and what drives the spend, so it can choose to
//! spend less. Read-only.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use hq_core::config::HqConfig;
use hq_db::Database;
use serde_json::{Value, json};

use crate::registry::{HqTool, ToolPolicy};
use crate::usage_forecast::{breakdown, forecast_report};

/// Bounds for the optional breakdown, so one call cannot ask for an unbounded scan or list.
const MAX_BREAKDOWN_DAYS: i64 = 90;
const MAX_BREAKDOWN_ROWS: usize = 50;
const DEFAULT_BREAKDOWN_ROWS: usize = 20;

pub fn create_budget_status_tools(db: Arc<Database>) -> Vec<Box<dyn HqTool>> {
    vec![Box::new(BudgetStatusTool { db })]
}

pub struct BudgetStatusTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for BudgetStatusTool {
    fn name(&self) -> &str {
        "budget_status"
    }

    fn description(&self) -> &str {
        "Show the spending budgets, how much of each is used, the current burn rate, the projected \
         month-end spend, and which models and kinds of work drive it. Use it before starting \
         expensive work or when asked about cost."
    }

    fn category(&self) -> &str {
        "system"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("spend, budget, cost, burn rate, headroom, tokens used")
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn tool_policy(&self) -> ToolPolicy {
        ToolPolicy::Weak
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "breakdown_days": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_BREAKDOWN_DAYS,
                    "description": "Also return spend by every model and origin over this many days (the report's drivers cover only the last 7 days, top 3)."
                },
                "breakdown_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_BREAKDOWN_ROWS,
                    "description": "Rows per group in the breakdown (default 20)."
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let breakdown_days = args["breakdown_days"]
            .as_i64()
            .map(|d| d.clamp(1, MAX_BREAKDOWN_DAYS));
        let breakdown_limit = args["breakdown_limit"]
            .as_u64()
            .map_or(DEFAULT_BREAKDOWN_ROWS, |n| {
                (n as usize).clamp(1, MAX_BREAKDOWN_ROWS)
            });
        tokio::task::spawn_blocking(move || {
            let budgets = HqConfig::load().map(|c| c.budgets).unwrap_or_default();
            let now = chrono::Utc::now().timestamp();
            let statuses = budgets
                .enforceable()
                .into_iter()
                .map(|b| db.with_conn(|c| hq_db::usage_ledger::budget_status(c, b, now)))
                .collect::<Result<Vec<_>>>()?;
            let by_window = breakdown_days
                .map(|days| breakdown(&db, now, days, breakdown_limit).map(|d| json!({ "days": days, "groups": d })))
                .transpose()?;
            Ok(json!({
                "budgets": statuses,
                "breakdown": by_window,
                "report": forecast_report(&db, &budgets, now)?,
                "note": if budgets.budgets.is_empty() { "No budgets are configured, so nothing is limited." } else { "" },
            }))
        })
        .await?
    }
}
