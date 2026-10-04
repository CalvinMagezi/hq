use hq_core::types::MessageRole;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct BlockMetadata {
    pub id: String,
    pub priority: f32,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextBlock {
    pub role: MessageRole,
    pub content: Arc<str>,
    pub cache_breakpoint: bool,
    pub metadata: BlockMetadata,
}

impl ContextBlock {
    pub fn new(role: MessageRole, content: impl Into<Arc<str>>) -> Self {
        Self {
            role,
            content: content.into(),
            cache_breakpoint: false,
            metadata: BlockMetadata {
                id: uuid::Uuid::new_v4().to_string(),
                priority: 1.0,
                source: None,
            },
        }
    }
}

pub struct TelemetryManifest {
    pub total_budget: usize,
    pub budget_used: usize,
    pub dropped_items_by_id: Vec<String>,
    pub reducers_applied: Vec<String>,
}
