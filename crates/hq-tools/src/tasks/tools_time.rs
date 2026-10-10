//! `task_time_report`: where the time went, by initiative and by agent.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::config::TasksConfig;
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::registry::HqTool;

/// Window when the caller names none.
const DEFAULT_REPORT_DAYS: i64 = 30;
/// Longest window: a year.
const MAX_REPORT_DAYS: i64 = 365;

pub(super) struct TaskTimeReportTool {
    pub(super) settings: TasksConfig,
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskTimeReportTool {
    fn name(&self) -> &str {
        "task_time_report"
    }
    fn description(&self) -> &str {
        "Where the time went over the last N days: leased hours, completed tasks, mean cycle time and \
         estimate accuracy per initiative, and leased hours per agent. A task with no recorded start \
         is counted as unknown, never guessed. Time comes from work leases, so only tasks claimed \
         with task_claim (or run by an HQ session) have any."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "days": { "type": "integer", "minimum": 1, "maximum": MAX_REPORT_DAYS, "description": "Window in days, default 30" }
            }
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let days = args
            .get("days")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_REPORT_DAYS)
            .clamp(1, MAX_REPORT_DAYS);
        let ttl = super::tools_lease::ttl_secs(&self.settings);
        let report = self.db.with_conn(move |c| t::time_report(c, days, ttl))?;
        Ok(json!({ "days": days, "report": report }))
    }
}
