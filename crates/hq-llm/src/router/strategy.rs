use std::pin::Pin;
use std::time::Instant;

use anyhow::{Result, bail};
use async_trait::async_trait;
use futures::StreamExt;
use tokio_stream::Stream;
use tracing::warn;

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};

use super::LlmRouter;
use super::health::{ProviderHealth, classify_anyhow_error, classify_error_for_telemetry};
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
                    let usage = Some((resp.input_tokens, resp.output_tokens));
                    self.record_attempt_ok(
                        &candidate.provider_name,
                        &routed_req.model,
                        task,
                        call_start.elapsed(),
                        usage,
                    );
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
                        &routed_req.model,
                        task,
                        latency,
                        &e,
                    );
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

                let fallback_model = self
                    .routes
                    .iter()
                    .find(|r| {
                        r.provider == *name && !r.model_id.is_empty() && !r.pattern.ends_with('*')
                    })
                    .map(|r| r.model_id.clone())
                    .unwrap_or_else(|| request.model.clone());

                let mut fallback_req = request.clone();
                fallback_req.model = fallback_model;

                let call_start = Instant::now();
                match provider.chat(&fallback_req).await {
                    Ok(resp) => {
                        let latency = call_start.elapsed();
                        self.round_robin
                            .store(idx + 1, std::sync::atomic::Ordering::Relaxed);
                        let usage = Some((resp.input_tokens, resp.output_tokens));
                        self.record_attempt_ok(name, &fallback_req.model, task, latency, usage);
                        return Ok(resp);
                    }
                    Err(e) => {
                        let latency = call_start.elapsed();
                        self.record_attempt_err(name, &fallback_req.model, task, latency, &e);
                        continue;
                    }
                }
            }
        }

        bail!("All LLM providers failed for model '{}'", request.model)
    }

    async fn chat_stream(
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
                        let usage = Some((resp.input_tokens, resp.output_tokens));
                        self.record_attempt_ok(
                            &candidate.provider_name,
                            &routed_req.model,
                            task,
                            call_start.elapsed(),
                            usage,
                        );
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
                            &routed_req.model,
                            task,
                            latency,
                            &e2,
                        );
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
                        &routed_req.model,
                        task,
                        latency,
                        &timeout_err,
                    );
                    continue;
                }
                Ok(inner) => match inner {
                    Some(Ok(first_chunk)) => {
                        self.record_attempt_ok(
                            &candidate.provider_name,
                            &routed_req.model,
                            task,
                            call_start.elapsed(),
                            None,
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
                                &routed_req.model,
                                task,
                                latency,
                                &e,
                            );
                            continue;
                        }

                        tracing::debug!(
                            provider = candidate.provider_name,
                            error = %e,
                            "stream first chunk failed, trying non-streaming fallback"
                        );
                        match candidate.provider.chat(&routed_req).await {
                            Ok(resp) => {
                                let usage = Some((resp.input_tokens, resp.output_tokens));
                                self.record_attempt_ok(
                                    &candidate.provider_name,
                                    &routed_req.model,
                                    task,
                                    call_start.elapsed(),
                                    usage,
                                );
                                return Ok(response_to_stream(resp));
                            }
                            Err(e2) => {
                                let latency = call_start.elapsed();
                                self.record_attempt_err(
                                    &candidate.provider_name,
                                    &routed_req.model,
                                    task,
                                    latency,
                                    &e2,
                                );
                                continue;
                            }
                        }
                    }
                    None => {
                        let latency = call_start.elapsed();
                        let empty_err = anyhow::anyhow!("empty stream");
                        self.record_attempt_err(
                            &candidate.provider_name,
                            &routed_req.model,
                            task,
                            latency,
                            &empty_err,
                        );
                        continue;
                    }
                },
            }
        }
        bail!(
            "No provider found for model '{}' (streaming)",
            request.model
        )
    }
}

impl LlmRouter {
    /// Record a successful attempt in health and in the outcome sink.
    /// `usage` is `(input, output)` tokens when the provider reported them.
    fn record_attempt_ok(
        &self,
        provider_name: &str,
        model: &str,
        task: TaskHint,
        latency: std::time::Duration,
        usage: Option<(u32, u32)>,
    ) {
        let tokens = usage.map_or(0, |(i, o)| i as u64 + o as u64);
        self.record_success(provider_name, task, tokens, latency);
        self.record_outcome(
            provider_name,
            model,
            task,
            latency,
            usage.map(|(i, _)| i),
            usage.map(|(_, o)| o),
            true,
            None,
        );
    }

    /// Record a failed attempt in health and in the outcome sink.
    fn record_attempt_err(
        &self,
        provider_name: &str,
        model: &str,
        task: TaskHint,
        latency: std::time::Duration,
        error: &anyhow::Error,
    ) {
        self.record_failure(provider_name, task, error);
        self.record_outcome(
            provider_name,
            model,
            task,
            latency,
            None,
            None,
            false,
            Some(error),
        );
    }

    /// Internal helper: emit a task outcome to the configured sink, if any.
    // Flat telemetry fields mirror `OutcomeEvent`; a params struct is tracked in TECHDEBT.md.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_outcome(
        &self,
        provider_name: &str,
        model: &str,
        task: TaskHint,
        latency: std::time::Duration,
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        success: bool,
        error: Option<&anyhow::Error>,
    ) {
        let Some(sink) = self.outcome_sink.clone() else {
            return;
        };
        let ctx = crate::outcome_sink::current_context();
        let cost_usd = match (input_tokens, output_tokens) {
            (Some(i), Some(o)) => crate::models::calculate_cost(model, i, o),
            _ => 0.0,
        };
        let event = crate::outcome_sink::OutcomeEvent {
            session_id: ctx.session_id,
            turn_idx: ctx.turn_idx,
            model: model.to_string(),
            provider: provider_name.to_string(),
            task_hint: task.as_str(),
            latency_ms: latency.as_millis().min(i64::MAX as u128) as i64,
            input_tokens: input_tokens.map(|v| v as i64),
            output_tokens: output_tokens.map(|v| v as i64),
            cost_usd,
            success,
            error_class: error.map(classify_error_for_telemetry),
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                sink.record(event).await;
            });
        } else {
            tracing::debug!("record_outcome skipped: no Tokio runtime active");
        }
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
