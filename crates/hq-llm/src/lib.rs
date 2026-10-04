//! LLM provider abstraction layer.

pub mod anthropic;
pub mod backend_chain;
pub mod cerebras;
pub mod copilot;
pub mod copilot_catalog;
pub mod copilot_burn;
pub mod copilot_usage;
pub mod decision;
pub mod decision_report;
pub mod http;
pub mod models;
pub mod ollama;
pub mod openai_compat;
pub mod outcome_sink;
pub mod prompted_tools;
/// Backward-compatible alias for the renamed module.
pub mod openrouter {
    pub use crate::openai_compat::*;
}
pub mod provider;
pub mod responses;
pub mod router;
#[cfg(feature = "turboquant")]
pub mod turboquant;

pub use anthropic::AnthropicProvider;
pub use cerebras::CerebrasProvider;
pub use copilot::CopilotProvider;
pub use ollama::OllamaProvider;
pub use openai_compat::OpenRouterProvider;
pub use outcome_sink::{
    OutcomeEvent, SESSION_CONTEXT, SessionContext, SharedSink, TaskOutcomeSink,
};
pub use provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};
pub use router::{CostTier, LlmRouter, TaskHint};
#[cfg(feature = "turboquant")]
pub use turboquant::TurboQuantProvider;
