//! Backend turn startup (with its pre-output retries) and stream consumption.

use anyhow::Result;
use hq_core::types::{EventSource, SessionEvent, ToolCall};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_stream::StreamExt;
use tracing::{debug, warn};

use super::context::estimate_token_count;
use super::{AgentSession, ToolCallBuilder};
use crate::backend::{BackendError, BackendEvent, BackendEventStream, BackendRequest};

/// Whatever a single turn's backend stream produced, collected for the engine.
pub(super) struct TurnOutput {
    /// Assistant answer text (deltas concatenated, or a buffered `Message`).
    pub(super) content: String,
    /// Reconstructed tool calls (empty for terminal/CLI turns).
    pub(super) tool_calls: Vec<ToolCall>,
    /// Reported input tokens (0 when the backend didn't report usage).
    pub(super) input_tokens: u32,
    /// Reported output tokens (0 when the backend didn't report usage).
    pub(super) output_tokens: u32,
    /// Prompt tokens served from the provider cache (buffered API path only).
    pub(super) cache_read_tokens: u32,
    /// Prompt tokens written into the provider cache this turn.
    pub(super) cache_write_tokens: u32,
    /// Reasoning tokens the provider reported (already inside `output_tokens`).
    pub(super) reasoning_tokens: u32,
    /// What the provider says it billed for the turn, when it said.
    pub(super) provider_cost_usd: Option<f64>,
    /// Concrete model the backend used (empty when unreported).
    pub(super) model: String,
    /// The backend the chain committed to (the primary's name, or a fallback's).
    /// Backend-origin events are tagged with this identity.
    pub(super) active_backend: String,
    /// Whether any committed output event was seen (text/reasoning/tool call).
    pub(super) produced_output: bool,
    /// A terminal error delivered inside the stream (post-`start`).
    pub(super) error: Option<anyhow::Error>,
    /// The root cancel flag tripped while consuming the stream.
    pub(super) cancelled: bool,
    /// A relay steer message arrived while the stream was in flight, carrying
    /// the text to inject. The stream was aborted the same way `cancelled`
    /// aborts it, but the run continues rather than terminating.
    pub(super) steered: Option<String>,
    /// Set when the chain reported at least one pre-output backend failure
    /// this turn — `active_backend` may then be a fallback, not the primary.
    pub(super) is_fallback: bool,
}

/// Reconstruct concrete [`ToolCall`]s from accumulated streaming deltas.
pub(super) fn builders_into_tool_calls(builders: Vec<ToolCallBuilder>) -> Vec<ToolCall> {
    builders
        .into_iter()
        .filter(|b| !b.id.is_empty())
        .map(|b| ToolCall {
            id: b.id,
            name: b.name,
            // A no-argument tool call accumulates an empty string (no
            // input_json_delta frames ever arrive for it); malformed/truncated
            // JSON is the other failure mode. Either way, `arguments` always
            // becomes `tool_use.input` on the wire — every consumer (the
            // Anthropic Messages API in particular) requires that to be a JSON
            // object, so the fallback must be `{}`, never a bare string.
            arguments: serde_json::from_str(&b.arguments).unwrap_or_else(|_| serde_json::json!({})),
        })
        .collect()
}

/// Lift a typed [`BackendError`] into `anyhow` while preserving its Display
/// (so downstream `.to_string()` checks still match the underlying message).
fn backend_error_to_anyhow(err: BackendError) -> anyhow::Error {
    anyhow::Error::new(err)
}

/// Jittered exponential backoff for legacy/single-backend start retries.
///
/// Mirrors the retired `call_with_retry` behavior: `retry_base_delay * 2^attempt`
/// with a 0.5x–1.5x jitter, so `SessionConfig::retry_base_delay` keeps its meaning.
fn retry_backoff(base: std::time::Duration, attempt: u32) -> std::time::Duration {
    use hq_core::middleware::{exponential_delay_floor, jittered_duration};
    let base_delay = exponential_delay_floor(base, attempt, None);
    jittered_duration(base_delay, &mut rand::thread_rng(), 0.5, 1.5)
}

/// Resolve when the cancel flag is set. Polls at a short interval so a
/// long-blocking backend poll (e.g. a CLI subprocess) can still be interrupted;
/// dropping the backend stream then aborts the in-flight work.
pub(super) async fn wait_for_cancel(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Resolve when a steer message is dropped in, returning its text. Same
/// polling shape as `wait_for_cancel`, for the same reason.
async fn wait_for_steer(pending: &std::sync::Mutex<Option<String>>) -> String {
    loop {
        if let Some(text) = pending.lock().unwrap().take() {
            return text;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

impl AgentSession {
    /// Begin one backend turn: build the request (with or without HQ tool
    /// schemas), then start the normalized stream.
    ///
    /// Recoveries owned here, all strictly **before any output or tool
    /// side-effects**:
    /// - context-overflow compaction + a single retry (always), and
    /// - for a legacy/single backend only, `max_retries` jittered-backoff retries
    ///   on failoverable startup errors plus a one-time switch to
    ///   `fallback_model`. A [`ProviderChain`](crate::backend::ProviderChain) is
    ///   skipped here — it owns its own pre-output failover, so the session must
    ///   not double-retry it.
    pub(super) async fn start_backend_turn(
        &mut self,
        hq_managed_tools: bool,
        want_stream: bool,
    ) -> Result<BackendEventStream> {
        let backend = self.backend.clone();
        // Only single backends get session-level retries; a chain fails over on
        // its own (retrying it here would duplicate that failover).
        let session_retries = !backend.owns_failover();
        let max_retries = self.config.max_retries;

        let mut attempt = 0u32;
        let mut tried_fallback = false;

        loop {
            let mut request = self.build_turn_request(hq_managed_tools).await;
            self.tool_schema_tokens = serde_json::to_string(&request.tools)
                .map(|s| estimate_token_count(&s))
                .unwrap_or(0);
            // After exhausting retries on the primary model, run a fresh round of
            // attempts against the configured fallback model.
            if tried_fallback && let Some(fallback) = self.config.fallback_model.as_ref() {
                request.model = fallback.clone();
            }
            let backend_request = BackendRequest::from_chat(&request, want_stream);
            let ctx = self.session_context();

            match hq_llm::SESSION_CONTEXT
                .scope(ctx, backend.start(&backend_request))
                .await
            {
                Ok(stream) => return Ok(stream),
                Err(BackendError::ContextOverflow(_)) => {
                    warn!(
                        "reactive compaction: context overflow at start, compacting and retrying"
                    );
                    self.emit(SessionEvent::ContextOverflowRecovery);
                    self.compact().await;
                    let retry_request = self.build_turn_request(hq_managed_tools).await;
                    let retry_backend_request =
                        BackendRequest::from_chat(&retry_request, want_stream);
                    let retry_ctx = self.session_context();
                    return hq_llm::SESSION_CONTEXT
                        .scope(retry_ctx, backend.start(&retry_backend_request))
                        .await
                        .map_err(backend_error_to_anyhow);
                }
                Err(e) => {
                    // Legacy retry/backoff, single backends only, and only on
                    // failoverable (transient / auth / unavailable) startup
                    // errors — i.e. before any output. Keeps
                    // `max_retries`/`retry_base_delay`/`fallback_model`
                    // meaningful and `RetryAttempt` events flowing.
                    if session_retries && e.is_failoverable() {
                        if attempt < max_retries {
                            let delay = retry_backoff(self.config.retry_base_delay, attempt);
                            self.emit(SessionEvent::RetryAttempt {
                                attempt: attempt + 1,
                                max_retries,
                                delay_ms: delay.as_millis() as u64,
                                error: e.to_string(),
                            });
                            tokio::time::sleep(delay).await;
                            attempt += 1;
                            continue;
                        }
                        if !tried_fallback && self.config.fallback_model.is_some() {
                            warn!(
                                fallback = ?self.config.fallback_model,
                                "primary model exhausted retries, switching to fallback model"
                            );
                            tried_fallback = true;
                            attempt = 0;
                            continue;
                        }
                    }
                    return Err(backend_error_to_anyhow(e));
                }
            }
        }
    }

    /// Consume one turn's normalized event stream into a [`TurnOutput`], emitting
    /// live [`SessionEvent`]s as events arrive and assembling tool-call deltas.
    ///
    /// Cooperative cancellation: the root cancel flag is checked before every
    /// event (racing a long backend poll), and setting it drops the stream —
    /// which aborts in-flight backend work (`kill_on_drop` for CLI children).
    pub(super) async fn consume_backend_stream(
        &self,
        mut stream: BackendEventStream,
    ) -> TurnOutput {
        let cancel = self.cancel.clone();
        let steer = self.pending_steer.clone();

        let mut content = String::new();
        let mut tool_call_builders: Vec<ToolCallBuilder> = Vec::new();
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;
        let mut cache_read_tokens = 0u32;
        let mut cache_write_tokens = 0u32;
        let mut reasoning_tokens = 0u32;
        let mut provider_cost_usd: Option<f64> = None;
        let mut model = String::new();
        // Default to the root backend's name; a ProviderChain overrides this with
        // the actually-selected backend via a `BackendSelected` event so fallback
        // identity is observable on backend-origin envelopes.
        let mut active_backend = self.backend.name().to_string();
        let mut produced_output = false;
        let mut error: Option<anyhow::Error> = None;
        let mut cancelled = false;
        let mut steered: Option<String> = None;
        // Set when the chain reported at least one pre-output backend failure
        // this turn — the committed `active_backend` may then be a fallback,
        // not the declared primary.
        let mut had_failover = false;
        // A well-behaved backend terminates its turn with `BackendEvent::Done`.
        // If the stream is instead exhausted (`next()` returns `None`) without a
        // `Done`, the turn was truncated — tracked here so it is surfaced as a
        // typed error rather than silently accepted as a clean completion.
        let mut saw_done = false;

        loop {
            let next = tokio::select! {
                biased;
                _ = wait_for_cancel(&cancel) => {
                    cancelled = true;
                    break;
                }
                text = wait_for_steer(&steer) => {
                    steered = Some(text);
                    break;
                }
                event = stream.next() => event,
            };

            let Some(item) = next else { break };

            match item {
                Ok(BackendEvent::TextDelta(delta)) => {
                    if !delta.is_empty() {
                        produced_output = true;
                        content.push_str(&delta);
                        self.emit_from(
                            EventSource::Backend(active_backend.clone()),
                            SessionEvent::TextDelta(delta),
                        );
                    }
                }
                Ok(BackendEvent::Message(text)) => {
                    // Buffered final answer: surface as a single text delta —
                    // one real event, never fabricated token-by-token streaming.
                    if !text.is_empty() {
                        produced_output = true;
                        content.push_str(&text);
                        self.emit_from(
                            EventSource::Backend(active_backend.clone()),
                            SessionEvent::TextDelta(text),
                        );
                    }
                }
                Ok(BackendEvent::ReasoningDelta(delta)) => {
                    produced_output = true;
                    self.emit_from(
                        EventSource::Backend(active_backend.clone()),
                        SessionEvent::Reasoning(delta),
                    );
                }
                Ok(BackendEvent::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_delta,
                }) => {
                    produced_output = true;
                    while tool_call_builders.len() <= index {
                        tool_call_builders.push(ToolCallBuilder::default());
                    }
                    let builder = &mut tool_call_builders[index];
                    if let Some(id) = id {
                        builder.id = id;
                    }
                    if let Some(name) = name {
                        builder.name = name;
                    }
                    builder.arguments.push_str(&arguments_delta);
                }
                Ok(BackendEvent::Progress(note)) => {
                    // A buffered/CLI lifecycle note — visible progress, not
                    // committed output. Surfaced so a CLI harness turn shows life.
                    self.emit_from(
                        EventSource::Backend(active_backend.clone()),
                        SessionEvent::ToolProgress {
                            tool_name: "backend".to_string(),
                            tool_call_id: "backend".to_string(),
                            message: note,
                        },
                    );
                }
                Ok(BackendEvent::Usage {
                    input_tokens: it,
                    output_tokens: ot,
                    cache_read_tokens: cr,
                    cache_write_tokens: cw,
                }) => {
                    input_tokens = it;
                    output_tokens = ot;
                    cache_read_tokens = cr;
                    cache_write_tokens = cw;
                }
                Ok(BackendEvent::Billing {
                    cost_usd,
                    reasoning_tokens: rt,
                }) => {
                    provider_cost_usd = cost_usd;
                    reasoning_tokens = rt;
                }
                Ok(BackendEvent::ModelInfo(m)) => {
                    model = m;
                }
                Ok(BackendEvent::BackendSelected(name)) => {
                    // The chain committed to this backend (may be a fallback);
                    // tag subsequent backend-origin events with its identity.
                    active_backend = name;
                }
                Ok(BackendEvent::Failover(failed_backend)) => {
                    debug!(backend = %failed_backend, "provider chain failed over this turn");
                    had_failover = true;
                }
                Ok(BackendEvent::Done) => {
                    saw_done = true;
                    break;
                }
                Err(e) => {
                    let msg = e.to_string();
                    debug!(error = %msg, "backend stream error");
                    error = Some(backend_error_to_anyhow(e));
                    break;
                }
            }
        }

        // Truncation guard: the stream ended (EOF) without a terminal `Done` and
        // without an explicit error or cancellation. Treat this as a typed
        // backend truncation so the turn resolves to `Failed` (preserving any
        // partial output) rather than being mistaken for a clean completion.
        if !cancelled && steered.is_none() && error.is_none() && !saw_done {
            let detail = if produced_output {
                "backend stream ended without a completion marker (truncated response)"
            } else {
                "backend stream ended without producing output or a completion marker"
            };
            error = Some(backend_error_to_anyhow(BackendError::Truncated(
                detail.to_string(),
            )));
        }

        TurnOutput {
            content,
            tool_calls: builders_into_tool_calls(tool_call_builders),
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            reasoning_tokens,
            provider_cost_usd,
            model,
            active_backend,
            produced_output,
            error,
            cancelled,
            steered,
            is_fallback: had_failover,
        }
    }
}
