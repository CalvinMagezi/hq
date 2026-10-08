//! Shared helpers for the `start` command subsystem.

/// Expand a short model name typed at `hq chat` to an OpenRouter model id.
/// Anything else, including a full id, passes through lowercased. Ids checked
/// against OpenRouter's model list on 2026-09-26 (haiku on 2026-10-09).
pub fn resolve_model_alias(name: &str) -> String {
    match name.to_lowercase().as_str() {
        "opus" => "anthropic/claude-opus-5.5".to_string(),
        "sonnet" => "anthropic/claude-sonnet-5".to_string(),
        "haiku" => "anthropic/claude-haiku-5.5".to_string(),
        "gemini" | "flash" => "google/gemini-3.8-flash".to_string(),
        "kimi" => "moonshotai/kimi-k3".to_string(),
        "qwen" => "qwen/qwen3.8-flash".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_model_alias;

    #[test]
    fn aliases_expand_to_openrouter_ids_and_full_ids_pass_through() {
        for alias in ["opus", "sonnet", "haiku", "gemini", "flash", "kimi", "qwen"] {
            let id = resolve_model_alias(alias);
            assert!(id.contains('/') && !id.contains(' '), "{alias} -> {id}");
        }
        assert_eq!(resolve_model_alias("Anthropic/Claude-Opus-5"), "anthropic/claude-opus-5");
        assert_eq!(resolve_model_alias("haiku"), "anthropic/claude-haiku-5.5");
    }
}
