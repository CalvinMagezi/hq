//! `calendar_create_event` as an agent tool. The logic is `hq_tools::calendar`; this wrapper
//! exists because the tool needs the model's tool call id, which the generic adapter drops.

use std::sync::Arc;

use anyhow::Result;
use hq_core::types::{ToolResult, ToolResultContent};
use hq_tools::calendar::DeviceCalendar;
use serde_json::Value;

use crate::tools::AgentTool;

/// The category a tool offered only to an owner's phone chat belongs to.
const CATEGORY: &str = "device";

pub struct CalendarCreateEventTool {
    device: Arc<DeviceCalendar>,
    /// The base text plus the phone's zone and date, so the model never guesses "today".
    description: String,
}

impl CalendarCreateEventTool {
    pub fn new(device: Arc<DeviceCalendar>) -> Self {
        let description = format!("{} {}", device.description(), device.context_line());
        Self {
            device,
            description,
        }
    }
}

#[async_trait::async_trait]
impl AgentTool for CalendarCreateEventTool {
    fn name(&self) -> &str {
        self.device.name()
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        self.device.parameters()
    }
    fn category(&self) -> &str {
        CATEGORY
    }
    fn is_read_only(&self) -> bool {
        false
    }
    fn requires_live_user_turn(&self) -> bool {
        true
    }
    async fn execute(&self, id: &str, args: Value) -> Result<ToolResult> {
        let outcome = self.device.create(id, &args).await;
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
