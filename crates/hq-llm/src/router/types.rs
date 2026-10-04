use serde::{Deserialize, Serialize};

use crate::provider::ChatRequest;

/// Size of the sliding window for per-task-type reliability tracking.
/// Only the last RELIABILITY_WINDOW outcomes per task type affect the reliability score.
pub(crate) const RELIABILITY_WINDOW: usize = 20;

/// How much a provider/model costs per token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CostTier {
    /// Completely free (Cerebras, Groq free tier, OpenRouter :free models)
    Free = 0,
    /// Very cheap (<$1/M tokens): DeepSeek, Gemini Flash
    Budget = 1,
    /// Mid-range ($1-5/M tokens): Kimi K2.5, paid OpenRouter models
    Standard = 2,
    /// Expensive (>$5/M tokens): GPT-4, Claude
    Premium = 3,
}

/// What kind of task is being routed. Derived from request characteristics.
/// WARNING: The discriminant of this enum is used to index into the `task_window` array in `ProviderHealth` (size 6).
/// Do not reorder or insert variants without updating the array size and Default impls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum TaskHint {
    /// Simple chat, no tools
    Simple = 0,
    /// Needs function/tool calling
    ToolUse = 1,
    /// Needs high-quality code generation
    Coding = 2,
    /// Needs reasoning/planning quality
    Planning = 3,
    /// High volume, optimize for throughput/cost
    Bulk = 4,
    /// Low-priority notification generation. Must use cheapest inference.
    Notification = 5,
}

impl TaskHint {
    /// Infer task type from the request characteristics.
    pub fn from_request(request: &ChatRequest) -> Self {
        let model = request.model.as_str();
        match model {
            "fast" | "bulk" | "verify" | "critic" => return TaskHint::Bulk,
            "plan" => return TaskHint::Planning,
            "code" => return TaskHint::Coding,
            "relay" | "premium" => return TaskHint::ToolUse,
            "notification" | "nudge" => return TaskHint::Notification,
            _ => {}
        }

        if !request.tools.is_empty() {
            TaskHint::ToolUse
        } else {
            TaskHint::Simple
        }
    }

    /// Stable string label used for DB storage and CLI filters.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskHint::Simple => "simple",
            TaskHint::ToolUse => "tool_use",
            TaskHint::Coding => "coding",
            TaskHint::Planning => "planning",
            TaskHint::Bulk => "bulk",
            TaskHint::Notification => "notification",
        }
    }

    /// Inverse of `as_str`. Returns `TaskHint::Simple` for unrecognized input.
    pub fn parse(s: &str) -> Self {
        match s {
            "tool_use" => TaskHint::ToolUse,
            "coding" => TaskHint::Coding,
            "planning" => TaskHint::Planning,
            "bulk" => TaskHint::Bulk,
            "notification" => TaskHint::Notification,
            _ => TaskHint::Simple,
        }
    }

    /// All task hints, in enum-discriminant order. Useful for iteration in
    /// aggregation and leaderboard queries.
    pub fn all() -> &'static [TaskHint] {
        &[
            TaskHint::Simple,
            TaskHint::ToolUse,
            TaskHint::Coding,
            TaskHint::Planning,
            TaskHint::Bulk,
            TaskHint::Notification,
        ]
    }
}

/// Model routing entry: maps a model pattern to a specific provider + model ID.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    /// Pattern to match (e.g. "cerebras/*", "groq/*", "fast", "bulk")
    pub pattern: String,
    /// Provider name (must match a registered provider)
    pub provider: String,
    /// Actual model ID to send to the provider
    pub model_id: String,
    /// Cost tier for this route
    pub cost_tier: CostTier,
    /// Whether this is a local inference provider (Ollama, TurboQuant).
    /// Local providers are "free" but slow; cloud free providers are preferred.
    pub is_local: bool,
}
