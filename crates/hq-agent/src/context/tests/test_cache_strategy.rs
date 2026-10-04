use crate::context::cache_strategy::{LlmCapabilities, TokenizerDict};

#[test]
fn test_provider_agnostic_struct() {
    let caps = LlmCapabilities {
        supports_prompt_caching: true,
        exact_context_window: 128000,
        requires_explicit_breakpoints: false,
        tokenizer: TokenizerDict::Claude,
    };
    assert_eq!(caps.exact_context_window, 128000);
}
