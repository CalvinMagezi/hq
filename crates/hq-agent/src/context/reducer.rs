use super::bpe::count_tokens_deterministic;
use super::cache_strategy::TokenizerDict;
use async_trait::async_trait;
use hq_core::tokens::snap_to_char_boundary;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContextError {
    #[error("Capacity exceeded")]
    CapacityExceeded,
    #[error("Reducer error: {0}")]
    ReducerError(String),
}

#[async_trait]
pub trait ContextReducer: Send + Sync {
    /// Returns the reduced string and the new token count.
    /// Input is Arc<str> to allow zerocopy offloading to spawn_blocking.
    async fn reduce(
        &self,
        text: Arc<str>,
        target_tokens: usize,
        dict: TokenizerDict,
    ) -> Result<(String, usize), ContextError>;

    fn name(&self) -> &'static str;
}

pub struct WhitespaceReducer;

#[async_trait]
impl ContextReducer for WhitespaceReducer {
    async fn reduce(
        &self,
        text: Arc<str>,
        _target_tokens: usize,
        dict: TokenizerDict,
    ) -> Result<(String, usize), ContextError> {
        let reduced = tokio::task::spawn_blocking(move || {
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        })
        .await
        .map_err(|e| ContextError::ReducerError(e.to_string()))?;

        let new_tokens = count_tokens_deterministic(&reduced, dict);
        Ok((reduced, new_tokens))
    }

    fn name(&self) -> &'static str {
        "WhitespaceReducer"
    }
}

pub struct DeduplicationReducer;

#[async_trait]
impl ContextReducer for DeduplicationReducer {
    async fn reduce(
        &self,
        text: Arc<str>,
        _target_tokens: usize,
        dict: TokenizerDict,
    ) -> Result<(String, usize), ContextError> {
        let reduced = tokio::task::spawn_blocking(move || {
            let lines: Vec<&str> = text.lines().collect();
            let mut result = Vec::new();
            let mut last_line = None;
            for line in lines {
                if Some(line) != last_line {
                    result.push(line);
                    last_line = Some(line);
                }
            }
            result.join("\n")
        })
        .await
        .map_err(|e| ContextError::ReducerError(e.to_string()))?;

        let new_tokens = count_tokens_deterministic(&reduced, dict);
        Ok((reduced, new_tokens))
    }

    fn name(&self) -> &'static str {
        "DeduplicationReducer"
    }
}

pub struct SemanticReducer;

#[async_trait]
impl ContextReducer for SemanticReducer {
    async fn reduce(
        &self,
        text: Arc<str>,
        target_tokens: usize,
        dict: TokenizerDict,
    ) -> Result<(String, usize), ContextError> {
        let mut current_tokens = count_tokens_deterministic(&text, dict);
        if current_tokens <= target_tokens {
            return Ok((text.to_string(), current_tokens));
        }

        // We use a loops to handle cases where the heuristic is too optimistic (e.g. very dense tokens)
        let mut factor = 2.0; // chars per token
        for _ in 0..3 {
            // Max 3 attempts to reduce further
            let byte_budget = (target_tokens as f64 * factor).floor() as usize;
            if byte_budget >= text.len() && factor > 0.5 {
                factor -= 0.5;
                continue;
            }

            let reduced_str = {
                let text_clone = text.clone();
                tokio::task::spawn_blocking(move || {
                    let mut safe_cut = snap_to_char_boundary(&text_clone, byte_budget);
                    let trimmed = text_clone.trim_start();
                    let is_json = trimmed.starts_with('{') || trimmed.starts_with('[');

                    if is_json {
                        let search_area = &text_clone[..safe_cut];
                        if let Some(last_comma) = search_area.rfind(',') {
                            safe_cut = last_comma;
                        } else if let Some(last_close) = search_area.rfind(['}', ']']) {
                            safe_cut = last_close + 1;
                        }
                    } else if let Some(last_nl) = text_clone[..safe_cut].rfind('\n') {
                        safe_cut = last_nl;
                    }

                    let head = &text_clone[..safe_cut];
                    if is_json {
                        let suffix = if trimmed.starts_with('[') { "]" } else { "}" };
                        format!("{}... [TRUNCATED]{}", head, suffix)
                    } else {
                        format!("{}... [TRUNCATED]", head)
                    }
                })
                .await
                .map_err(|e| ContextError::ReducerError(e.to_string()))?
            };

            current_tokens = count_tokens_deterministic(&reduced_str, dict);
            if current_tokens <= target_tokens {
                return Ok((reduced_str, current_tokens));
            }
            factor *= 0.7; // Reduce the factor and try again
        }

        // Fallback: Hard truncate at 1 byte per token if everything else fails
        let final_cut = snap_to_char_boundary(&text, target_tokens / 2);
        let final_str = format!("{}... [HARD TRUNCATED]", &text[..final_cut]);
        let final_tokens = count_tokens_deterministic(&final_str, dict);
        Ok((final_str, final_tokens))
    }

    fn name(&self) -> &'static str {
        "SemanticReducer"
    }
}
