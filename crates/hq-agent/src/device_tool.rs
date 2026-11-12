//! Every device tool as an agent tool. The wrapper exists because the tool needs the model's tool
//! call id, which the generic adapter drops; what the tool is comes from its `DeviceToolSpec`.

use std::sync::Arc;

use anyhow::Result;
use hq_core::types::{ToolResult, ToolResultContent};
use hq_tools::device_actions::{DeviceLink, DeviceToolSpec, RiskClass};
use serde_json::Value;

use crate::tools::AgentTool;

/// The category a tool offered only to an owner's phone chat belongs to.
const CATEGORY: &str = "device";

pub struct DeviceActionTool {
    spec: &'static DeviceToolSpec,
    link: Arc<DeviceLink>,
    /// The base text plus the phone's zone and date, so the model never guesses "today".
    description: String,
}

impl DeviceActionTool {
    pub fn new(spec: &'static DeviceToolSpec, link: Arc<DeviceLink>) -> Self {
        let description = format!("{} {}", spec.description, link.context_line());
        Self {
            spec,
            link,
            description,
        }
    }
}

#[async_trait::async_trait]
impl AgentTool for DeviceActionTool {
    fn name(&self) -> &str {
        self.spec.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        (self.spec.parameters)()
    }
    fn category(&self) -> &str {
        CATEGORY
    }
    fn is_read_only(&self) -> bool {
        self.spec.risk == RiskClass::Read
    }
    fn requires_live_user_turn(&self) -> bool {
        true
    }
    async fn execute(&self, id: &str, args: Value) -> Result<ToolResult> {
        let outcome = (self.spec.run)(&self.link, id, &args).await;
        Ok(ToolResult {
            content: vec![ToolResultContent {
                r#type: "text".into(),
                text: outcome.to_string(),
            }],
            details: None,
            context_modifier: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_tools::device_actions::{DeviceActionStore, PhoneClock};
    use serde_json::json;

    const SNAPSHOT: &str = include_str!("../tests/fixtures/device_tool_definitions.json");
    const SNAPSHOT_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/device_tool_definitions.json"
    );

    fn definitions() -> Value {
        let phone = PhoneClock::parse("Europe/London", "2026-01-10T12:00:00Z").unwrap();
        let link = Arc::new(DeviceLink::new(
            "thread-1",
            Arc::new(DeviceActionStore::default()),
            Arc::new(|_| {}),
            phone,
        ));
        let tools: Vec<DeviceActionTool> = hq_tools::device_actions::offered()
            .map(|spec| DeviceActionTool::new(spec, link.clone()))
            .collect();
        Value::Array(
            tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters(),
                        "category": t.category(),
                        "read_only": t.is_read_only(),
                        "live_user_turn": t.requires_live_user_turn(),
                    })
                })
                .collect(),
        )
    }

    #[test]
    fn the_four_tool_definitions_match_the_snapshot_taken_from_main() {
        let now = serde_json::to_string_pretty(&definitions()).unwrap();
        if std::env::var_os("HQ_WRITE_SNAPSHOT").is_some() {
            std::fs::write(SNAPSHOT_PATH, format!("{now}\n")).unwrap();
            return;
        }
        assert_eq!(now.trim(), SNAPSHOT.trim());
    }
}
