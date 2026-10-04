use super::block::{BlockMetadata, ContextBlock};
use super::bpe::count_tokens_deterministic;
use super::cache_strategy::TokenizerDict;
use super::reducer::{ContextReducer, SemanticReducer}; // Added SemanticReducer
use hq_core::types::MessageRole;
use std::sync::Arc;

/// A candidate for context inclusion.
pub struct ContextCandidate {
    pub id: String,
    pub content: Arc<str>,
    pub role: MessageRole,
    pub base_priority: f32, // 0.0 to 1.0
    pub source: Option<String>,
    /// Pre-computed token count from vault_cache. When present, skips the deterministic counter.
    pub token_count_hint: Option<usize>,
}

/// Allocate tokens using a greedy priority-based approach with Elastic Reduction.
///
/// High-priority items (>0.8) that don't fit are given one last reduction attempt
/// to fit exactly within the remaining budget.
pub async fn allocate_knapsack(
    mut candidates: Vec<ContextCandidate>,
    total_budget: usize,
    dict: TokenizerDict,
) -> (Vec<ContextBlock>, Vec<String>, Vec<String>) {
    // Sort by priority descending
    candidates.sort_by(|a, b| {
        b.base_priority
            .partial_cmp(&a.base_priority)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut remaining = total_budget;
    let mut selected = Vec::new();
    let mut dropped = Vec::new();
    let mut applied_reducers = Vec::new();

    for candidate in candidates {
        if remaining == 0 {
            dropped.push(candidate.id);
            continue;
        }

        let tokens = candidate
            .token_count_hint
            .unwrap_or_else(|| count_tokens_deterministic(&candidate.content, dict));
        if tokens <= remaining {
            selected.push(ContextBlock {
                role: candidate.role,
                content: candidate.content,
                cache_breakpoint: false,
                metadata: BlockMetadata {
                    id: candidate.id,
                    priority: candidate.base_priority,
                    source: candidate.source,
                },
            });
            remaining -= tokens;
        } else if candidate.base_priority >= 0.8 && remaining > 50 {
            // ELASTIC REDUCTION: High priority item didn't fit, try to reduce it to fit exactly.
            if let Ok((reduced_text, reduced_tokens)) = SemanticReducer
                .reduce(candidate.content.clone(), remaining, dict)
                .await
            {
                if reduced_tokens <= remaining {
                    selected.push(ContextBlock {
                        role: candidate.role,
                        content: Arc::from(reduced_text),
                        cache_breakpoint: false,
                        metadata: BlockMetadata {
                            id: candidate.id.clone(),
                            priority: candidate.base_priority,
                            source: candidate.source,
                        },
                    });
                    remaining -= reduced_tokens;
                    applied_reducers.push(format!("ElasticReduction({})", candidate.id));
                } else {
                    dropped.push(candidate.id);
                }
            } else {
                dropped.push(candidate.id);
            }
        } else {
            dropped.push(candidate.id);
        }
    }

    (selected, dropped, applied_reducers)
}

/// Dynamic weights for layers according to spec Section 6.1.
pub fn layer_priority(layer: &str) -> f32 {
    match layer {
        "injections" => 1.0,
        "user_message" => 0.9,
        "system" => 0.8,
        "thread" => 0.6,
        "memory" => 0.4,
        _ => 0.1,
    }
}
