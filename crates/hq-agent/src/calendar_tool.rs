//! The calendar tools as agent tools. The logic is `hq_tools::calendar`; this wrapper
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

/// `calendar_list_events`: reads the owner's phone calendar through the same link.
pub struct CalendarListEventsTool {
    device: Arc<DeviceCalendar>,
    description: String,
}

impl CalendarListEventsTool {
    pub fn new(device: Arc<DeviceCalendar>) -> Self {
        let description = format!("{} {}", device.list_description(), device.context_line());
        Self {
            device,
            description,
        }
    }
}

#[async_trait::async_trait]
impl AgentTool for CalendarListEventsTool {
    fn name(&self) -> &str {
        self.device.list_name()
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        self.device.list_parameters()
    }
    fn category(&self) -> &str {
        CATEGORY
    }
    fn is_read_only(&self) -> bool {
        true
    }
    fn requires_live_user_turn(&self) -> bool {
        true
    }
    async fn execute(&self, id: &str, args: Value) -> Result<ToolResult> {
        let outcome = self.device.list(id, &args).await;
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

/// `calendar_update_event` through the same link.
pub struct CalendarUpdateEventTool {
    device: Arc<DeviceCalendar>,
    description: String,
}

impl CalendarUpdateEventTool {
    pub fn new(device: Arc<DeviceCalendar>) -> Self {
        let description = format!("{} {}", device.update_description(), device.context_line());
        Self {
            device,
            description,
        }
    }
}

#[async_trait::async_trait]
impl AgentTool for CalendarUpdateEventTool {
    fn name(&self) -> &str {
        self.device.update_name()
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        self.device.update_parameters()
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
        let outcome = self.device.update(id, &args).await;
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

/// `calendar_delete_event` through the same link.
pub struct CalendarDeleteEventTool {
    device: Arc<DeviceCalendar>,
    description: String,
}

impl CalendarDeleteEventTool {
    pub fn new(device: Arc<DeviceCalendar>) -> Self {
        let description = format!("{} {}", device.delete_description(), device.context_line());
        Self {
            device,
            description,
        }
    }
}

#[async_trait::async_trait]
impl AgentTool for CalendarDeleteEventTool {
    fn name(&self) -> &str {
        self.device.delete_name()
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        self.device.delete_parameters()
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
        let outcome = self.device.delete(id, &args).await;
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
