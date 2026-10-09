use std::pin::Pin;
use std::time::Instant;

use anyhow::{Result, bail};
use async_trait::async_trait;
use futures::StreamExt;
use tokio_stream::Stream;
use tracing::warn;

use crate::budget::{is_budget_block, last_blocked, with_blocked_slot};
use crate::cost::Usage;
use crate::instrument::usage_of;
use crate::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};

use super::LlmRouter;
use super::health::{ProviderHealth, classify_anyhow_error};
use super::selection::resolve_scored;
use super::types::TaskHint;

/// Seed provider health entries from an external slice. Used for bench-seeding
/// before the provider has seen live traffic.
pub fn seed_health_entry(
    health: &mut Vec<(String, ProviderHealth)>,
    provider_name: &str,
    task_hint: super::types::TaskHint,
    results: &[(bool, u64)],
) {
    if results.is_empty() {
        return;
    }

    let idx = task_hint as usize;
    let entry = if let Some(pos) = health.iter().position(|(n, _)| n == provider_name) {
        &mut health[pos].1
    } else {
        health.push((provider_name.to_string(), ProviderHealth::default()));
        let last = health.len() - 1;
        &mut health[last].1
    };

    for &(passed, duration_ms) in results {
        entry.task_window[idx].push(passed);
        if entry.task_window[idx].len() > super::types::RELIABILITY_WINDOW {
            entry.task_window[idx].remove(0);
        }

        if passed && duration_ms > 0 {
            let ms = duration_ms as f64;
            if entry.avg_latency_ms == 0.0 {
                entry.avg_latency_ms = ms;
            } else {
                entry.avg_latency_ms = entry.avg_latency_ms * 0.7 + ms * 0.3;
            }
        }
    }
}

/// Convert a non-streaming `ChatResponse` into a synthetic stream.
pub(super) fn response_to_stream(
    resp: ChatResponse,
) -> Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>> {
    let mut chunks: Vec<Result<StreamChunk>> = Vec::new();

    if !resp.model.is_empty() {
        chunks.push(Ok(StreamChunk::ModelInfo(resp.model)));
    }

    if !resp.message.content.is_empty() {
        chunks.push(Ok(StreamChunk::Text(resp.message.content.clone())));
    }

    for (i, tc) in resp.message.tool_calls.iter().enumerate() {
        chunks.push(Ok(StreamChunk::ToolCallDelta {
            index: i,
            id: Some(tc.id.clone()),
            name: Some(tc.name.clone()),
            arguments_delta: String::new(),
        }));
        let args_str = match &tc.arguments {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        chunks.push(Ok(StreamChunk::ToolCallDelta {
            index: i,
            id: None,
            name: None,
            arguments_delta: args_str,
        }));
    }

    chunks.push(Ok(StreamChunk::Usage {
        input_tokens: resp.input_tokens,
        output_tokens: resp.output_tokens,
        cache_read_tokens: resp.cache_read_tokens,
        cache_write_tokens: resp.cache_write_tokens,
    }));
    if resp.provider_cost_usd.is_some() || resp.reasoning_tokens > 0 {
        chunks.push(Ok(StreamChunk::Billing {
            cost_usd: resp.provider_cost_usd,
            reasoning_tokens: resp.reasoning_tokens,
        }));
    }

    chunks.push(Ok(StreamChunk::Done));

    Box::pin(futures::stream::iter(chunks))
}

/// Quick sync TCP probe to check if Ollama is reachable.
pub(super) fn ollama_is_available_sync() -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    let addr = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "127.0.0.1:11434".into());
    let addr = addr
        .strip_prefix("http://")
        .or_else(|| addr.strip_prefix("https://"))
        .unwrap_or(&addr);
    addr.parse::<std::net::SocketAddr>()
        .map(|a| TcpStream::connect_timeout(&a, Duration::from_millis(500)).is_ok())
        .unwrap_or(false)
}

/// Resolve the actual model ID to send to a provider.
/// When model_id is empty (wildcard route), extracts the suffix from the requested model string.
fn resolve_actual_model(model_id: &str, requested_model: &str) -> String {
    if model_id.is_empty() {
        if let Some(slash) = requested_model.find('/') {
            requested_model[slash + 1..].to_string()
        } else {
            requested_model.to_string()
        }
    } else {
        model_id.to_string()
    }
}

#[async_trait]
impl LlmProvider for LlmRouter {
    fn name(&self) -> &str {
        "router"
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        with_blocked_slot(self.chat_routed(request)).await
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        with_blocked_slot(self.chat_stream_routed(request)).await
    }
}

impl LlmRouter {
    async fn chat_routed(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let task = TaskHint::from_request(request);
        let candidates = {
            let health = self.health.lock().unwrap();
            resolve_scored(&request.model, task, &self.providers, &self.routes, &health)
        };

        for candidate in &candidates {
            let actual_model = resolve_actual_model(&candidate.model_id, &request.model);
            let mut routed_req = request.clone();
            routed_req.model = actual_model;

            let call_start = Instant::now();
            match candidate.provider.chat(&routed_req).await {
                Ok(resp) => {
                    let usage = Some(usage_of(&resp));
                    self.record_attempt_ok(
                        &candidate.provider_name,
                        task,
                        call_start.elapsed(),
                        usage);
                    return Ok(resp);
                }
                Err(e) => {
                    let latency = call_start.elapsed();
                    warn!(
                        provider = candidate.provider_name,
                        model = routed_req.model,
                        error = %e,
                        score = candidate.score,
                        "route failed, trying next candidate"
                    );
                    self.record_attempt_err(
                        &candidate.provider_name,
                        task,
                        latency,
                        &e);
                    continue;
                }
            }
        }

        // If no routes matched, try round-robin through all providers
        if candidates.is_empty()
            && let Some(start) = Some(self.round_robin.load(std::sync::atomic::Ordering::Relaxed))
        {
            for i in 0..self.providers.len() {
                let idx = (start + i) % self.providers.len();
                let (name, provider) = &self.providers[idx];

                let mut fallback_req = request.clone();
                fallback_req.model = self.fallback_model(name, &request.model);

                let call_start = Instant::now();
                match provider.chat(&fallback_req).await {
                    Ok(resp) => {
                        let latency = call_start.elapsed();
                        self.round_robin
                            .store(idx + 1, std::sync::atomic::Ordering::Relaxed);
                        let usage = Some(usage_of(&resp));
                        self.record_attempt_ok(
                            name,
                            task,
                            latency,
                            usage);
                        return Ok(resp);
                    }
                    Err(e) => {
                        let latency = call_start.elapsed();
                        self.record_attempt_err(name, task, latency, &e);
                        continue;
                    }
                }
            }
        }

        if let Some(blocked) = last_blocked() {
            return Err(blocked.into());
        }
        bail!("All LLM providers failed for model '{}'", request.model)
    }

    async fn chat_stream_routed(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let task = TaskHint::from_request(request);
        let candidates = {
            let health = self.health.lock().unwrap();
            resolve_scored(&request.model, task, &self.providers, &self.routes, &health)
        };

        for candidate in &candidates {
            let actual_model = resolve_actual_model(&candidate.model_id, &request.model);
            let mut routed_req = request.clone();
            routed_req.model = actual_model;

            let call_start = Instant::now();

            let mut stream = match candidate.provider.chat_stream(&routed_req).await {
                Ok(s) => s,
                Err(e) => match candidate.provider.chat(&routed_req).await {
                    Ok(resp) => {
                        let usage = Some(usage_of(&resp));
                        self.record_attempt_ok(
                            &candidate.provider_name,
                            task,
                            call_start.elapsed(),
                            usage);
                        return Ok(response_to_stream(resp));
                    }
                    Err(e2) => {
                        let latency = call_start.elapsed();
                        warn!(
                            provider = candidate.provider_name,
                            stream_err = %e,
                            fallback_err = %e2,
                            "both stream and fallback failed"
                        );
                        self.record_attempt_err(
                            &candidate.provider_name,
                            task,
                            latency,
                            &e2);
                        continue;
                    }
                },
            };

            match tokio::time::timeout(std::time::Duration::from_secs(30), stream.next()).await {
                Err(_timeout) => {
                    let latency = call_start.elapsed();
                    let timeout_err = anyhow::anyhow!("stream timeout: no first chunk in 30s");
                    self.record_attempt_err(
                        &candidate.provider_name,
                        task,
                        latency,
                        &timeout_err);
                    continue;
                }
                Ok(inner) => match inner {
                    Some(Ok(first_chunk)) => {
                        self.record_success(
                            &candidate.provider_name,
                            task,
                            0,
                            call_start.elapsed(),
                        );
                        let prepended = futures::stream::once(async move { Ok(first_chunk) });
                        return Ok(Box::pin(prepended.chain(stream)));
                    }
                    Some(Err(e)) => {
                        let is_rate_limit = e.to_string().contains("429")
                            || e.to_string().contains("Too Many Requests")
                            || e.to_string().contains("rate");
                        if is_rate_limit {
                            let latency = call_start.elapsed();
                            tracing::debug!(
                                provider = candidate.provider_name,
                                "stream 429 — skipping to next provider"
                            );
                            self.record_attempt_err(
                                &candidate.provider_name,
                                task,
                                latency,
                                &e);
                            continue;
                        }

                        tracing::debug!(
                            provider = candidate.provider_name,
                            error = %e,
                            "stream first chunk failed, trying non-streaming fallback"
                        );
                        match candidate.provider.chat(&routed_req).await {
                            Ok(resp) => {
                                let usage = Some(usage_of(&resp));
                                self.record_attempt_ok(
                                    &candidate.provider_name,
                                    task,
                                    call_start.elapsed(),
                                    usage);
                                return Ok(response_to_stream(resp));
                            }
                            Err(e2) => {
                                let latency = call_start.elapsed();
                                self.record_attempt_err(
                                    &candidate.provider_name,
                                    task,
                                    latency,
                                    &e2);
                                continue;
                            }
                        }
                    }
                    None => {
                        let latency = call_start.elapsed();
                        let empty_err = anyhow::anyhow!("empty stream");
                        self.record_attempt_err(
                            &candidate.provider_name,
                            task,
                            latency,
                            &empty_err);
                        continue;
                    }
                },
            }
        }
        // No route matched: like `chat`, try every registered provider in turn with the model name
        // unchanged. Without this a fresh install's default model (anthropic/claude-sonnet-4) had no
        // provider when OpenRouter was the only one configured.
        if candidates.is_empty() {
            let start = self.round_robin.load(std::sync::atomic::Ordering::Relaxed);
            for i in 0..self.providers.len() {
                let idx = (start + i) % self.providers.len();
                let (name, provider) = &self.providers[idx];
                let mut fallback_req = request.clone();
                fallback_req.model = self.fallback_model(name, &request.model);

                let call_start = Instant::now();
                let opened = match provider.chat_stream(&fallback_req).await {
                    Ok(s) => Ok(s),
                    Err(_) => provider.chat(&fallback_req).await.map(response_to_stream),
                };
                // A stream can open and then fail on its first chunk (an unknown model is a 404
                // there), so a provider only counts once it has produced something.
                let outcome = match opened {
                    Ok(mut s) => {
                        match tokio::time::timeout(std::time::Duration::from_secs(30), s.next())
                            .await
                        {
                            Ok(Some(Ok(first))) => {
                                let prepended = futures::stream::once(async move { Ok(first) });
                                Ok(Box::pin(prepended.chain(s))
                                    as Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>)
                            }
                            Ok(Some(Err(e))) => Err(e),
                            Ok(None) => Err(anyhow::anyhow!("empty stream")),
                            Err(_) => Err(anyhow::anyhow!("stream timeout: no first chunk in 30s")),
                        }
                    }
                    Err(e) => Err(e),
                };
                match outcome {
                    Ok(s) => {
                        self.round_robin
                            .store(idx + 1, std::sync::atomic::Ordering::Relaxed);
                        self.record_success(name, task, 0, call_start.elapsed());
                        return Ok(s);
                    }
                    Err(e) => {
                        self.record_attempt_err(
                            name,
                            task,
                            call_start.elapsed(),
                            &e);
                    }
                }
            }
        }
        if let Some(blocked) = last_blocked() {
            return Err(blocked.into());
        }
        bail!(
            "No provider could serve model '{}' (streaming); every registered provider failed or none is configured",
            request.model
        )
    }
}

impl LlmRouter {
    /// The model name to send a provider that no route claimed. A concrete `vendor/model` id goes
    /// through unchanged. An alias such as `relay` becomes the provider's own explicit route target
    /// when it has one, otherwise the alias unchanged.
    fn fallback_model(&self, provider_name: &str, requested: &str) -> String {
        if requested.contains('/') {
            return requested.to_string();
        }
        self.routes
            .iter()
            .find(|r| {
                r.provider == provider_name && !r.model_id.is_empty() && !r.pattern.ends_with('*')
            })
            .map(|r| r.model_id.clone())
            .unwrap_or_else(|| requested.to_string())
    }
}

impl LlmRouter {
    /// Record a successful attempt in provider health. `usage` is `(input, output)` tokens when the
    /// provider reported them. The ledger row is written by the instrumented provider itself.
    fn record_attempt_ok(
        &self,
        provider_name: &str,
        task: TaskHint,
        latency: std::time::Duration,
        usage: Option<Usage>,
    ) {
        let tokens = usage.map_or(0, |u| u.input as u64 + u.output as u64);
        self.record_success(provider_name, task, tokens, latency);
    }

    /// Record a failed attempt in provider health. A budget refusal says nothing about the
    /// provider, and no call was made.
    fn record_attempt_err(
        &self,
        provider_name: &str,
        task: TaskHint,
        _latency: std::time::Duration,
        error: &anyhow::Error,
    ) {
        if is_budget_block(error) {
            return;
        }
        self.record_failure(provider_name, task, error);
    }

    pub(super) fn record_success(
        &self,
        provider_name: &str,
        task: TaskHint,
        tokens: u64,
        latency: std::time::Duration,
    ) {
        let mut health = self.health.lock().unwrap();
        if let Some((_, h)) = health.iter_mut().find(|(n, _)| n == provider_name) {
            h.record_success(task, tokens, latency);
        }
    }

    pub(super) fn record_failure(
        &self,
        provider_name: &str,
        task: TaskHint,
        error: &anyhow::Error,
    ) {
        let llm_error = classify_anyhow_error(error);
        let mut health = self.health.lock().unwrap();
        if let Some((_, h)) = health.iter_mut().find(|(n, _)| n == provider_name) {
            h.record_failure(task, &llm_error);
        }
    }
}
