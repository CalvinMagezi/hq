//! Context window management: token estimation, compaction, preemptive summarization.

use hq_core::types::{ChatMessage, MessageRole, SessionEvent};
use hq_llm::provider::ChatRequest;
use tracing::{debug, info, warn};

use super::{AgentSession, PrecomputedSummary};

pub use hq_core::tokens::count_tokens_char_weighted as estimate_token_count;

/// Every compaction summary starts with this, so later compactions leave it in place.
const SUMMARY_PREFIX: &str = "[CONTEXT SUMMARY";

/// How many of the newest messages a compaction keeps as they are.
const KEEP_RECENT: usize = 10;

/// A compaction whose summary failed puts the (tool-pruned) messages back when they are at most
/// this many tokens, instead of dropping them.
const RESTORE_ON_FAILURE_TOKENS: usize = 8_000;

/// Characters of a tool call's arguments shown to the summarizer.
const TOOL_ARGS_CHARS: usize = 300;

fn is_summary(m: &ChatMessage) -> bool {
    m.role == MessageRole::User && m.content.starts_with(SUMMARY_PREFIX)
}

/// Where the messages after the leading summaries begin. Earlier summaries are never rewritten:
/// each compaction summarizes only what came after them, so detail is summarized once.
fn summaries_end(messages: &[ChatMessage]) -> usize {
    messages.iter().take_while(|m| is_summary(m)).count()
}

/// The end of the range to compact, `messages[start..cut]`, keeping about the newest `keep`.
/// The cut never lands on a tool result, so a call and its results stay together. It moves back
/// first (keeping more), and forward only when moving back would leave nothing to compact.
fn compact_cut(messages: &[ChatMessage], start: usize, keep: usize) -> Option<usize> {
    let len = messages.len();
    if len <= start + keep {
        return None;
    }
    let ideal = len - keep;
    let at_result = |i: usize| i < len && messages[i].role == MessageRole::Tool;
    let mut back = ideal;
    while back > start && at_result(back) {
        back -= 1;
    }
    if back > start + 1 {
        return Some(back);
    }
    let mut forward = ideal;
    while forward < len && at_result(forward) {
        forward += 1;
    }
    (forward > start && forward < len).then_some(forward)
}

/// The messages as the summarizer reads them, tool calls and their arguments included.
fn render_for_summary(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    for m in messages {
        let role = format!("{:?}", m.role).to_lowercase();
        out.push_str(&format!("[{role}] {}\n", m.content));
        for call in &m.tool_calls {
            let args = call.arguments.to_string();
            let args: String = args.chars().take(TOOL_ARGS_CHARS).collect();
            out.push_str(&format!("  -> called {}({args})\n", call.name));
        }
    }
    out
}

/// Build a summary-request ChatRequest from a slice of messages.
fn build_summary_request(model: &str, messages: &[ChatMessage]) -> ChatRequest {
    let turns_count = messages.len();
    let history = render_for_summary(messages);
    let prompt = format!(
        "You are summarizing part of a conversation to preserve context after compaction. \
         Earlier summaries stay in place, so summarize only these messages. \
         Use this exact format:\n\n\
         ## Conversation Summary\n\
         **Turns summarized:** {turns_count}\n\n\
         ### User Goal\n\
         One sentence describing what the user is trying to accomplish.\n\n\
         ### What Was Done\n\
         - Bullet points of completed actions, decisions made (with the reason), and key outputs\n\
         - Keep exact file paths with line numbers, function names, commands, error text and config values\n\
         - Note any errors encountered and how they were resolved, and approaches that failed and why\n\n\
         ### Current State\n\
         What is the system/code/task state right now? What was the last thing discussed?\n\n\
         ### Pending / Next Steps\n\
         - What remains to be done, including everything the user asked for that is not done yet\n\
         - Any open questions or blockers\n\n\
         ### Key Context\n\
         - Important constraints, preferences, or decisions that must not be forgotten\n\
         - Specific values: model names, ports, paths, credentials references, versions\n\n\
         Keep the summary under 800 tokens. Every token should carry information. \
         No pleasantries or meta-commentary. Never copy secrets.\n\n\
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

fn summary_message(content: String) -> ChatMessage {
    ChatMessage {
        image_parts: Vec::new(),
        role: MessageRole::User,
        content,
        tool_calls: Vec::new(),
        tool_call_id: None,
        reasoning_content: None,
    }
}

impl AgentSession {
    /// Estimate total tokens in the conversation (O(1) via cached per-message counts).
    pub(super) fn estimate_tokens(&self) -> usize {
        self.system_prompt_tokens
            + self.tool_schema_tokens
            + self.message_tokens.iter().sum::<usize>()
    }

    /// Keeps the full text of messages about to leave live history, when this session knows its
    /// vault, and returns the note that points at it.
    fn spill_note(&self, messages: &[ChatMessage]) -> String {
        let Some(vault) = &self.vault_path else {
            return String::new();
        };
        match hq_tools::spill::write(
            vault,
            &self.session_id,
            "compacted",
            &render_for_summary(messages),
        ) {
            Ok(r) => format!(" Full text kept as {r}; tool_output_read(\"{r}\") returns it."),
            Err(e) => {
                debug!(error = %e, "compact: could not spill the compacted messages");
                String::new()
            }
        }
    }

    /// Compact the message history when context is getting full.
    ///
    /// Keeps the newest messages and summarizes the ones between the earlier summaries and them,
    /// so an earlier summary is never summarized again. The cut never separates a tool call from
    /// its results. The compacted messages' full text is kept on disk (`hq_tools::spill`). If the
    /// summary call fails, small batches go back as they were; larger ones become a note that
    /// names where their full text is.
    pub async fn compact(&mut self) {
        let start = summaries_end(&self.messages);
        let Some(mut cut) = compact_cut(&self.messages, start, KEEP_RECENT) else {
            return;
        };

        let precomputed = {
            let mut lock = self.precomputed.lock().await;
            lock.take()
        };
        let precomputed = precomputed.filter(|p| {
            p.generation == self.compaction_generation
                && p.start == start
                && p.covers_up_to > start
                && p.covers_up_to <= cut
        });

        let old_len = self.messages.len();
        if let Some(p) = &precomputed {
            cut = p.covers_up_to;
        }
        let mut older: Vec<ChatMessage> = self.messages.drain(start..cut).collect();
        self.message_tokens.drain(start..cut);
        self.compaction_generation += 1;
        let kept = self.spill_note(&older);

        if let Some(summary) = precomputed {
            self.insert_message(
                start,
                summary_message(format!(
                    "{SUMMARY_PREFIX} - {} earlier messages compacted (preemptive).{kept}]\n{}",
                    older.len(),
                    summary.text,
                )),
            );
            self.finish_compaction(old_len, "compacted via preemptive summary (instant)");
            return;
        }

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
            let joined = render_for_summary(&older);
            self.insert_message(
                start,
                summary_message(format!(
                    "{SUMMARY_PREFIX} - {} earlier messages compacted (deterministic, no LLM).{kept}]\n{}",
                    older.len(),
                    joined
                )),
            );
            self.finish_compaction(old_len, "compacted via deterministic pruning (no LLM call)");
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
                    start,
                    summary_message(format!(
                        "{SUMMARY_PREFIX} - {} earlier messages compacted.{kept}]\n{}",
                        older.len(),
                        response.message.content
                    )),
                );
                self.finish_compaction(old_len, "compacted via LLM summarization");
            }
            Err(e) if pruned_total <= RESTORE_ON_FAILURE_TOKENS => {
                // Pruning alone made room: put the messages back rather than lose them.
                for (i, m) in older.into_iter().enumerate() {
                    self.insert_message(start + i, m);
                }
                self.finish_compaction(
                    old_len,
                    "compaction summary failed; kept the pruned messages",
                );
                warn!(error = %e, "compaction summary failed, kept the messages with tool output pruned");
            }
            Err(e) => {
                let gist = hq_core::tokens::truncate_with_strategy(
                    &render_for_summary(&older),
                    crate::context::compactor::DETERMINISTIC_SKIP_THRESHOLD * 2,
                    hq_core::tokens::TruncationStrategy::MiddleOut,
                );
                self.insert_message(
                    start,
                    summary_message(format!(
                        "{SUMMARY_PREFIX} - {} earlier messages could not be summarized and are shown cut.{kept}]\n{gist}",
                        older.len(),
                    )),
                );
                self.finish_compaction(old_len, "compaction summary failed; kept a cut copy");
                warn!(error = %e, "compaction summary failed, kept a cut copy and the spilled full text");
            }
        }
    }

    fn finish_compaction(&mut self, old_len: usize, how: &'static str) {
        let new_len = self.messages.len();
        self.emit(SessionEvent::Compaction {
            old_messages: old_len,
            new_messages: new_len,
        });
        info!(old = old_len, new = new_len, "{how}");
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

    /// Spawn a background task to precompute a summary for instant use at next compaction. It
    /// summarizes the same range `compact` would cut now; `compact` uses it only if no other
    /// compaction moved the messages in between.
    pub(super) fn spawn_preemptive_summarizer(&mut self) {
        let start = summaries_end(&self.messages);
        let Some(covers_up_to) = compact_cut(&self.messages, start, KEEP_RECENT) else {
            return;
        };
        if covers_up_to < start + 5 {
            return;
        }
        let snapshot: Vec<ChatMessage> = self.messages[start..covers_up_to].to_vec();
        let generation = self.compaction_generation;

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
                        start,
                        covers_up_to,
                        generation,
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
    /// Delegates to `hq_core::microcompact::microcompact` for deduplication and head+tail, and
    /// keeps the full output on disk (`hq_tools::spill`) when the session knows its vault, so the
    /// agent can read what was cut.
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

        let Some(vault) = &self.vault_path else {
            return mc.text;
        };
        match hq_tools::spill::write(vault, &self.session_id, tool_name, &result_text) {
            Ok(r) => format!(
                "{}\n\n[Output cut to fit; the full {} tokens are kept as {r}. tool_output_read(\"{r}\") reads any part of it.]",
                mc.text, mc.original_tokens
            ),
            Err(e) => {
                debug!(error = %e, "microcompact: could not spill the full output");
                mc.text
            }
        }
    }
}
