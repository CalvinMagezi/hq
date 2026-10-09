//! Lets the agent see its own budget, burn rate and what drives the spend, so it can choose to
//! spend less. Read-only.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use hq_core::config::HqConfig;
use hq_db::Database;
use serde_json::{Value, json};

use crate::registry::{HqTool, ToolPolicy};
use crate::usage_forecast::forecast_report;

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
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let budgets = HqConfig::load().map(|c| c.budgets).unwrap_or_default();
            let now = chrono::Utc::now().timestamp();
            let statuses = budgets
                .enforceable()
                .into_iter()
                .map(|b| db.with_conn(|c| hq_db::usage_ledger::budget_status(c, b, now)))
                .collect::<Result<Vec<_>>>()?;
            Ok(json!({
                "budgets": statuses,
                "report": forecast_report(&db, &budgets, now)?,
                "note": if budgets.budgets.is_empty() { "No budgets are configured, so nothing is limited." } else { "" },
            }))
        })
        .await?
    }
}
