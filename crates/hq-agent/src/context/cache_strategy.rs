//! Tokenizer and prompt-cache capabilities of the target LLM.

/// Tokenizer dictionaries supported by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TokenizerDict {
    Cl100kBase, // OpenAI
    O200kBase,  // Omni
    Llama3,     // Local/Meta
    #[default]
    Claude, // Anthropic
}

/// LLM engine capabilities used to drive context assembly.
#[derive(Debug, Clone)]
pub struct LlmCapabilities {
    pub supports_prompt_caching: bool,
    pub exact_context_window: usize,
    pub requires_explicit_breakpoints: bool,
    pub tokenizer: TokenizerDict,
}

impl Default for LlmCapabilities {
    fn default() -> Self {
        Self {
            supports_prompt_caching: false,
            exact_context_window: 128_000,
            requires_explicit_breakpoints: false,
            tokenizer: TokenizerDict::Cl100kBase,
        }
    }
}

impl LlmCapabilities {
    pub fn new(
        supports_prompt_caching: bool,
        exact_context_window: usize,
        requires_explicit_breakpoints: bool,
        tokenizer: TokenizerDict,
    ) -> Self {
        Self {
            supports_prompt_caching,
            exact_context_window,
            requires_explicit_breakpoints,
            tokenizer,
        }
    }
}
