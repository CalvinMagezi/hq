//! Context window management: token estimation, compaction, preemptive summarization.

use hq_core::types::{ChatMessage, MessageRole, SessionEvent};
use hq_llm::provider::ChatRequest;
use tracing::{debug, info, warn};

use super::{AgentSession, PrecomputedSummary};

pub use hq_core::tokens::count_tokens_char_weighted as estimate_token_count;

/// Build a summary-request ChatRequest from a slice of messages.
fn build_summary_request(model: &str, messages: &[ChatMessage]) -> ChatRequest {
    let turns_count = messages.len();
    let history = messages
        .iter()
        .map(|m| format!("[{}] {}", format!("{:?}", m.role).to_lowercase(), m.content))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "You are summarizing a conversation to preserve context after compaction. \
         Use this exact format:\n\n\
         ## Conversation Summary\n\
         **Turns summarized:** {turns_count}\n\n\
         ### User Goal\n\
         One sentence describing what the user is trying to accomplish.\n\n\
         ### What Was Done\n\
         - Bullet points of completed actions, decisions made, and key outputs\n\
         - Include specific file paths, function names, variable names, and config values\n\
         - Note any errors encountered and how they were resolved\n\n\
         ### Current State\n\
         What is the system/code/task state right now? What was the last thing discussed?\n\n\
         ### Pending / Next Steps\n\
         - What remains to be done\n\
         - Any open questions or blockers\n\n\
         ### Key Context\n\
         - Important constraints, preferences, or decisions that must not be forgotten\n\
         - Specific values: model names, ports, paths, credentials references, versions\n\n\
         Keep the summary under 800 tokens. Every token should carry information. \
         No pleasantries or meta-commentary.\n\n\
         ---\n\n\
         Conversation to summarize:\n\n{history}",
    );
    ChatRequest {
        model: model.to_string(),
        messages: vec![ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: prompt,
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        }],
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(2000),
    }
}

impl AgentSession {
    /// Estimate total tokens in the conversation (O(1) via cached per-message counts).
    pub(super) fn estimate_tokens(&self) -> usize {
        self.system_prompt_tokens
            + self.tool_schema_tokens
            + self.message_tokens.iter().sum::<usize>()
    }

    /// Compact the message history when context is getting full.
    ///
    /// Keeps the last 10 messages and summarizes older ones via an LLM call.
    /// On failure, falls back to simple truncation.
    pub async fn compact(&mut self) {
        let keep_count = 10;
        if self.messages.len() <= keep_count {
            return;
        }

        let precomputed = {
            let mut lock = self.precomputed.lock().await;
            lock.take()
        };

        let old_len = self.messages.len();

        if let Some(summary) = precomputed {
            let drain_to = summary.covers_up_to.min(old_len - keep_count);
            let drained: Vec<_> = self.messages.drain(..drain_to).collect();
            self.message_tokens.drain(..drain_to);
            self.insert_message(
                0,
                ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::User,
                    content: format!(
                        "[CONTEXT SUMMARY - {} earlier messages compacted (preemptive)]\n{}",
                        drained.len(),
                        summary.text,
                    ),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                },
            );
            let new_len = self.messages.len();
            self.emit(SessionEvent::Compaction {
                old_messages: old_len,
                new_messages: new_len,
            });
            info!(
                old = old_len,
                new = new_len,
                "compacted via preemptive summary (instant)"
            );
            return;
        }

        let drain_count = old_len - keep_count;
        let mut older: Vec<ChatMessage> = self.messages.drain(..drain_count).collect();
        self.message_tokens.drain(..drain_count);

        // Deterministic pre-pass (no LLM): shrink oversized tool-result
        // messages before reaching for a summarization call. Mirrors
        // `microcompact_result`'s per-call pass but runs a second, stricter
        // time at compaction, since these messages are leaving live history
        // either way and only their gist matters to a downstream summary.
        let pruned_tokens = crate::context::compactor::prune_tool_results(
            &mut older,
            crate::context::compactor::PRE_COMPACT_TOOL_RESULT_CAP,
        );
        // Same counting convention `prune_tool_results` itself used to decide
        // truncation points (`count_tokens_fast`), not `estimate_token_count`'s
        // heavier heuristic — mixing the two would make the skip-threshold
        // comparison below meaningless.
        let pruned_total: usize = older
            .iter()
            .map(|m| hq_core::tokens::count_tokens_fast(&m.content))
            .sum();
        if pruned_tokens > 0 {
            debug!(
                pruned_tokens,
                pruned_total, "compact: deterministic tool-result pruning pass"
            );
        }

        // Common case: one bloated tool output padded out an otherwise-small
        // batch. Once pruned, the batch is already smaller than a summary
        // would be — skip the LLM round-trip entirely and fold it back in
        // as-is.
        if pruned_total <= crate::context::compactor::DETERMINISTIC_SKIP_THRESHOLD {
            let joined = older
                .iter()
                .map(|m| format!("[{}] {}", format!("{:?}", m.role).to_lowercase(), m.content))
                .collect::<Vec<_>>()
                .join("\n");
            self.insert_message(
                0,
                ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::User,
                    content: format!(
                        "[CONTEXT SUMMARY - {} earlier messages compacted (deterministic, no LLM)]\n{}",
                        older.len(),
                        joined
                    ),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                },
            );
            let new_len = self.messages.len();
            self.emit(SessionEvent::Compaction {
                old_messages: old_len,
                new_messages: new_len,
            });
            info!(
                old = old_len,
                new = new_len,
                "compacted via deterministic pruning (no LLM call)"
            );
            return;
        }

        let summary_request = build_summary_request(&self.config.model, &older);
        let ctx = self.session_context();
        match hq_llm::SESSION_CONTEXT
            .scope(ctx, self.provider.chat(&summary_request))
            .await
        {
            Ok(response) => {
                self.insert_message(
                    0,
                    ChatMessage {
                        image_parts: Vec::new(),
                        role: MessageRole::User,
                        content: format!(
                            "[CONTEXT SUMMARY - {} earlier messages compacted]\n{}",
                            older.len(),
                            response.message.content
                        ),
                        tool_calls: Vec::new(),
                        tool_call_id: None,
                        reasoning_content: None,
                    },
                );
                let new_len = self.messages.len();
                self.emit(SessionEvent::Compaction {
                    old_messages: old_len,
                    new_messages: new_len,
                });
                info!(
                    old = old_len,
                    new = new_len,
                    "compacted via LLM summarization"
                );
            }
            Err(e) => {
                let new_len = self.messages.len();
                self.emit(SessionEvent::Compaction {
                    old_messages: old_len,
                    new_messages: new_len,
                });
                warn!(error = %e, old = old_len, new = new_len, "compaction summary failed, fell back to truncation");
            }
        }
    }

    /// Check if compaction or preemptive summarization is needed, and act.
    /// Returns true if compaction was triggered.
    pub(super) async fn check_compaction(&mut self) -> bool {
        let estimated = self.estimate_tokens();
        let threshold =
            (self.config.context_window as f64 * self.config.compaction_threshold) as usize;
        let compacted = if estimated > threshold {
            info!(
                estimated_tokens = estimated,
                threshold, "compacting context"
            );
            self.compact().await;
            true
        } else {
            false
        };

        if self.preemptive_handle.is_none() {
            let preemptive_threshold =
                (self.config.context_window as f64 * self.config.preemptive_threshold) as usize;
            if estimated > preemptive_threshold {
                self.spawn_preemptive_summarizer();
            }
        }

        compacted
    }

    /// Spawn a background task to precompute a summary for instant use at next compaction.
    pub(super) fn spawn_preemptive_summarizer(&mut self) {
        let keep_count = 10;
        if self.messages.len() <= keep_count + 5 {
            return;
        }

        let snapshot_end = self.messages.len() - keep_count;
        let snapshot: Vec<ChatMessage> = self.messages[..snapshot_end].to_vec();
        let covers_up_to = snapshot_end;

        let provider = self.provider.clone();
        let model = self.config.model.clone();
        let precomputed = self.precomputed.clone();
        let sid = self.session_id.clone();
        let turn = self.tool_call_count as i64;
        let origin = self.ledger_origin();

        self.preemptive_handle = Some(tokio::spawn(async move {
            let request = build_summary_request(&model, &snapshot);
            let ctx = hq_llm::SessionContext {
                session_id: sid,
                turn_idx: turn,
                origin,
            };
            match hq_llm::SESSION_CONTEXT
                .scope(ctx, provider.chat(&request))
                .await
            {
                Ok(response) => {
                    let mut lock = precomputed.lock().await;
                    *lock = Some(PrecomputedSummary {
                        text: response.message.content,
                        covers_up_to,
                    });
                    info!(covers_up_to, "preemptive summary ready");
                }
                Err(e) => {
                    warn!(error = %e, "preemptive summarization failed (will fall back to sync)");
                }
            }
        }));
    }

    /// Apply microcompact truncation to an oversized tool result.
    ///
    /// Delegates to `hq_core::microcompact::microcompact` for deduplication,
    /// head+tail, and disk spill.
    pub(super) fn microcompact_result(&self, tool_name: &str, result_text: String) -> String {
        use hq_core::microcompact::{MicrocompactStrategy, microcompact as canonical_microcompact};

        let mc = canonical_microcompact(&result_text);

        if mc.strategy == MicrocompactStrategy::PassThrough {
            return result_text;
        }

        debug!(
            tool = %tool_name,
            original_tokens = mc.original_tokens,
            compacted_tokens = mc.compacted_tokens,
            strategy = ?mc.strategy,
            "microcompact: truncated tool result"
        );

        mc.text
    }
}
