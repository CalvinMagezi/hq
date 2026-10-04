//! Thread compaction — summarize older messages to fit within budget.

use hq_core::types::{ChatMessage, MessageRole};

use hq_core::tokens::{TruncationStrategy, count_tokens_fast, truncate_with_strategy};

/// Per-message token cap applied by [`prune_tool_results`]. Deliberately
/// stricter than `microcompact::MICROCOMPACT_THRESHOLD` (4000): these
/// messages are about to be summarized or dropped from live history, so
/// only their gist needs to survive into the summarization prompt.
pub const PRE_COMPACT_TOOL_RESULT_CAP: usize = 800;

/// Below this total token count, a pruned batch is already small enough
/// that an LLM summarization call would cost more (latency, a network
/// round-trip, $) than it saves. Deliberately set above a single
/// [`PRE_COMPACT_TOOL_RESULT_CAP`]-capped message plus modest conversational
/// overhead, so the common case — one bloated tool output dominating an
/// otherwise-small batch — clears it. Skipping the call for these batches is
/// what makes `prune_tool_results` deliver "zero extra LLM calls for the
/// common case of one bloated tool output" as a real behavior rather than
/// just a smaller prompt.
pub const DETERMINISTIC_SKIP_THRESHOLD: usize = 1000;

/// Deterministically shrink oversized tool-result messages in a batch about
/// to be compacted, via head/middle/tail truncation — no LLM call. Meant to
/// run as a pre-pass immediately before the caller's LLM-summary path so the common case — one or more
/// bloated tool outputs padding out an otherwise-small batch — needs no
/// summarization at all once combined with [`DETERMINISTIC_SKIP_THRESHOLD`].
///
/// Only `Tool`-role messages are touched (user/assistant/system turns carry
/// the actual conversation and are left alone). Returns the total tokens
/// removed across all touched messages.
pub fn prune_tool_results(messages: &mut [ChatMessage], per_message_cap: usize) -> usize {
    let mut saved = 0usize;
    for msg in messages.iter_mut() {
        if msg.role != MessageRole::Tool {
            continue;
        }
        let before = count_tokens_fast(&msg.content);
        if before <= per_message_cap {
            continue;
        }
        let truncated =
            truncate_with_strategy(&msg.content, per_message_cap, TruncationStrategy::MiddleOut);
        let after = count_tokens_fast(&truncated);
        saved += before.saturating_sub(after);
        msg.content = truncated;
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_msg(content: &str) -> ChatMessage {
        ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::Tool,
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: Some("tc-1".to_string()),
            reasoning_content: None,
        }
    }

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }
    }

    #[test]
    fn prune_tool_results_leaves_small_messages_untouched() {
        let mut messages = vec![tool_msg("short output"), user_msg("hello")];
        let saved = prune_tool_results(&mut messages, PRE_COMPACT_TOOL_RESULT_CAP);
        assert_eq!(saved, 0);
        assert_eq!(messages[0].content, "short output");
        assert_eq!(messages[1].content, "hello");
    }

    #[test]
    fn prune_tool_results_shrinks_oversized_tool_message() {
        let big = "line\n".repeat(5000); // well over the 800-token cap
        let mut messages = vec![tool_msg(&big)];
        let before_tokens = count_tokens_fast(&messages[0].content);
        let saved = prune_tool_results(&mut messages, PRE_COMPACT_TOOL_RESULT_CAP);
        let after_tokens = count_tokens_fast(&messages[0].content);
        assert!(saved > 0);
        assert!(after_tokens < before_tokens);
        assert!(after_tokens <= PRE_COMPACT_TOOL_RESULT_CAP + 50); // truncation isn't byte-exact
    }

    #[test]
    fn prune_tool_results_never_touches_non_tool_messages() {
        let big = "line\n".repeat(5000);
        let mut messages = vec![user_msg(&big)];
        let saved = prune_tool_results(&mut messages, PRE_COMPACT_TOOL_RESULT_CAP);
        assert_eq!(saved, 0);
        assert_eq!(messages[0].content, big);
    }

    #[test]
    fn prune_tool_results_gets_bloated_batch_under_skip_threshold() {
        // One bloated tool output alongside small conversational turns —
        // the exact "common case" dsh's rule targets: after pruning, the
        // whole batch should collapse under DETERMINISTIC_SKIP_THRESHOLD
        // without ever calling an LLM.
        let mut messages = vec![
            user_msg("please read this file"),
            tool_msg(&"x".repeat(40_000)),
            user_msg("thanks"),
        ];
        prune_tool_results(&mut messages, PRE_COMPACT_TOOL_RESULT_CAP);
        let total: usize = messages.iter().map(|m| count_tokens_fast(&m.content)).sum();
        assert!(
            total <= DETERMINISTIC_SKIP_THRESHOLD,
            "expected pruned batch to fit under the skip threshold, got {total} tokens"
        );
    }
}
