//! Chat Completions stream decoding into normalized chunks.

use anyhow::Result;
use async_openai::types::{CompletionUsage, CreateChatCompletionStreamResponse};
use futures::StreamExt;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio_stream::Stream;

use super::wire::classify_openai_error;
use crate::provider::{LlmError, StreamChunk};

/// Deserialize one raw SSE data payload into a typed stream chunk, patching
/// in a missing `object` field first: GitHub Copilot's `/chat/completions`
/// proxy omits it entirely (live-verified), but async-openai's
/// `CreateChatCompletionStreamResponse` declares it a required `String`, so
/// every chunk from Copilot would otherwise fail deserialization. A no-op
/// for compliant payloads that already carry the field.
pub(super) fn parse_stream_chunk(
    payload: &str,
) -> serde_json::Result<CreateChatCompletionStreamResponse> {
    let mut value: serde_json::Value = serde_json::from_str(payload)?;
    if let Some(obj) = value.as_object_mut() {
        obj.entry("object")
            .or_insert_with(|| serde_json::json!("chat.completion.chunk"));
    }
    serde_json::from_value(value)
}

/// Where the raw SSE reader leaves the provider-billed cost of the call, which the typed chunk
/// cannot carry. Set before the chunk holding `usage` is sent, so the consumer sees it in time.
pub(super) type CostSidecar = Arc<Mutex<Option<f64>>>;

/// The pinned, `Send` upstream stream of raw OpenAI stream chunks.
type OpenAiResponseStream = Pin<
    Box<
        dyn Stream<
                Item = std::result::Result<
                    CreateChatCompletionStreamResponse,
                    async_openai::error::OpenAIError,
                >,
            > + Send,
    >,
>;

/// Streaming state for [`finalize_openai_stream`]. Keeps the terminal `Done` /
/// truncation decision tied to whether a real `finish_reason` was ever seen.
enum OpenAiStreamPhase {
    /// Pumping upstream chunks. `buffer` holds normalized chunks mapped from the
    /// most recent upstream response but not yet yielded (one upstream chunk can
    /// map to several). `finish_seen` records whether any choice carried a real
    /// `finish_reason`; `last_model` tracks the provider-echoed model name.
    Active {
        inner: OpenAiResponseStream,
        buffer: VecDeque<Result<StreamChunk>>,
        finish_seen: bool,
        last_model: String,
        sidecar: Option<CostSidecar>,
    },
    /// Upstream drained — emit exactly one terminal marker (`Done` on a real
    /// finish, otherwise a typed truncation error) and stop.
    Terminal { finish_seen: bool },
    /// Fully consumed.
    Ended,
}

/// Wrap the raw async-openai stream so the terminal marker is state-driven:
/// `Done` is appended once, after any usage-only chunk, and **only** when a real
/// `finish_reason` was observed. A premature EOF (no `finish_reason`) instead
/// yields a typed [`LlmError::Network`] truncation error and never a `Done`, so
/// the unified consumer treats it as a failed turn rather than a clean finish.
pub(super) fn finalize_openai_stream<S>(
    inner: S,
    fallback_model: String,
) -> impl Stream<Item = Result<StreamChunk>> + Send
where
    S: Stream<
            Item = std::result::Result<
                CreateChatCompletionStreamResponse,
                async_openai::error::OpenAIError,
            >,
        > + Send
        + 'static,
{
    finalize_openai_stream_with_cost(inner, fallback_model, None)
}

/// [`finalize_openai_stream`] that also reports the provider-billed cost left in `sidecar`.
pub(super) fn finalize_openai_stream_with_cost<S>(
    inner: S,
    fallback_model: String,
    sidecar: Option<CostSidecar>,
) -> impl Stream<Item = Result<StreamChunk>> + Send
where
    S: Stream<
            Item = std::result::Result<
                CreateChatCompletionStreamResponse,
                async_openai::error::OpenAIError,
            >,
        > + Send
        + 'static,
{
    let phase = OpenAiStreamPhase::Active {
        inner: Box::pin(inner),
        buffer: VecDeque::new(),
        finish_seen: false,
        last_model: fallback_model,
        sidecar,
    };
    futures::stream::unfold(phase, advance_openai_stream)
}

/// Whether a [`StreamChunk`] represents committed output — mirrors
/// [`BackendEvent::is_output`](hq_agent) semantics one layer down, at the
/// raw provider stream, so a truncated-before-output stream can be retried
/// here instead of only failing over to a worse backend one layer up.
fn is_output_chunk(chunk: &StreamChunk) -> bool {
    matches!(
        chunk,
        StreamChunk::Text(_) | StreamChunk::Reasoning(_) | StreamChunk::ToolCallDelta { .. }
    )
}

/// Re-issue a streaming request once if it truncates (ends without a
/// `finish_reason`, via [`finalize_openai_stream`]) before producing any
/// output. DeepSeek's V4 endpoints have been observed dropping the SSE
/// connection mid-stream with no error frame — a bare connection blip
/// shouldn't cost a silent fail-over to a worse backend
/// ([`ProviderChain`](hq_agent)) when a single retry recovers it. Only
/// retries when *zero* output was seen: once any content/tool-call delta has
/// been produced, the buffered prelude is chained onto the live stream and
/// forwarded as-is, matching the chain's own "fail over only before output"
/// rule one layer up.
pub(super) async fn stream_with_truncation_retry<F, Fut>(
    mut make_stream: F,
) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<
            Output = Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>,
        >,
{
    let mut retried = false;
    let mut stream = make_stream().await?;
    loop {
        let mut prelude: Vec<Result<StreamChunk>> = Vec::new();
        loop {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    let output = is_output_chunk(&chunk);
                    prelude.push(Ok(chunk));
                    if output {
                        return Ok(Box::pin(tokio_stream::iter(prelude).chain(stream)));
                    }
                }
                Some(Err(e)) if !retried => {
                    tracing::warn!(
                        error = %e,
                        "stream truncated before any output, retrying once"
                    );
                    retried = true;
                    stream = make_stream().await?;
                    break;
                }
                Some(Err(e)) => {
                    return Ok(Box::pin(tokio_stream::iter(
                        prelude.into_iter().chain([Err(e)]),
                    )));
                }
                None => return Ok(Box::pin(tokio_stream::iter(prelude))),
            }
        }
    }
}

/// Advance the streaming state machine by one yielded item.
///
/// Drains any buffered normalized chunks first, then pulls the next upstream
/// chunk. On EOF (`None`) it transitions to [`OpenAiStreamPhase::Terminal`],
/// which emits `Done` when a `finish_reason` was seen or a typed truncation
/// error otherwise. An explicit upstream transport error terminates the stream:
/// after a clean finish it still resolves to `Done` (a trailing tail error must
/// not fail an already-complete turn); before any finish it surfaces the typed
/// error with no `Done`.
async fn advance_openai_stream(
    mut phase: OpenAiStreamPhase,
) -> Option<(Result<StreamChunk>, OpenAiStreamPhase)> {
    loop {
        match phase {
            OpenAiStreamPhase::Active {
                mut inner,
                mut buffer,
                mut finish_seen,
                mut last_model,
                sidecar,
            } => {
                if let Some(chunk) = buffer.pop_front() {
                    return Some((
                        chunk,
                        OpenAiStreamPhase::Active {
                            inner,
                            buffer,
                            finish_seen,
                            last_model,
                            sidecar,
                        },
                    ));
                }
                match inner.next().await {
                    Some(Ok(response)) => {
                        // The requested/resolved model (`fallback_model`, seeded into
                        // `last_model` at construction) is authoritative: it's what
                        // the router/chain actually resolved and dispatched to. Some
                        // OpenAI-compatible servers (notably local llama.cpp-family
                        // endpoints) echo back a stale or generic `model` field in
                        // their response envelope that does not reflect what's
                        // actually loaded/serving (e.g. a leftover alias from a
                        // previous config). Only ever use `response.model` to fill in
                        // when we don't already have a resolved model — never let it
                        // clobber a known-good one, or the turn-badge silently
                        // mislabels the backend that actually served the turn.
                        if last_model.is_empty() && !response.model.is_empty() {
                            last_model = response.model.clone();
                        }
                        if response.choices.iter().any(|c| c.finish_reason.is_some()) {
                            finish_seen = true;
                        }
                        buffer.extend(stream_response_to_chunks(&response, &last_model));
                        if let Some(billing) = response
                            .usage
                            .as_ref()
                            .and_then(|u| billing_chunk(u, sidecar.as_ref()))
                        {
                            buffer.push_back(Ok(billing));
                        }
                        phase = OpenAiStreamPhase::Active {
                            inner,
                            buffer,
                            finish_seen,
                            last_model,
                            sidecar,
                        };
                        continue;
                    }
                    Some(Err(e)) => {
                        if finish_seen {
                            // The response already finished; a trailing transport
                            // error (e.g. on the usage/`[DONE]` tail) must not fail
                            // an otherwise-complete turn — fall through to `Done`.
                            phase = OpenAiStreamPhase::Terminal { finish_seen: true };
                            continue;
                        }
                        return Some((
                            Err(classify_openai_error(&e).into()),
                            OpenAiStreamPhase::Ended,
                        ));
                    }
                    None => {
                        phase = OpenAiStreamPhase::Terminal { finish_seen };
                        continue;
                    }
                }
            }
            OpenAiStreamPhase::Terminal { finish_seen } => {
                let item = if finish_seen {
                    Ok(StreamChunk::Done)
                } else {
                    Err(LlmError::Network(
                        "OpenAI stream ended before a finish_reason (truncated response)"
                            .to_string(),
                    )
                    .into())
                };
                return Some((item, OpenAiStreamPhase::Ended));
            }
            OpenAiStreamPhase::Ended => return None,
        }
    }
}

/// Map one streamed response chunk into normalized [`StreamChunk`]s, *without* a
/// terminal `Done` (appended once at stream end by the caller).
///
/// Handles both the content-bearing chunks (delta text/tool calls + model info
/// on the finish chunk) and the trailing usage-only chunk that OpenAI emits when
/// `stream_options.include_usage` is set (empty `choices`, populated `usage`).
pub(super) fn stream_response_to_chunks(
    response: &CreateChatCompletionStreamResponse,
    model: &str,
) -> Vec<Result<StreamChunk>> {
    let mut out = Vec::new();
    if let Some(choice) = response.choices.first() {
        out.extend(stream_delta_to_chunks(
            &choice.delta,
            choice.finish_reason.is_some(),
            model,
        ));
    }
    // Usage arrives either on the finish chunk or as a separate usage-only chunk
    // (empty choices). Emit it before the terminal `Done` either way.
    if let Some(usage) = response.usage.as_ref() {
        out.push(Ok(usage_to_chunk(usage)));
    }
    out
}

/// Translate an OpenAI [`CompletionUsage`] into a normalized [`StreamChunk::Usage`].
///
/// `prompt_tokens` already includes cached prompt tokens (mirroring the buffered
/// path and [`calculate_cost_with_cache`](hq_llm::models::calculate_cost_with_cache)'s
/// assumption), and `prompt_tokens_details.cached_tokens` is the cache-read
/// split when the provider reports one. OpenAI-style APIs don't bill a separate
/// cache-write rate, so `cache_write_tokens` is zero.
pub(super) fn usage_to_chunk(usage: &CompletionUsage) -> StreamChunk {
    let cache_read_tokens = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|d| d.cached_tokens)
        .unwrap_or(0);
    StreamChunk::Usage {
        input_tokens: usage.prompt_tokens,
        output_tokens: usage.completion_tokens,
        cache_read_tokens,
        cache_write_tokens: 0,
    }
}

/// The billed cost and reasoning tokens that ride along with a `usage` payload, when it has any.
fn billing_chunk(usage: &CompletionUsage, sidecar: Option<&CostSidecar>) -> Option<StreamChunk> {
    let reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|d| d.reasoning_tokens)
        .unwrap_or(0);
    let cost_usd = sidecar.and_then(|cell| cell.lock().ok()?.take());
    (reasoning_tokens > 0 || cost_usd.is_some()).then_some(StreamChunk::Billing {
        cost_usd,
        reasoning_tokens,
    })
}

/// Flatten one streamed choice delta into normalized [`StreamChunk`]s.
///
/// A single upstream chunk can carry multiple parallel tool-call deltas (models
/// that emit parallel tool calls pack several entries into one `delta.tool_calls`
/// array). Every one is emitted — not just the first — preserving
/// tool-then-text-then-model order. `finished` signals this chunk carried a
/// `finish_reason`, so model info (when known) is appended last.
///
/// Note: the terminal [`StreamChunk::Done`] is intentionally **not** emitted
/// here. It is appended once at stream end by [`chat_stream`], after any
/// trailing usage-only chunk, so usage is never stranded behind an early `Done`.
pub(super) fn stream_delta_to_chunks(
    delta: &async_openai::types::ChatCompletionStreamResponseDelta,
    finished: bool,
    model: &str,
) -> Vec<Result<StreamChunk>> {
    let mut out: Vec<Result<StreamChunk>> = Vec::new();

    if let Some(ref tool_calls) = delta.tool_calls {
        for tc in tool_calls {
            if let Some(ref func) = tc.function {
                out.push(Ok(StreamChunk::ToolCallDelta {
                    index: tc.index as usize,
                    id: tc.id.clone(),
                    name: func.name.clone(),
                    arguments_delta: func.arguments.clone().unwrap_or_default(),
                }));
            }
        }
    }

    if let Some(ref content) = delta.content
        && !content.is_empty()
    {
        out.push(Ok(StreamChunk::Text(content.clone())));
    }

    if finished && !model.is_empty() {
        out.push(Ok(StreamChunk::ModelInfo(model.to_string())));
    }

    out
}
