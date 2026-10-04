//! The leaf-vs-agentic harness distinction.

/// Returns `true` if the harness name refers to an agentic harness (one that
/// can run multi-step tasks with tool use). Returns `false` for the known
/// pure-inference leaf providers. Unknown harnesses default to `true`.
pub fn is_agentic_harness(name: &str) -> bool {
    !matches!(name, "groq" | "cerebras")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_providers_are_not_agentic() {
        for name in ["groq", "cerebras"] {
            assert!(
                !is_agentic_harness(name),
                "leaf provider {name:?} should NOT be agentic"
            );
        }
    }

    #[test]
    fn agent_harnesses_and_unknowns_are_agentic() {
        let known_agentic = &[
            "antigravity",
            "claude-code",
            "cursor",
            "opencode",
            "hq",
            "native",
            "local",
            "gemini-cli",
            "kimi-cli",
            "deepseek-tui",
            "qwen-cli",
            "pi",
            "github-copilot",
            "codex",
            "fleet",
        ];
        for name in known_agentic {
            assert!(
                is_agentic_harness(name),
                "known agentic harness {name:?} should be agentic"
            );
        }
        // Unknown harnesses should also be treated as agentic (safe default).
        assert!(is_agentic_harness("unknown-harness"));
        assert!(is_agentic_harness(""));
    }
}
