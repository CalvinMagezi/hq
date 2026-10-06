//! Native Anthropic Messages API provider.
//!
//! A generic client for the Anthropic Messages API (`POST {base}/messages`),
//! implemented directly on top of the shared `reqwest` client plus `futures`
//! streaming — no Anthropic SDK is pulled in. It is configurable by API key and
//! base URL, so it targets both the canonical `api.anthropic.com` surface and
//! any gateway that speaks the same Messages wire format.
//!
//! Supported for HQ sessions:
//! - system extraction (system turns are hoisted into the top-level `system`
//!   field; the remaining user/assistant/tool turns keep their order),
//! - user / assistant / tool-result message mapping (tool results ride back as
//!   `tool_result` content blocks on a `user` turn), with coalescing of
//!   consecutive same-role turns as the Messages API expects,
//! - tool definitions (`input_schema`),
//! - buffered chat responses and SSE streaming: text deltas, thinking/reasoning
//!   deltas, `tool_use` start + `input_json_delta` assembly into
//!   [`StreamChunk::ToolCallDelta`], usage, model info, and a terminal `Done`,
//! - structured HTTP / network / auth / rate-limit / context errors.
//!
//! Protocol note: streaming [`StreamChunk::Usage`] now carries the prompt-cache
//! split (`cache_read_tokens`/`cache_write_tokens`) captured from the
//! `message_start` usage object, mirroring the buffered [`ChatResponse`] so cache
//! accounting is preserved across both paths. Replayed assistant `thinking`
//! blocks are intentionally omitted because the Messages API requires their
//! original signatures, which HQ's [`ChatMessage`] does not retain.

use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use std::pin::Pin;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;

use crate::provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};

mod stream;
mod wire;

pub(crate) use stream::{AnthropicStreamState, drain_sse};
pub(crate) use wire::{build_messages_body, classify_anthropic_error, parse_messages_response, parse_usage};

/// Canonical base URL for the public Anthropic Messages API.
pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com/v1";

/// Default `anthropic-version` header. Pinned to the stable Messages release.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The Messages API requires an explicit `max_tokens`; used when a caller omits
/// one so requests never fail validation for a missing field.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Native Anthropic Messages API provider.
///
/// Reuses the shared HTTP client so no new connection pool is introduced.
/// Construct with [`AnthropicProvider::new`] for the canonical endpoint, or
/// [`AnthropicProvider::new_with_base`] for a compatible gateway.
pub struct AnthropicProvider {
    http: reqwest::Client,
    api_key: String,
    /// Base URL without a trailing slash. `/messages` is appended per request.
    api_base: String,
    version: String,
    default_max_tokens: u32,
}

impl AnthropicProvider {
    /// Provider pointed at the canonical Anthropic Messages API.
    pub fn new(api_key: &str) -> Self {
        Self::new_with_base(api_key, ANTHROPIC_BASE_URL)
    }

    /// Provider pointed at an explicit base URL (the canonical API or a gateway
    /// that speaks the same Messages wire format).
    pub fn new_with_base(api_key: &str, base_url: &str) -> Self {
        Self {
            http: crate::http::SHARED_HTTP_CLIENT.clone(),
            api_key: api_key.to_string(),
            api_base: base_url.trim_end_matches('/').to_string(),
            version: ANTHROPIC_VERSION.to_string(),
            default_max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    fn messages_url(&self) -> String {
        format!("{}/messages", self.api_base)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let body = build_messages_body(request, self.default_max_tokens, false);

        let resp = self
            .http
            .post(self.messages_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.version)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::from_request_error(&e))?;

        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| LlmError::Other(anyhow::anyhow!("failed to read response: {e}")))?;

        if !(200..300).contains(&status) {
            return Err(classify_anthropic_error(status, &bytes).into());
        }

        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
            LlmError::Other(anyhow::anyhow!("failed to parse Anthropic response: {e}"))
        })?;

        let message = parse_messages_response(&json)?;
        let model = json
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&request.model)
            .to_string();

        let (input_tokens, output_tokens, cache_read_tokens, cache_write_tokens) =
            parse_usage(json.get("usage"));

        Ok(ChatResponse {
            message,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            model,
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let body = build_messages_body(request, self.default_max_tokens, true);

        let resp = self
            .http
            .post(self.messages_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.version)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::from_request_error(&e))?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let bytes = resp.bytes().await.unwrap_or_default();
            return Err(classify_anthropic_error(status, &bytes).into());
        }

        // Pump the SSE byte stream on a task and forward normalized chunks over a
        // bounded channel so backpressure flows to the transport. The line
        // buffer preserves frames split across byte-chunk boundaries.
        //
        // Cancellation: the receiver ([`ReceiverStream`]) is dropped as soon as a
        // consumer stops polling (turn cancelled, timeout, early `Done`). A plain
        // `byte_stream.next().await` would keep this task parked on a hung or
        // slow-trickling transport until the socket itself times out, leaking the
        // connection. Racing `tx.closed()` against every `next()` makes the drop
        // abort the pump immediately: the future returns, `byte_stream` (and its
        // underlying reqwest response) is dropped, and the transport is torn down.
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamChunk>>(64);
        tokio::spawn(async move {
            let mut byte_stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut state = AnthropicStreamState::default();

            loop {
                let next = tokio::select! {
                    biased;
                    // Receiver gone: stop pumping now and drop the transport.
                    _ = tx.closed() => return,
                    next = byte_stream.next() => next,
                };

                match next {
                    Some(Ok(bytes)) => {
                        buf.extend_from_slice(&bytes);
                        for chunk in drain_sse(&mut buf, &mut state) {
                            let is_err = chunk.is_err();
                            if tx.send(chunk).await.is_err() {
                                return;
                            }
                            if is_err {
                                return;
                            }
                        }
                        if state.done_emitted {
                            return;
                        }
                    }
                    Some(Err(e)) => {
                        let _ = tx.send(Err(LlmError::from_request_error(&e).into())).await;
                        return;
                    }
                    None => {
                        // Stream closed without an explicit `message_stop`. This
                        // is a truncated turn, not a clean completion: surface a
                        // typed, retryable transport error and — crucially — do
                        // NOT synthesize a terminal `Done`. Fabricating `Done`
                        // here would let the session mistake a cut-off stream for
                        // a finished one; the unified consumer instead treats a
                        // missing `Done` (or this explicit error) as a failure and
                        // preserves any partial output under `SessionResult::Failed`.
                        if !state.done_emitted {
                            let _ = tx
                                .send(Err(LlmError::Network(
                                    "Anthropic stream closed before message_stop (truncated response)"
                                        .to_string(),
                                )
                                .into()))
                                .await;
                        }
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests;
