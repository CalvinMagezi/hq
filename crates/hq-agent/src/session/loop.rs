//! Main turn engine: one streaming state machine that drives both the
//! buffered `prompt()` facade and the streaming `prompt_stream()` path.
//!
//! Every turn flows through [`AgentSession::run_turns`], which consumes the
//! normalized [`BackendEventStream`](crate::backend::BackendEventStream) from
//! the session's explicit root backend, assembles tool-call deltas, executes
//! governed tools, and continues. `prompt()` and `prompt_stream()` differ only
//! in whether they ask API backends for token streaming — there is no second
//! turn implementation.

use anyhow::Result;
use hq_core::types::{
    ChatMessage, EventSource, MessageRole, SessionEvent, SessionResult, ToolCall,
};
use hq_llm::provider::ChatRequest;
use std::sync::atomic::Ordering;
use tracing::{debug, warn};

use super::context::estimate_token_count;
use super::credits::StepUsage;
use super::stream::{TurnOutput, wait_for_cancel};
use super::{AgentSession, SessionMode};
use crate::backend::BackendError;

/// What the turn loop does after inspecting an error delivered in the stream.
enum StreamErrorStep {
    /// Carry on with the turn, keeping the post-output error message if any.
    Proceed(Option<String>),
    /// A pre-output overflow was compacted away: run the turn again.
    Retry,
}

/// Turns before this one retry a blank reply once, since it is usually a hiccup.
const EMPTY_REPLY_RETRY_TURNS: u32 = 2;

/// A reply with no text and no tool calls early in the run.
fn is_early_blank_turn(turn: u32, content: &str, tool_calls: &[ToolCall]) -> bool {
    tool_calls.is_empty() && content.trim().is_empty() && turn < EMPTY_REPLY_RETRY_TURNS
}

/// Whether an `anyhow` error lifted from a backend is a context overflow.
///
/// A [`ProviderChain`](crate::backend::ProviderChain) delivers its errors lazily
/// through the stream (so `start` never blocks), so a pre-output context
/// overflow arrives here as a stream error rather than a `start` failure. This
/// lets the turn engine apply the same compact-and-retry recovery in both cases.
fn is_backend_context_overflow(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<BackendError>(),
        Some(BackendError::ContextOverflow(_))
    )
}

impl AgentSession {
    /// Run the prompt loop, collecting the run into a [`SessionResult`].
    ///
    /// API backends deliver a single buffered message (no token deltas); the
    /// same engine drives it. Callers that render live tokens use
    /// [`prompt_stream`](Self::prompt_stream) instead.
    pub async fn prompt(&mut self, text: &str) -> Result<SessionResult> {
        self.run_prompt(text, false, Vec::new()).await
    }

    /// Same as [`prompt`](Self::prompt), but attaches images to the user
    /// turn (FR-017) for vision-capable models. Callers that already staged
    /// the current turn's images into session history (e.g. via
    /// `push_message`) should NOT also call this with the same images —
    /// that would duplicate the turn. See `native_hq.rs`/relay dispatch code
    /// for the pattern of popping a pre-staged current-turn message and
    /// passing its `image_parts` here instead.
    pub async fn prompt_with_images(
        &mut self,
        text: &str,
        image_parts: Vec<hq_core::types::ImageAttachment>,
    ) -> Result<SessionResult> {
        self.run_prompt(text, false, image_parts).await
    }

    /// Run the prompt loop, asking streaming-capable API backends for token
    /// deltas. Emits [`SessionEvent::TextDelta`] chunks as they arrive. Tool
    /// execution, compaction, budget, limits, and continuation are identical to
    /// [`prompt`](Self::prompt) — it is the same engine, one `stream` flag apart.
    pub async fn prompt_stream(&mut self, text: &str) -> Result<SessionResult> {
        self.run_prompt(text, true, Vec::new()).await
    }

    /// Streaming counterpart to [`prompt_with_images`](Self::prompt_with_images).
    pub async fn prompt_stream_with_images(
        &mut self,
        text: &str,
        image_parts: Vec<hq_core::types::ImageAttachment>,
    ) -> Result<SessionResult> {
        self.run_prompt(text, true, image_parts).await
    }

    /// Shared entry: user message, run correlation, the turn engine, then
    /// cleanup + skill proposals. `stream` selects whether API backends stream token deltas.
    async fn run_prompt(
        &mut self,
        text: &str,
        stream: bool,
        image_parts: Vec<hq_core::types::ImageAttachment>,
    ) -> Result<SessionResult> {
        self.enrich_with_matching_skills(text);

        self.push_message(ChatMessage {
            image_parts,
            role: MessageRole::User,
            content: text.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        });

        self.begin_run();
        let result = self.run_turns(stream).await;
        self.end_run(&result);

        // Once per prompt, on the exchange the user actually saw.
        if let (Some(callback), Ok(done)) = (&self.post_turn_callback, &result)
            && !done.text().trim().is_empty()
        {
            let exchange = vec![
                ("user".to_string(), text.to_string()),
                ("assistant".to_string(), done.text().to_string()),
            ];
            let cb = callback.clone();
            tokio::spawn(async move { cb(exchange) });
        }

        if let Some(handle) = self.preemptive_handle.take() {
            handle.abort();
        }

        self.record_skill_outcome(&result);
        self.review_skills_async();

        result
    }

    /// The one turn loop. Consumes the root backend's normalized event stream,
    /// translates events to [`SessionEvent`]s, executes governed tools, appends
    /// results, and continues — respecting backend capabilities:
    ///
    /// - HQ-managed API primaries receive tool schemas and drive the tool loop.
    /// - Backend-managed CLI primaries receive **no** HQ tool schemas and end
    ///   their turn terminally (they run their own tools and return final text).
    ///
    /// Root backend selection is explicit and fixed for the run (never adaptive
    /// per turn); provider startup/pre-output failover is owned by
    /// [`ProviderChain`](crate::backend::ProviderChain). This loop owns only
    /// context compaction/overflow recovery and session-level limits.
    async fn run_turns(&mut self, stream: bool) -> Result<SessionResult> {
        let root_caps = self.backend.root_capabilities();
        let hq_managed_tools = root_caps.tools;
        // A streaming request only makes sense when the primary streams; a
        // buffered primary (CLI) always serves buffered so it stays selectable.
        let want_stream = stream && root_caps.streaming;

        let mut turn: u32 = 0;
        let mut last_text = String::new();
        // Tracks the turn at which a pre-output context overflow was already
        // recovered by compaction, so the retry doesn't loop forever if a single
        // compaction pass doesn't free enough room.
        let mut reactive_compacted_turn: Option<u32> = None;
        self.begin_credit_baseline();

        loop {
            // Check cancel flag — relay may have set it while the last tool ran.
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(SessionResult::Cancelled(self.build_cancel_summary()));
            }

            // A relay message arrived while the last tool ran — inject it as
            // the newest user turn before starting the next backend call.
            // Safe unconditionally here: no backend call has started yet this
            // iteration, so there's nothing to abort.
            let steer_text = self.pending_steer.lock().unwrap().take();
            if let Some(text) = steer_text {
                self.steer(&text);
                continue;
            }

            if let Some(result) = self.check_limits(&last_text) {
                return Ok(result);
            }

            let _ = self.check_compaction().await;

            // Start the backend stream. Startup failure is owned here only for
            // context overflow (compact + retry once); everything else — including
            // provider failover — is the backend/chain's responsibility.
            let event_stream = self
                .start_backend_turn(hq_managed_tools, want_stream)
                .await?;

            let mut out = self.consume_backend_stream(event_stream).await;

            if out.cancelled {
                return Ok(SessionResult::Cancelled(self.build_cancel_summary()));
            }

            if let Some(steer_text) = out.steered.take() {
                // Commit whatever streamed before the abort as a real assistant
                // turn so context isn't silently dropped, but never with
                // partial tool_calls: process_tool_results never ran on this
                // path, so any dangling tool_call id here would have no
                // paired tool-result message, and a provider that enforces
                // that pairing would reject the next request outright.
                if !out.content.is_empty() {
                    self.push_assistant(out.content.clone(), Vec::new());
                }
                self.steer(&steer_text);
                continue;
            }

            let stream_error = match self
                .handle_stream_error(&mut out, turn, &mut reactive_compacted_turn)
                .await?
            {
                StreamErrorStep::Retry => continue,
                StreamErrorStep::Proceed(stream_error) => stream_error,
            };
            let had_stream_error = stream_error.is_some();

            let usage = self.account_turn_usage(&out);
            let credits_after = self.begin_credit_read(&out.active_backend);
            let TurnOutput {
                content,
                tool_calls,
                active_backend,
                ..
            } = out;

            if !content.is_empty() {
                last_text = content.clone();
            }

            self.push_assistant(content.clone(), tool_calls.clone());

            // Terminal when: the model asked for no tools, the backend does not
            // drive our tool loop (CLI), or the turn ended in a post-output error.
            let terminal = tool_calls.is_empty() || !hq_managed_tools || had_stream_error;
            if terminal {
                // Empty-response retry guard — HQ-managed turns only. A blank
                // answer with no tools early in the run is usually a hiccup.
                if hq_managed_tools
                    && !had_stream_error
                    && is_early_blank_turn(turn, &content, &tool_calls)
                {
                    warn!("empty response from model on turn {}, retrying", turn);
                    self.emit(SessionEvent::Error(
                        "Empty response — retrying with a different provider...".into(),
                    ));
                    turn += 1;
                    continue;
                }
                let result =
                    self.finish_terminal_turn(turn, &active_backend, content, stream_error);
                self.emit_step_credits(turn, usage, credits_after).await;
                return Ok(result);
            }

            if let Some(cancelled) = self.run_tool_calls(&tool_calls).await {
                return Ok(cancelled);
            }

            turn += 1;
            self.emit(SessionEvent::TurnEnd { turn });
            self.emit_step_credits(turn, usage, credits_after).await;
        }
    }

    /// A terminal error delivered inside the stream. With no committed output
    /// it is terminal for the whole run and propagates as `Err` (mirrors a
    /// terminal auth failure at start). With committed output, the error is
    /// kept and the turn resolves to a non-`Complete` `Failed` result, so the
    /// partial text is never mistaken for a clean completion.
    async fn handle_stream_error(
        &mut self,
        out: &mut TurnOutput,
        turn: u32,
        reactive_compacted_turn: &mut Option<u32>,
    ) -> Result<StreamErrorStep> {
        let Some(err) = out.error.take() else {
            return Ok(StreamErrorStep::Proceed(None));
        };
        let message = err.to_string();
        if !out.produced_output {
            // A pre-output context overflow from the lazy provider chain
            // arrives here (not as a `start` failure). Mirror the start-time
            // recovery: compact once per turn and retry the turn rather than
            // failing the whole run.
            if is_backend_context_overflow(&err) && *reactive_compacted_turn != Some(turn) {
                warn!(
                    "reactive compaction: context overflow before output, compacting and retrying"
                );
                self.emit(SessionEvent::ContextOverflowRecovery);
                self.compact().await;
                *reactive_compacted_turn = Some(turn);
                return Ok(StreamErrorStep::Retry);
            }
            return Err(err);
        }
        self.emit_from(
            EventSource::Backend(out.active_backend.clone()),
            SessionEvent::Error(format!("stream: {err}")),
        );
        Ok(StreamErrorStep::Proceed(Some(message)))
    }

    /// Run a turn's tool calls, racing them against cancel. `Some` means the
    /// run was cancelled mid-call.
    ///
    /// Dropping the tool future on cancel drops whatever the tool is doing;
    /// whether an OS process dies depends on its spawn setting
    /// `kill_on_drop(true)`. Steer does not race here: a call in flight finishes.
    async fn run_tool_calls(&mut self, tool_calls: &[ToolCall]) -> Option<SessionResult> {
        let tool_results = tokio::select! {
            biased;
            _ = wait_for_cancel(&self.cancel) => {
                return Some(SessionResult::Cancelled(self.build_cancel_summary()));
            }
            r = self.execute_tools_parallel(tool_calls) => r,
        };
        self.process_tool_results(tool_calls, tool_results);
        None
    }

    fn push_assistant(&mut self, content: String, tool_calls: Vec<ToolCall>) {
        self.push_message(ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::Assistant,
            content,
            tool_calls,
            tool_call_id: None,
            reasoning_content: None,
        });
    }

    /// Price the turn, fold it into the session totals and emit `CostUpdate`.
    ///
    /// The *actual* backend model is resolved before pricing so cost, the
    /// CostUpdate event, budget tracking, and `last_resolved_model` all agree
    /// on the model this turn ran; a chain fallback or a provider-resolved id
    /// can differ from the configured alias.
    fn account_turn_usage(&mut self, out: &TurnOutput) -> StepUsage {
        let resolved_model = if out.model.is_empty() {
            self.config.model.clone()
        } else {
            self.last_resolved_model = Some(out.model.clone());
            out.model.clone()
        };

        // Estimate when the backend didn't report usage.
        let (input_tokens, output_tokens) =
            self.finalize_usage(out.input_tokens, out.output_tokens, &out.content);
        self.total_input_tokens += input_tokens as u64;
        self.total_output_tokens += output_tokens as u64;
        self.total_cache_read_tokens += out.cache_read_tokens as u64;
        self.total_cache_write_tokens += out.cache_write_tokens as u64;
        let usage = hq_llm::cost::Usage {
            input: input_tokens,
            output: output_tokens,
            cache_read: out.cache_read_tokens,
            cache_write: out.cache_write_tokens,
            reasoning: 0,
        };
        self.total_cost += hq_llm::cost::price_call(
            hq_llm::cost::ProviderClass::of_name(&out.active_backend).for_session_budget(),
            &resolved_model,
            &usage,
            None,
        )
        .usd;
        // Accounting is session-generated bookkeeping, not backend output.
        self.emit(SessionEvent::CostUpdate {
            total_usd: self.total_cost,
            input_tokens: self.total_input_tokens,
            output_tokens: self.total_output_tokens,
            model: resolved_model.clone(),
            is_fallback: out.is_fallback,
        });
        StepUsage {
            input_tokens,
            output_tokens,
            model: resolved_model,
        }
    }

    /// Emit the closing events of a terminal turn and build its result. A
    /// post-output stream error is a failed turn, not a completion: the
    /// partial text is preserved under a distinct `Failed` result.
    fn finish_terminal_turn(
        &mut self,
        turn: u32,
        active_backend: &str,
        content: String,
        stream_error: Option<String>,
    ) -> SessionResult {
        self.emit_from(
            EventSource::Backend(active_backend.to_string()),
            SessionEvent::TextDone(content.clone()),
        );
        self.emit(SessionEvent::TurnEnd { turn });
        match stream_error {
            Some(error) => SessionResult::Failed {
                partial: content,
                error,
            },
            None => SessionResult::Complete(content),
        }
    }

    /// Fill in token usage when the backend didn't report it, estimating input
    /// from the assembled messages and output from the produced content.
    fn finalize_usage(&self, input_tokens: u32, output_tokens: u32, content: &str) -> (u32, u32) {
        let input = if input_tokens == 0 {
            let est: usize = self
                .messages
                .iter()
                .map(|m| estimate_token_count(&m.content))
                .sum();
            let sys = self
                .system_prompt
                .as_ref()
                .map(|s| estimate_token_count(s))
                .unwrap_or(0);
            (est + sys) as u32
        } else {
            input_tokens
        };
        let output = if output_tokens == 0 && !content.is_empty() {
            estimate_token_count(content) as u32
        } else {
            output_tokens
        };
        (input, output)
    }

    /// Check budget and wall-clock limits. Returns `Some(SessionResult)` if
    /// the session should stop. There is deliberately no turn-count check
    /// here (FR-056): a session runs until natural completion, explicit
    /// cancellation, budget exhaustion, or a wall-clock/provider limit.
    pub(super) fn check_limits(&self, last_text: &str) -> Option<SessionResult> {
        if let Some(budget) = self.config.max_budget_usd
            && self.total_cost >= budget
        {
            warn!(spent = self.total_cost, budget, "USD budget exhausted");
            self.emit(SessionEvent::BudgetExhausted {
                spent: self.total_cost,
                budget,
            });
            return Some(SessionResult::BudgetExhausted(last_text.to_string()));
        }

        // Wall-clock time limit.
        if let Some(max_secs) = self.config.max_duration_secs
            && self.start_time.elapsed().as_secs() >= max_secs
        {
            let elapsed = self.start_time.elapsed();
            let secs = elapsed.as_secs();
            let msg = format!(
                "{last_text}\n\n[Session stopped: time limit reached after {}h {}m {}s]",
                secs / 3600,
                (secs % 3600) / 60,
                secs % 60
            );
            return Some(SessionResult::TimeLimitReached(msg));
        }

        None
    }

    /// Build the ChatRequest for a turn, including deferred tool catalog injection
    /// and plan-mode suffix.
    ///
    /// When `include_tools` is `false` (a backend-managed CLI primary that runs
    /// its own tools), no HQ tool definitions or deferred catalog are attached —
    /// the model is asked for a plain completion.
    pub(super) async fn build_turn_request(&self, include_tools: bool) -> ChatRequest {
        let (mut tool_defs, deferred_catalog) = if include_tools {
            let tools = self.tools.lock().await;
            (tools.active_definitions(), tools.deferred_catalog())
        } else {
            (Vec::new(), Vec::new())
        };

        if !deferred_catalog.is_empty() {
            tool_defs.push(hq_core::types::ToolDefinition {
                name: "tool_search".to_string(),
                description: "Search for and load deferred tool schemas by keyword. \
                    Use this to discover tools not shown in the initial tool list."
                    .to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Keyword to search tool names and descriptions"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Max tools to return (default: 5)"
                        }
                    }
                }),
            });
        }

        let mut request_messages = Vec::new();
        let plan_mode_suffix = if self.config.mode == SessionMode::Plan {
            Some(
                "\n\n[PLAN MODE] You are in plan mode. Do NOT make any file modifications. \
                 Only use read-only tools (read_file, find_files, grep, list_dir, web_search, \
                 web_fetch). Analyze the codebase and produce a plan.",
            )
        } else {
            None
        };
        let sys_content = match (&self.system_prompt, deferred_catalog.is_empty()) {
            (Some(sys), false) => {
                let catalog_lines: Vec<String> = deferred_catalog
                    .iter()
                    .map(|(name, hint)| format!("- {name}: {hint}"))
                    .collect();
                Some(format!(
                    "{sys}\n\n## Deferred Tools\nUse tool_search to load full schemas:\n{}",
                    catalog_lines.join("\n")
                ))
            }
            (Some(sys), true) => Some(sys.clone()),
            _ => None,
        };
        let sys_content = match (sys_content, plan_mode_suffix) {
            (Some(s), Some(suffix)) => Some(format!("{s}{suffix}")),
            (s, _) => s,
        };
        if let Some(content) = sys_content {
            request_messages.push(ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::System,
                content,
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: None,
            });
        }
        request_messages.extend(self.messages.clone());
        sanitize_tool_messages(&mut request_messages);

        ChatRequest {
            model: self.config.model.clone(),
            messages: request_messages,
            tools: tool_defs,
            temperature: self.config.temperature,
            max_tokens: self.config.max_tokens,
        }
    }

    /// Record a file path when the tool is a known write/edit tool.
    fn record_file_if_applicable(&mut self, tool_name: &str, input: &serde_json::Value) {
        let is_file_tool = matches!(
            tool_name,
            "edit" | "write" | "str_replace_editor" | "file_edit" | "file_write"
        );
        if !is_file_tool {
            return;
        }
        let path = input
            .get("path")
            .or_else(|| input.get("file_path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(p) = path
            && !self.files_touched.contains(&p)
        {
            self.files_touched.push(p);
        }
    }

    /// Process tool call results: emit events, microcompact, push messages,
    /// and inject context modifiers.
    pub(super) fn process_tool_results(
        &mut self,
        tool_calls: &[ToolCall],
        tool_results: Vec<(String, Option<String>)>,
    ) {
        let mut context_modifiers = Vec::new();

        for (tc, (result_text, ctx_mod)) in tool_calls.iter().zip(tool_results) {
            self.record_file_if_applicable(&tc.name, &tc.arguments);

            self.emit(SessionEvent::ToolEnd {
                tool_name: tc.name.clone(),
                tool_call_id: tc.id.clone(),
                result: result_text.clone(),
            });

            if let Some(modifier) = ctx_mod {
                context_modifiers.push(modifier);
            }

            let compacted_text = self.microcompact_result(&tc.name, result_text);

            self.push_message(ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Tool,
                content: compacted_text,
                tool_calls: Vec::new(),
                tool_call_id: Some(tc.id.clone()),
                reasoning_content: None,
            });

            self.tool_call_count += 1;
            // The agent maintained its skills itself; restart the review count.
            if tc.name == "skill_manage" {
                self.skill_review_mark = self.tool_call_count;
            }
        }
        // Folded into the one context message: a user turn between tool results is rejected.
        context_modifiers.extend(self.skill_reminders(tool_calls));

        if !context_modifiers.is_empty() {
            let annotation = format!("[CONTEXT] {}", context_modifiers.join("; "));
            debug!(annotation = %annotation, "injecting context modifiers");
            self.push_message(ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: annotation,
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: None,
            });
        }
    }

    /// Inject parent context into this session (fork mode).
    ///
    /// Prepends the parent's messages as a read-only context block, separated
    /// by a fork marker. The child sees the full parent conversation but cannot
    /// modify it.
    pub fn inject_fork_context(&mut self, parent_messages: Vec<ChatMessage>) {
        if parent_messages.is_empty() {
            return;
        }

        let fork_marker = ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: format!(
                "[FORK CONTEXT — {} parent messages inherited. Your task follows below.]",
                parent_messages.len()
            ),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        };

        let mut injected = parent_messages;
        injected.push(fork_marker);

        let mut injected_tokens: Vec<usize> =
            injected.iter().map(super::message_token_estimate).collect();

        injected.append(&mut self.messages);
        injected_tokens.append(&mut self.message_tokens);
        self.messages = injected;
        self.message_tokens = injected_tokens;
    }
}

/// Remove structural inconsistencies in the message list before sending to a provider.
///
/// Front-trimming during compaction can leave orphaned `role:Tool` messages (whose
/// `tool_calls` parent was removed) or dangling `role:Assistant` messages that carry
/// `tool_calls` with no following Tool response. Both patterns trigger API 400 errors
/// from providers that validate message ordering.
///
/// Pass 1: drop Tool messages that have no preceding assistant+tool_calls.
/// Pass 2: strip `tool_calls` from assistant messages whose responses were all dropped;
///         if nothing remains in the message, drop it entirely.
fn sanitize_tool_messages(messages: &mut Vec<ChatMessage>) {
    // Pass 1: drop orphaned Tool messages.
    let mut in_tool_batch = false;
    let mut cleaned: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    for m in messages.drain(..) {
        match m.role {
            MessageRole::Tool => {
                if in_tool_batch {
                    cleaned.push(m);
                }
                // else: orphaned — drop silently
            }
            MessageRole::Assistant if !m.tool_calls.is_empty() => {
                in_tool_batch = true;
                cleaned.push(m);
            }
            _ => {
                in_tool_batch = false;
                cleaned.push(m);
            }
        }
    }

    // Pass 2: strip dangling tool_calls (assistant with tool_calls but no following Tool).
    let n = cleaned.len();
    for i in 0..n {
        if cleaned[i].role == MessageRole::Assistant && !cleaned[i].tool_calls.is_empty() {
            let next_is_tool = cleaned
                .get(i + 1)
                .map(|m| m.role == MessageRole::Tool)
                .unwrap_or(false);
            if !next_is_tool {
                cleaned[i].tool_calls.clear();
            }
        }
    }
    // Drop assistant messages left with neither content nor tool_calls.
    cleaned.retain(|m| {
        m.role != MessageRole::Assistant || !m.content.is_empty() || !m.tool_calls.is_empty()
    });

    *messages = cleaned;
}

#[cfg(test)]
#[path = "loop_tests.rs"]
mod loop_tests;
