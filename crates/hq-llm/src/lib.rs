//! LLM provider abstraction layer.

pub mod anthropic;
pub mod backend_chain;
pub mod budget;
pub mod cerebras;
pub mod copilot;
pub mod copilot_burn;
pub mod copilot_catalog;
pub mod copilot_usage;
pub mod cost;
pub mod decision;
pub mod decision_report;
pub mod forecast;
pub mod http;
pub mod instrument;
pub mod models;
pub mod ollama;
pub mod openai_compat;
pub mod openrouter_usage;
pub mod outcome_sink;
pub mod prompted_tools;
/// Backward-compatible alias for the renamed module.
pub mod openrouter {
    pub use crate::openai_compat::*;
}
pub mod provider;
pub mod provider_usage;
pub mod ratelimit;
pub mod reconcile;
mod tap;
pub mod responses;
pub mod router;
#[cfg(feature = "turboquant")]
pub mod turboquant;

pub use anthropic::AnthropicProvider;
pub use instrument::{ExternalCall, Instruments, InstrumentedProvider, usage_from_openrouter};
pub use cerebras::CerebrasProvider;
pub use copilot::CopilotProvider;
pub use ollama::OllamaProvider;
pub use openai_compat::OpenRouterProvider;
pub use outcome_sink::{
    OutcomeEvent, SESSION_CONTEXT, SessionContext, SharedSink, TaskOutcomeSink, origin,
    unscoped_calls, with_default_origin, with_origin,
};
pub use provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};
pub use router::{CostTier, LlmRouter, TaskHint};
#[cfg(feature = "turboquant")]
pub use turboquant::TurboQuantProvider;
