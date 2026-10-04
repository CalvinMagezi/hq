use crate::context::bpe::count_tokens_deterministic;
use crate::context::cache_strategy::{LlmCapabilities, TokenizerDict};
use crate::context::engine::ContextEngine;
use crate::context::layers::FrameInput;
use tokio;

fn stress_input(multiplier: usize) -> FrameInput {
    FrameInput {
        profile: "standard".to_string(),
        total_tokens: 4000,
        soul: "System soul context. ".repeat(multiplier),
        harness_instructions: String::new(),
        user_message: "User message query. ".repeat(multiplier),
        memory: "Long term memory snippet. ".repeat(multiplier * 5),
        private_tags: vec![],
        thread: vec![],
        pinned_notes: vec![],
        search_results: vec![],
        query_entities: vec![],
    }
}

#[tokio::test]
async fn test_ironclad_oom_prevention() {
    let engine = ContextEngine::new();
    // Huge input that would cause OOM if handled as String clones
    let input = stress_input(5000);
    let caps = LlmCapabilities::default(); // 128k context default

    let res = engine.build_context(input, caps).await;
    assert!(
        res.is_ok(),
        "Engine failed under heavy load: {:?}",
        res.err()
    );

    let (blocks, manifest) = res.unwrap();
    assert!(manifest.budget_used <= manifest.total_budget);
    assert!(!blocks.is_empty());
}

#[tokio::test]
async fn test_ironclad_drift_safety() {
    let engine = ContextEngine::new();
    let mut input = stress_input(100);
    input.memory = "Custom text with high drift potential... ".repeat(500);

    // Simulate a model with no native BPE (Claude)
    let caps = LlmCapabilities {
        supports_prompt_caching: true,
        exact_context_window: 4000,
        requires_explicit_breakpoints: true,
        tokenizer: TokenizerDict::Claude,
    };

    let (blocks, manifest) = engine.build_context(input, caps).await.unwrap();

    // Calculate REAL tokens if we had GPT-4 (as a proxy for ground truth)
    let total_content: String = blocks
        .iter()
        .map(|b| b.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n");
    let ground_truth = count_tokens_deterministic(&total_content, TokenizerDict::Cl100kBase);

    // Manifest budget_used should be >= ground_truth due to DRIFT_SAFETY_FACTOR (1.10)
    // This confirms we are staying safe.
    assert!(
        manifest.budget_used >= ground_truth,
        "Drift safety failed: manifest={}, truth={}",
        manifest.budget_used,
        ground_truth
    );
}

#[tokio::test]
async fn test_ironclad_elastic_knapsack() {
    let engine = ContextEngine::new();
    let input = FrameInput {
        profile: "standard".to_string(),
        total_tokens: 4000,
        soul: "A ".repeat(5000), // ~5000 tokens
        harness_instructions: String::new(),
        user_message: "Hello".to_string(),
        memory: String::new(),
        private_tags: vec![],
        thread: vec![],
        pinned_notes: vec![],
        search_results: vec![],
        query_entities: vec![],
    };

    let caps = LlmCapabilities {
        exact_context_window: 1000, // Small window
        ..Default::default()
    };

    let (blocks, _manifest) = engine.build_context(input, caps).await.unwrap();

    let found_ids: Vec<String> = blocks.iter().map(|b| b.metadata.id.clone()).collect();
    println!("Found block IDs: {:?}", found_ids);

    // Check if the high priority item was REDUCED instead of DROPPED
    let soul_block = blocks.iter().find(|b| b.metadata.id == "system_soul");
    assert!(
        soul_block.is_some(),
        "System soul was dropped! Found: {:?}",
        found_ids
    );
    assert!(
        soul_block.unwrap().content.contains("[TRUNCATED]"),
        "System soul was not truncated"
    );
}
