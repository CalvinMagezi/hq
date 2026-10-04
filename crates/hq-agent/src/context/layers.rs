//! Layer types for context assembly.

use hq_core::types::{Note, SearchResult};

/// A simple conversation message (role + content).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
}

/// The six context layers, ordered by assembly priority.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContextLayer {
    /// Tokens reserved for the LLM response (never filled by us).
    ResponseReserve,
    /// SOUL + harness instructions.
    System,
    /// The current user message.
    UserMessage,
    /// Long-term memory facts.
    Memory,
    /// Recent conversation thread.
    Thread,
    /// Pinned notes, search results, progressive disclosure.
    Injections,
}

impl ContextLayer {
    /// Budget key used in `TokenBudget.layers`.
    pub fn key(&self) -> &'static str {
        match self {
            Self::ResponseReserve => "response",
            Self::System => "system",
            Self::UserMessage => "user_message",
            Self::Memory => "memory",
            Self::Thread => "thread",
            Self::Injections => "injections",
        }
    }
}

/// Everything needed to build a single context frame.
#[derive(Debug, Clone)]
pub struct FrameInput {
    /// Budget profile name: "quick", "standard", "thorough", "delegation".
    pub profile: String,

    /// Total token budget for the model context window.
    pub total_tokens: usize,

    /// SOUL identity text.
    pub soul: String,

    /// Harness-specific instructions.
    pub harness_instructions: String,

    /// The current user message.
    pub user_message: String,

    /// Long-term memory text (from MEMORY.md / facts).
    pub memory: String,

    /// Private tags to strip from memory before injection.
    pub private_tags: Vec<String>,

    /// Recent conversation history.
    pub thread: Vec<ConversationMessage>,

    /// Pinned vault notes.
    pub pinned_notes: Vec<Note>,

    /// Search results to inject.
    pub search_results: Vec<SearchResult>,

    /// Entities/topics extracted from the user message for attention-weighted
    /// memory retrieval.
    /// When non-empty, memory retrieval prioritizes memories matching these cues.
    pub query_entities: Vec<String>,
}
