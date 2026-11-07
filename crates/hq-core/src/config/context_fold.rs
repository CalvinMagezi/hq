use serde::{Deserialize, Serialize};

/// How a long chat thread keeps its history (`docs/plans/context-fold.md`).
///
/// A thread's newest messages go to the model as they are (the tail). When the tail grows past
/// twice its target, its oldest part is summarized into one fold block, kept beside the original
/// messages, and sent instead of them from then on. Earlier blocks are never rewritten, so what
/// the model sees grows only at the end and old detail is summarized once, not once per round.
///
/// On by default. Off, a thread sends only its newest messages, as before.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFoldConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// The tail a fold leaves behind, in estimated tokens. A fold runs once the tail passes
    /// twice this, or twice `tail_messages`.
    #[serde(default = "default_tail_tokens")]
    pub tail_tokens: usize,

    /// The tail a fold leaves behind, in messages.
    #[serde(default = "default_tail_messages")]
    pub tail_messages: usize,

    /// When a thread's fold blocks together pass this many estimated tokens, the oldest of them
    /// are distilled into one block.
    #[serde(default = "default_blocks_budget_tokens")]
    pub blocks_budget_tokens: usize,

    /// The model (or alias) that writes fold summaries. A cheap one is enough.
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_enabled() -> bool {
    true
}

fn default_tail_tokens() -> usize {
    6_000
}

fn default_tail_messages() -> usize {
    16
}

fn default_blocks_budget_tokens() -> usize {
    6_000
}

fn default_model() -> String {
    "bulk".to_string()
}

impl Default for ContextFoldConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            tail_tokens: default_tail_tokens(),
            tail_messages: default_tail_messages(),
            blocks_budget_tokens: default_blocks_budget_tokens(),
            model: default_model(),
        }
    }
}

impl ContextFoldConfig {
    /// The tail target in messages, never below 4 so a fold always leaves a real exchange.
    pub fn target_messages(&self) -> usize {
        self.tail_messages.max(4)
    }

    /// The tail target in tokens, never below 1,000.
    pub fn target_tokens(&self) -> usize {
        self.tail_tokens.max(1_000)
    }

    /// The fold-block budget in tokens, never below 1,000.
    pub fn blocks_budget(&self) -> usize {
        self.blocks_budget_tokens.max(1_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_hold_when_the_section_is_empty() {
        let cfg: ContextFoldConfig = serde_yaml::from_str("{}").unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.target_messages(), 16);
        assert_eq!(cfg.model, "bulk");
    }

    #[test]
    fn tiny_values_are_raised_to_a_floor() {
        let cfg = ContextFoldConfig {
            tail_messages: 0,
            tail_tokens: 1,
            blocks_budget_tokens: 0,
            ..Default::default()
        };
        assert_eq!(cfg.target_messages(), 4);
        assert_eq!(cfg.target_tokens(), 1_000);
        assert_eq!(cfg.blocks_budget(), 1_000);
    }
}
