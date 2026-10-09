//! Chat and streaming transport: the LlmProvider impl, Responses POST, and raw SSE.

use anyhow::{Context, Result};
use async_openai::types::{ChatCompletionStreamOptions, CreateChatCompletionStreamResponse};
use async_trait::async_trait;
use futures::StreamExt;
use std::pin::Pin;
use tokio_stream::Stream;

use super::stream::{
    CostSidecar, finalize_openai_stream, finalize_openai_stream_with_cost, parse_stream_chunk,
    stream_with_truncation_retry,
};
use super::wire::{build_request, classify_openai_error, parse_flexible_response};
use super::{
    COPILOT_INTEGRATOR_FLAP_MAX_ATTEMPTS, COPILOT_INTEGRATOR_FLAP_RETRY_DELAY, OpenRouterProvider,
};
use crate::provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};

fn with_referer(
    builder: reqwest::RequestBuilder,
    referer: Option<&str>,
) -> reqwest::RequestBuilder {
    match referer {
        Some(r) => builder.header("HTTP-Referer", r),
        None => builder,
    }
}

#[async_trait]
impl LlmProvider for OpenRouterProvider {
    fn name(&self) -> &str {
        "openrouter"
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        if self.responses_api {
            let body = crate::responses::build_body(request, false);
            let json: serde_json::Value = self
                .post_responses(&body)
                .await?
                .json()
                .await
                .map_err(|e| LlmError::Network(format!("failed to read response: {e}")))?;
            return crate::responses::parse_response(&json, &request.model);
        }
        // Build the request body using async-openai types for correct serialization,
        // but send via raw reqwest to handle non-standard response fields from
        // models like Qwen (reasoning), Kimi (reasoning_details), DeepSeek, etc.
        let oai_request = build_request(request)?;
        let mut body = serde_json::to_value(&oai_request).context("serialize request")?;
        self.mutate_body_for_endpoint(&mut body, request);

        let (api_base, auth_token) = self.copilot_aware_credential().await;
        let url = format!("{api_base}/chat/completions");
        let mut attempt = 0u32;
        let json: serde_json::Value = loop {
            attempt += 1;
            let builder = with_referer(
                self.http
                    .post(&url)
                    .header("Authorization", format!("Bearer {auth_token}"))
                    .header("Content-Type", "application/json")
                    .header("X-Title", "Agent HQ"),
                hq_core::config::http_referer().as_deref(),
            );
            let resp = self
                .attach_copilot_headers(builder)
                .json(&body)
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        LlmError::Network(format!("request timeout: {e}"))
                    } else {
                        LlmError::Network(format!("connection error: {e}"))
                    }
                })?;

            let status = resp.status().as_u16();
            if status == 429 {
                return Err(LlmError::RateLimit { retry_after: None }.into());
            }
            if status == 402 {
                return Err(LlmError::Auth {
                    status: 402,
                    message: "Model requires payment or insufficient credits".to_string(),
                }
                .into());
            }
            if status == 401 || status == 403 {
                return Err(LlmError::Auth {
                    status,
                    message: "Unauthorized".to_string(),
                }
                .into());
            }
            if (500..600).contains(&status) {
                return Err(LlmError::ServerError {
                    status,
                    message: format!("OpenRouter returned {status}"),
                }
                .into());
            }

            let parsed: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| LlmError::Other(anyhow::anyhow!("failed to read response: {e}")))?;

            // Check for API-level errors in the response body
            if let Some(err) = parsed.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                let code = err
                    .get("code")
                    .and_then(|c| c.as_str().or_else(|| c.as_i64().map(|_| "")).or(Some("")))
                    .unwrap_or("");
                if msg.contains("rate limit") || code == "429" {
                    return Err(LlmError::RateLimit { retry_after: None }.into());
                }
                // See COPILOT_INTEGRATOR_FLAP_MAX_ATTEMPTS's doc comment: this
                // specific error is live-verified non-deterministic per
                // request, not caused by request content.
                if code == "model_not_available_for_integrator"
                    && attempt < COPILOT_INTEGRATOR_FLAP_MAX_ATTEMPTS
                {
                    tracing::warn!(
                        attempt,
                        "Copilot's entitlement check rejected the model this attempt — retrying"
                    );
                    tokio::time::sleep(COPILOT_INTEGRATOR_FLAP_RETRY_DELAY).await;
                    continue;
                }
                return Err(LlmError::Other(anyhow::anyhow!("API error: {msg}")).into());
            }
            break parsed;
        };

        // Parse response flexibly — handle reasoning models, null content, etc.
        let message = parse_flexible_response(&json)?;
        let model = json
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&request.model)
            .to_string();

        let usage = json.get("usage");
        let input_tokens = usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        let output_tokens = usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        // DeepSeek native API: prompt_cache_hit_tokens directly on usage.
        // OpenAI-style (OpenRouter): nested in prompt_tokens_details.cached_tokens.
        let cache_read_tokens = usage
            .and_then(|u| {
                u.get("prompt_cache_hit_tokens")
                    .and_then(|t| t.as_u64())
                    .filter(|&n| n > 0)
                    .or_else(|| {
                        u.get("prompt_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(|t| t.as_u64())
                    })
            })
            .unwrap_or(0) as u32;
        let cache_miss_tokens = usage
            .and_then(|u| u.get("prompt_cache_miss_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        let reasoning_tokens = usage
            .and_then(|u| u.pointer("/completion_tokens_details/reasoning_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        // OpenRouter reports what it charged on every response; other providers omit it.
        let provider_cost_usd = usage.and_then(|u| u.get("cost")).and_then(|c| c.as_f64());
        if cache_read_tokens > 0 || cache_miss_tokens > 0 {
            tracing::debug!(
                "[LLM] cache: {} hit / {} miss tokens",
                cache_read_tokens,
                cache_miss_tokens
            );
        }

        Ok(ChatResponse {
            message,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens: 0,
            reasoning_tokens,
            provider_cost_usd,
            model,
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        if self.responses_api {
            return self.responses_stream(request).await;
        }
        let mut oai_request = build_request(request)?;
        oai_request.stream = Some(true);
        // Ask for a terminal usage chunk (empty `choices`, populated `usage`).
        // Supported by OpenAI and the vast majority of compatible providers;
        // those that ignore it simply omit `usage`, which the mapper tolerates.
        oai_request.stream_options = Some(ChatCompletionStreamOptions {
            include_usage: true,
        });

        // Kimi/Moonshot endpoints need the same body fixups as the buffered
        // path (thinking effort, temperature clamp) — the typed async-openai
        // client cannot carry extra fields, so stream raw SSE into the same
        // finalizer instead. Copilot needs the raw path for a different
        // reason (see `is_copilot_endpoint`), not body fixups.
        if self.is_kimi_family() || self.is_copilot_endpoint() {
            let mut body = serde_json::to_value(&oai_request).context("serialize request")?;
            self.mutate_body_for_endpoint(&mut body, request);
            let inner = self.raw_sse_stream(body, None).await?;
            return Ok(Box::pin(finalize_openai_stream(
                inner,
                request.model.clone(),
            )));
        }

        // OpenRouter bills inline: the final chunk's `usage.cost` is what the call cost. The typed
        // client drops it, so read the SSE ourselves and hand the cost over in a sidecar.
        if self.is_openrouter_endpoint() {
            let mut body = serde_json::to_value(&oai_request).context("serialize request")?;
            self.mutate_body_for_endpoint(&mut body, request);
            return stream_with_truncation_retry(|| {
                let body = body.clone();
                let model = request.model.clone();
                async move {
                    let sidecar = CostSidecar::default();
                    let inner = self.raw_sse_stream(body, Some(sidecar.clone())).await?;
                    Ok(Box::pin(finalize_openai_stream_with_cost(
                        inner,
                        model,
                        Some(sidecar),
                    ))
                        as Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>)
                }
            })
            .await;
        }

        // A terminal `Done` is emitted **only** after a real `finish_reason` is
        // observed, and only once — after any trailing usage-only chunk, so usage
        // is never stranded behind an early `Done`. If the upstream stream instead
        // ends (EOF) before any `finish_reason`, the turn was truncated: a typed
        // network error is surfaced (never a synthetic `Done`) so the session
        // resolves to `Failed` and preserves the partial output — unless a retry
        // (see `stream_with_truncation_retry`) recovers it first.
        stream_with_truncation_retry(|| {
            let client = self.client.clone();
            let oai_request = oai_request.clone();
            let model = request.model.clone();
            async move {
                let stream = client
                    .chat()
                    .create_stream(oai_request)
                    .await
                    .map_err(|e| classify_openai_error(&e))?;
                Ok(Box::pin(finalize_openai_stream(stream, model))
                    as Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>)
            }
        })
        .await
    }
}

impl OpenRouterProvider {
    /// POST a Responses body, retrying Copilot's integrator flap like the
    /// chat-completions path does. Returns the 2xx response.
    async fn post_responses(&self, body: &serde_json::Value) -> Result<reqwest::Response> {
        let (api_base, auth_token) = self.copilot_aware_credential().await;
        let url = format!("{api_base}/responses");
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let builder = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {auth_token}"))
                .header("Content-Type", "application/json");
            let resp = self
                .attach_copilot_headers(builder)
                .json(body)
                .send()
                .await
                .map_err(|e| LlmError::from_request_error(&e))?;
            let status = resp.status().as_u16();
            if (200..300).contains(&status) {
                return Ok(resp);
            }
            let text = resp.text().await.unwrap_or_default();
            if text.contains("model_not_available_for_integrator")
                && attempt < COPILOT_INTEGRATOR_FLAP_MAX_ATTEMPTS
            {
                tokio::time::sleep(COPILOT_INTEGRATOR_FLAP_RETRY_DELAY).await;
                continue;
            }
            return Err(crate::responses::classify_error(status, &text).into());
        }
    }

    /// Stream a Responses turn. A stream that ends before `response.completed`
    /// is an error, never a synthetic `Done`.
    async fn responses_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let body = crate::responses::build_body(request, true);
        let resp = self.post_responses(&body).await?;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamChunk>>(64);
        tokio::spawn(async move {
            let mut state = crate::responses::StreamState::default();
            let mut byte_stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            loop {
                let next = tokio::select! {
                    biased;
                    _ = tx.closed() => return,
                    next = byte_stream.next() => next,
                };
                let bytes = match next {
                    Some(Ok(bytes)) => bytes,
                    Some(Err(e)) => {
                        let _ = tx
                            .send(Err(LlmError::Network(format!("transport: {e}")).into()))
                            .await;
                        return;
                    }
                    None => break,
                };
                buf.extend_from_slice(&bytes);
                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line);
                    let Some(payload) = line.trim().strip_prefix("data:") else {
                        continue;
                    };
                    let chunks = match state.on_event(payload.trim()) {
                        Ok(chunks) => chunks,
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
                            return;
                        }
                    };
                    for chunk in chunks {
                        if tx.send(Ok(chunk)).await.is_err() {
                            return;
                        }
                    }
                    if state.completed() {
                        return;
                    }
                }
            }
            let _ = tx
                .send(Err(LlmError::Network(
                    "Responses stream ended before response.completed".to_string(),
                )
                .into()))
                .await;
        });
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    /// POST a raw chat-completions body with `stream: true` and pump the SSE
    /// response into typed stream chunks. Mirrors the Anthropic provider's
    /// pump: bounded channel for backpressure, `tx.closed()` race so a dropped
    /// consumer tears down the transport immediately.
    async fn raw_sse_stream(
        &self,
        body: serde_json::Value,
        sidecar: Option<CostSidecar>,
    ) -> Result<
        tokio_stream::wrappers::ReceiverStream<
            std::result::Result<
                CreateChatCompletionStreamResponse,
                async_openai::error::OpenAIError,
            >,
        >,
    > {
        use async_openai::error::OpenAIError;

        let (api_base, auth_token) = self.copilot_aware_credential().await;
        let url = format!("{api_base}/chat/completions");
        let mut attempt = 0u32;
        let resp = loop {
            attempt += 1;
            let builder = self
                .http
                .post(&url)
                .header("Authorization", format!("Bearer {auth_token}"))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream");
            let resp = self
                .attach_copilot_headers(builder)
                .json(&body)
                .send()
                .await
                .map_err(|e| LlmError::Network(format!("connection error: {e}")))?;

            let status = resp.status().as_u16();
            if status == 429 {
                return Err(LlmError::RateLimit { retry_after: None }.into());
            }
            if status == 402 {
                return Err(LlmError::Auth {
                    status: 402,
                    message: "Model requires payment or insufficient credits".to_string(),
                }
                .into());
            }
            if status == 401 || status == 403 {
                return Err(LlmError::Auth {
                    status,
                    message: "Unauthorized".to_string(),
                }
                .into());
            }
            if !(200..300).contains(&status) {
                let text = resp.text().await.unwrap_or_default();
                let is_integrator_flap =
                    status == 400 && text.contains("model_not_available_for_integrator");
                if is_integrator_flap && attempt < COPILOT_INTEGRATOR_FLAP_MAX_ATTEMPTS {
                    tracing::warn!(
                        attempt,
                        "Copilot's entitlement check rejected the model this attempt \
                         (non-deterministic per request, not caused by this request's \
                         content) — retrying"
                    );
                    tokio::time::sleep(COPILOT_INTEGRATOR_FLAP_RETRY_DELAY).await;
                    continue;
                }
                return Err(LlmError::ServerError {
                    status,
                    message: format!("streaming request failed: {}", &text[..text.len().min(300)]),
                }
                .into());
            }
            break resp;
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<
            std::result::Result<CreateChatCompletionStreamResponse, OpenAIError>,
        >(64);
        tokio::spawn(async move {
            let mut byte_stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            loop {
                let next = tokio::select! {
                    biased;
                    _ = tx.closed() => return,
                    next = byte_stream.next() => next,
                };
                match next {
                    Some(Ok(bytes)) => {
                        buf.extend_from_slice(&bytes);
                        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                            let line: Vec<u8> = buf.drain(..=pos).collect();
                            let line = String::from_utf8_lossy(&line);
                            let line = line.trim();
                            let Some(payload) = line.strip_prefix("data:") else {
                                continue;
                            };
                            let payload = payload.trim();
                            if payload == "[DONE]" {
                                return;
                            }
                            if let Some(cell) = sidecar.as_ref() {
                                note_billed_cost(cell, payload);
                            }
                            match parse_stream_chunk(payload) {
                                Ok(chunk) => {
                                    if tx.send(Ok(chunk)).await.is_err() {
                                        return;
                                    }
                                }
                                Err(e) => {
                                    let _ = tx
                                        .send(Err(OpenAIError::StreamError(format!(
                                            "bad SSE chunk: {e}"
                                        ))))
                                        .await;
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        let _ = tx
                            .send(Err(OpenAIError::StreamError(format!("transport: {e}"))))
                            .await;
                        return;
                    }
                    None => return,
                }
            }
        });
        Ok(tokio_stream::wrappers::ReceiverStream::new(rx))
    }
}

/// Remember the `usage.cost` a payload reports. Cheap check first: only the final chunk has it.
fn note_billed_cost(cell: &CostSidecar, payload: &str) {
    if !payload.contains("\"cost\"") {
        return;
    }
    let cost = serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.pointer("/usage/cost").and_then(|c| c.as_f64()));
    if let (Some(cost), Ok(mut slot)) = (cost, cell.lock()) {
        *slot = Some(cost);
    }
}

#[cfg(test)]
mod referer_tests {
    use super::with_referer;

    fn built(referer: Option<&str>) -> reqwest::Request {
        let client = reqwest::Client::new();
        with_referer(client.post("http://localhost/x"), referer)
            .build()
            .unwrap()
    }

    #[test]
    fn referer_header_is_omitted_unless_configured() {
        assert!(built(None).headers().get("HTTP-Referer").is_none());
    }

    #[test]
    fn configured_referer_is_sent() {
        let req = built(Some("https://hq.example.com"));
        assert_eq!(req.headers()["HTTP-Referer"], "https://hq.example.com");
    }
}
