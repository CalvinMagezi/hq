//! Microcompact: incremental result compression for tool outputs.
//!
//! When tool outputs exceed a threshold, microcompact reduces them while
//! preserving the most useful information. Unlike full compaction (which
//! requires an LLM), microcompact uses rule-based heuristics:
//!
//! 1. Head+tail preservation (keep first N + last M lines)
//! 2. Deduplication of repeated patterns (e.g., test output lines)
//! 3. Whitespace normalization

use crate::tokens::{TruncationStrategy, count_tokens_fast, truncate_with_strategy};

/// Threshold above which microcompact activates (in tokens).
pub const MICROCOMPACT_THRESHOLD: usize = 4000;

/// Maximum tokens to send to the model after compaction.
pub const MICROCOMPACT_TARGET: usize = 2000;

/// Result of microcompact processing.
#[derive(Debug, Clone)]
pub struct MicrocompactResult {
    /// The compacted text to send to the model.
    pub text: String,
    /// Original token count before compaction.
    pub original_tokens: usize,
    /// Token count after compaction.
    pub compacted_tokens: usize,
    /// What strategy was used.
    pub strategy: MicrocompactStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrocompactStrategy {
    /// No compaction needed.
    PassThrough,
    /// Head + tail preservation.
    HeadTail,
    /// Deduplication of repeated lines.
    Deduplicated,
    /// Middle-out truncation.
    MiddleOut,
}

/// Apply microcompact to a tool output string.
pub fn microcompact(text: &str) -> MicrocompactResult {
    let original_tokens = count_tokens_fast(text);

    if original_tokens <= MICROCOMPACT_THRESHOLD {
        return MicrocompactResult {
            text: text.to_string(),
            original_tokens,
            compacted_tokens: original_tokens,
            strategy: MicrocompactStrategy::PassThrough,
        };
    }

    // Try deduplication first (common in test output, logs)
    let deduped = deduplicate_lines(text);
    let deduped_tokens = count_tokens_fast(&deduped);
    if deduped_tokens <= MICROCOMPACT_TARGET {
        return MicrocompactResult {
            text: deduped,
            original_tokens,
            compacted_tokens: deduped_tokens,
            strategy: MicrocompactStrategy::Deduplicated,
        };
    }

    // Middle-out truncation for medium-sized outputs
    let compacted =
        truncate_with_strategy(text, MICROCOMPACT_TARGET, TruncationStrategy::MiddleOut);
    let compacted_tokens = count_tokens_fast(&compacted);

    MicrocompactResult {
        text: compacted,
        original_tokens,
        compacted_tokens,
        strategy: MicrocompactStrategy::MiddleOut,
    }
}

/// Deduplicate consecutive identical or near-identical lines.
fn deduplicate_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut result = Vec::new();
    let mut repeat_count = 0u32;
    let mut last_line: Option<&str> = None;

    for line in &lines {
        if Some(*line) == last_line {
            repeat_count += 1;
        } else {
            if repeat_count > 0 {
                result.push(format!("  ... ({repeat_count} identical lines omitted)"));
            }
            result.push(line.to_string());
            repeat_count = 0;
            last_line = Some(line);
        }
    }

    if repeat_count > 0 {
        result.push(format!("  ... ({repeat_count} identical lines omitted)"));
    }

    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_small_output() {
        let result = microcompact("hello world");
        assert_eq!(result.strategy, MicrocompactStrategy::PassThrough);
        assert_eq!(result.text, "hello world");
    }

    #[test]
    fn deduplication_works() {
        let repeated = "test passed\n".repeat(200);
        let result = deduplicate_lines(&repeated);
        assert!(count_tokens_fast(&result) < count_tokens_fast(&repeated));
        assert!(result.contains("identical lines omitted"));
    }

    #[test]
    fn large_output_compacted() {
        let large = "x".repeat(50_000);
        let result = microcompact(&large);
        assert!(result.compacted_tokens < result.original_tokens);
        assert_ne!(result.strategy, MicrocompactStrategy::PassThrough);
    }
}
