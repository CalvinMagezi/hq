//! The modern candidate-driven context assembly engine.

use super::block::{ContextBlock, TelemetryManifest};
use super::bpe::count_tokens_deterministic;
use super::budget::{ContextCandidate, allocate_knapsack, layer_priority};
use super::cache_strategy::LlmCapabilities;
use super::layers::FrameInput;
use super::reducer::{
    ContextError, ContextReducer, DeduplicationReducer, SemanticReducer, WhitespaceReducer,
};
use super::trust;
use hq_core::types::MessageRole;
use std::sync::Arc;

pub struct ContextEngine;

impl Default for ContextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextEngine {
    pub fn new() -> Self {
        Self
    }

    /// Build a fully token-budgeted context utilizing Priority-Based Knapsack allocation.
    pub async fn build_context(
        &self,
        input: FrameInput,
        capabilities: LlmCapabilities,
    ) -> Result<(Vec<ContextBlock>, TelemetryManifest), ContextError> {
        let dict = capabilities.tokenizer;
        let total_budget = capabilities.exact_context_window;
        let mut reducers_applied = Vec::new();

        // Step 2: Prepare Candidates
        let mut candidates = Vec::new();

        // System
        candidates.push(ContextCandidate {
            id: "system_soul".to_string(),
            content: Arc::from(input.soul),
            role: MessageRole::System,
            base_priority: layer_priority("system"),
            source: None,
            token_count_hint: None,
        });

        // Harness instructions — environment, machine profile, tool usage
        // notes, and whatever the calling surface appended.
        //
        // This field was populated by every caller and never read, so the
        // whole harness block was silently dropped on this path. It shares
        // the "system" priority with the soul: both describe who the agent is
        // and what it can do, and neither is worth having half of.
        if !input.harness_instructions.is_empty() {
            candidates.push(ContextCandidate {
                id: "harness_instructions".to_string(),
                content: Arc::from(input.harness_instructions),
                role: MessageRole::System,
                base_priority: layer_priority("system"),
                source: None,
                token_count_hint: None,
            });
        }

        // User Message
        candidates.push(ContextCandidate {
            id: "user_query".to_string(),
            content: Arc::from(input.user_message),
            role: MessageRole::User,
            base_priority: layer_priority("user_message"),
            source: None,
            token_count_hint: None,
        });

        // Memory (Reduced if exceeding 2000 tokens)
        if !input.memory.is_empty() {
            let (content, _) = self
                .apply_pipeline(
                    "memory",
                    Arc::from(input.memory),
                    2000,
                    dict,
                    &mut reducers_applied,
                )
                .await;
            let content = trust::wrap_untrusted("vault-memory", &content);
            candidates.push(ContextCandidate {
                id: "long_term_memory".to_string(),
                content: Arc::from(content),
                role: MessageRole::System,
                base_priority: layer_priority("memory"),
                source: None,
                token_count_hint: None,
            });
        }

        // Thread (Recent messages)
        for (i, msg) in input.thread.iter().enumerate() {
            let role = match msg.role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                _ => MessageRole::System,
            };
            candidates.push(ContextCandidate {
                id: format!("thread_{}", i),
                content: Arc::from(msg.content.clone()),
                role,
                base_priority: layer_priority("thread"),
                source: None,
                token_count_hint: None,
            });
        }

        // Injections (Pinned notes & Search results - reduced if exceeding 1500 tokens)
        for (i, note) in input.pinned_notes.iter().enumerate() {
            let id = format!("pinned_{}", i);
            let (content, _) = self
                .apply_pipeline(
                    &id,
                    Arc::from(note.content.clone()),
                    1500,
                    dict,
                    &mut reducers_applied,
                )
                .await;
            let content = trust::wrap_untrusted(&format!("vault-note: {}", note.path), &content);
            candidates.push(ContextCandidate {
                id,
                content: Arc::from(content),
                role: MessageRole::System,
                base_priority: layer_priority("injections"),
                source: Some(note.path.clone()),
                token_count_hint: None,
            });
        }

        for (i, result) in input.search_results.iter().enumerate() {
            let id = format!("search_{}", i);
            let (content, _) = self
                .apply_pipeline(
                    &id,
                    Arc::from(result.snippet.clone()),
                    1500,
                    dict,
                    &mut reducers_applied,
                )
                .await;
            let content =
                trust::wrap_untrusted(&format!("web-search: {}", result.note_path), &content);
            candidates.push(ContextCandidate {
                id,
                content: Arc::from(content),
                role: MessageRole::System,
                base_priority: layer_priority("injections"),
                source: Some(result.note_path.clone()),
                token_count_hint: None,
            });
        }

        // Step 3: Allocate using Knapsack
        let (mut selected, dropped, elastic_reducers) =
            allocate_knapsack(candidates, total_budget, dict).await;
        reducers_applied.extend(elastic_reducers);

        // Step 4: Finalize and return manifest
        let budget_used = selected
            .iter()
            .map(|b| count_tokens_deterministic(&b.content, dict))
            .sum();

        // Optional: Mark soul as cacheable if provider supports it
        if let Some(soul) = selected
            .iter_mut()
            .find(|b| b.metadata.id == "system_soul")
            .filter(|_| capabilities.supports_prompt_caching)
        {
            soul.cache_breakpoint = true;
        }

        let manifest = TelemetryManifest {
            total_budget,
            budget_used,
            dropped_items_by_id: dropped,
            reducers_applied,
        };

        Ok((selected, manifest))
    }

    /// Progressively apply reducers to a context block until it fits the target budget.
    async fn apply_pipeline(
        &self,
        id: &str,
        text: Arc<str>,
        target: usize,
        dict: super::cache_strategy::TokenizerDict,
        applied: &mut Vec<String>,
    ) -> (String, usize) {
        let mut current_text = text;
        let mut current_tokens = count_tokens_deterministic(&current_text, dict);

        if current_tokens <= target {
            return (current_text.to_string(), current_tokens);
        }

        // 1. Line deduplication
        if let Ok((new_text, new_tokens)) = DeduplicationReducer
            .reduce(current_text.clone(), target, dict)
            .await
        {
            if new_tokens < current_tokens {
                current_text = Arc::from(new_text);
                current_tokens = new_tokens;
                applied.push(format!("DeduplicationReducer({})", id));
            } else if current_text.len() > new_text.len() {
                // Byte count decreased but tokens didn't (rare but possible with BPE)
                current_text = Arc::from(new_text);
                current_tokens = new_tokens;
                applied.push(format!("DeduplicationReducer({})", id));
            }
        }

        if current_tokens <= target {
            return (current_text.to_string(), current_tokens);
        }

        // 2. Whitespace reduction
        if let Some((new_text, new_tokens)) = WhitespaceReducer
            .reduce(current_text.clone(), target, dict)
            .await
            .ok()
            .filter(|(_, nt)| *nt < current_tokens)
        {
            current_text = Arc::from(new_text);
            current_tokens = new_tokens;
            applied.push(format!("WhitespaceReducer({})", id));
        }

        if current_tokens <= target {
            return (current_text.to_string(), current_tokens);
        }

        // 3. Semantic truncation (Last resort)
        if let Ok((new_text, new_tokens)) = SemanticReducer
            .reduce(current_text.clone(), target, dict)
            .await
        {
            current_text = Arc::from(new_text);
            current_tokens = new_tokens;
            applied.push(format!("SemanticReducer({})", id));
        }

        (current_text.to_string(), current_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::cache_strategy::LlmCapabilities;
    use crate::context::layers::FrameInput;

    fn minimal_input() -> FrameInput {
        FrameInput {
            profile: "standard".to_string(),
            total_tokens: 4000,
            soul: "You are a soul.".to_string(),
            harness_instructions: String::new(),
            user_message: "Hello".to_string(),
            memory: "Memory".to_string(),
            private_tags: vec![],
            thread: vec![],
            pinned_notes: vec![],
            search_results: vec![],
            query_entities: vec![],
        }
    }

    #[tokio::test]
    async fn build_context_succeeds() {
        let engine = ContextEngine::new();
        let (blocks, manifest) = engine
            .build_context(minimal_input(), LlmCapabilities::default())
            .await
            .unwrap();

        assert!(!blocks.is_empty());
        assert!(manifest.budget_used > 0);
        assert!(manifest.budget_used <= manifest.total_budget);
    }

    #[tokio::test]
    async fn failsafe_triggers_reduction() {
        let engine = ContextEngine::new();
        let mut input = minimal_input();
        input.soul = "A".repeat(1000); // very large
        let caps = LlmCapabilities {
            exact_context_window: 10, // small window
            ..Default::default()
        };

        let (blocks, _manifest) = engine.build_context(input, caps).await.unwrap();
        assert!(!blocks.is_empty());
        let content = &blocks[0].content;
        assert!(
            content.contains("TRUNCATED") || content.len() < 1000,
            "Content was neither truncated nor reduced: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_build_context_with_reduction() {
        let engine = ContextEngine::new();
        let mut input = minimal_input();
        // Memory is 12000 chars, which is ~3000-4000 tokens based on heuristic
        input.memory = "repeated line\n".repeat(1000);

        let caps = LlmCapabilities {
            exact_context_window: 10000,
            ..Default::default()
        };

        let (blocks, manifest) = engine.build_context(input, caps).await.unwrap();

        // Ensure memory block exists
        let memory_block = blocks
            .iter()
            .find(|b| b.metadata.id == "long_term_memory")
            .unwrap();

        // It should have been reduced
        assert!(memory_block.content.len() < 12000);
        assert!(!manifest.reducers_applied.is_empty());
        assert!(
            manifest
                .reducers_applied
                .iter()
                .any(|r| r.contains("DeduplicationReducer"))
        );
    }
}
