use anyhow::{Context, Result};
use async_openai::{Client, config::OpenAIConfig};
use async_trait::async_trait;
use futures::StreamExt;
use std::pin::Pin;
use tokio_stream::Stream;

use crate::openai_compat::{build_request, classify_openai_error, parse_flexible_response};
use crate::provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};

/// Cerebras provider (OpenAI-compatible API at api.cerebras.ai).
/// Ultra-fast inference: 1000-2200 tok/s depending on model.
pub struct CerebrasProvider {
    /// Used only for streaming (`chat_stream`); non-streaming `chat` bypasses
    /// this in favor of raw reqwest — see `chat()` doc comment for why.
    client: Client<OpenAIConfig>,
    http: reqwest::Client,
    api_key: String,
    api_base: String,
}

impl CerebrasProvider {
    pub fn new(api_key: &str) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base("https://api.cerebras.ai/v1");

        let client = Client::with_config(config);
        Self {
            client,
            http: crate::http::SHARED_HTTP_CLIENT.clone(),
            api_key: api_key.to_string(),
            api_base: "https://api.cerebras.ai/v1".to_string(),
        }
    }

    /// Create from CEREBRAS_API_KEY env var.
    pub fn from_env() -> Result<Self> {
        let key = std::env::var("CEREBRAS_API_KEY").context("CEREBRAS_API_KEY not set")?;
        Ok(Self::new(&key))
    }
}

/// Cerebras error responses are flat (`{"message","type","param","code"}`),
/// not nested under an `"error"` key the way OpenAI's format is. async_openai's
/// `WrappedError` type requires that nesting, so handing this body to it fails
/// with `JSONDeserialize("missing field \`error\`")` — which `classify_openai_error`
/// then degrades to a generic `Other` error, losing the real status (rate limit,
/// auth, etc.). Classify directly from the flat shape instead.
fn classify_cerebras_error(status: u16, bytes: &[u8]) -> LlmError {
    let message = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).to_string());

    // 402 is the one status Cerebras needs classified differently from
    // `LlmError::from_http`'s generic handling (which would fall through to
    // `Other`): treat it as an auth failure with a payment-specific message,
    // matching `OpenRouterProvider`'s wording. Everything else defers to the
    // shared classifier instead of re-deriving its match arms here.
    if status == 402 {
        return LlmError::Auth {
            status: 402,
            message: "Model requires payment or insufficient credits".to_string(),
        };
    }

    LlmError::from_http(status, &message)
}

#[async_trait]
impl LlmProvider for CerebrasProvider {
    fn name(&self) -> &str {
        "cerebras"
    }

    /// Uses raw reqwest instead of async_openai's strict client for both the
    /// success and error response paths. async_openai's typed response structs
    /// (and its `WrappedError` error type) assume OpenAI's exact JSON shapes;
    /// Cerebras deviates on error bodies (flat, not nested under `"error"`),
    /// which made every non-2xx response — rate limits included — surface as
    /// an opaque "missing field `error`" deserialization failure instead of
    /// being classified correctly. `build_request` is still used for
    /// request-building (correct serialization); only response parsing bypasses
    /// async_openai. Mirrors `OpenRouterProvider::chat()`'s existing pattern.
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let oai_request = build_request(request)?;
        let body = serde_json::to_value(&oai_request).context("serialize request")?;

        let resp = self
            .http
            .post(format!("{}/chat/completions", self.api_base))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
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

        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| LlmError::Other(anyhow::anyhow!("failed to read response: {e}")))?;

        if !status.is_success() {
            return Err(classify_cerebras_error(status.as_u16(), &bytes).into());
        }

        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
            LlmError::Other(anyhow::anyhow!("failed to parse Cerebras response: {e}"))
        })?;

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

        Ok(ChatResponse {
            message,
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model,
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let mut oai_request = build_request(request)?;
        oai_request.stream = Some(true);

        let stream = self
            .client
            .chat()
            .create_stream(oai_request)
            .await
            .map_err(|e| classify_openai_error(&e))?;

        let last_model = std::cell::RefCell::new(String::new());
        let mapped = stream.map(move |result| match result {
            Ok(response) => {
                if !response.model.is_empty() {
                    *last_model.borrow_mut() = response.model.clone();
                }
                let choice = match response.choices.first() {
                    Some(c) => c,
                    None => return Ok(StreamChunk::Done),
                };
                let delta = &choice.delta;
                if let Some(ref tool_calls) = delta.tool_calls {
                    for tc in tool_calls {
                        if let Some(ref func) = tc.function {
                            return Ok(StreamChunk::ToolCallDelta {
                                index: tc.index as usize,
                                id: tc.id.clone(),
                                name: func.name.clone(),
                                arguments_delta: func.arguments.clone().unwrap_or_default(),
                            });
                        }
                    }
                }
                if let Some(ref content) = delta.content
                    && !content.is_empty()
                {
                    return Ok(StreamChunk::Text(content.clone()));
                }
                if choice.finish_reason.is_some() {
                    let model = last_model.borrow().clone();
                    if !model.is_empty() {
                        return Ok(StreamChunk::ModelInfo(model));
                    }
                    return Ok(StreamChunk::Done);
                }
                Ok(StreamChunk::Text(String::new()))
            }
            Err(e) => Err(anyhow::anyhow!("stream error: {}", e)),
        });

        Ok(Box::pin(mapped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture captured live via Step 1 of task-5-brief.md: Cerebras's rate-limit
    /// error body is flat, not nested under `"error"` like OpenAI's shape.
    /// Before the fix this body, handed to async_openai, produced
    /// `OpenAIError::JSONDeserialize(Error("missing field \`error\`", ...))`,
    /// which `classify_openai_error` turned into a generic `Other` error —
    /// losing the fact that this was a retryable rate limit.
    const CEREBRAS_RATE_LIMIT_BODY: &str = r#"{"message":"Requests per minute limit exceeded - too many requests sent.","type":"too_many_requests_error","param":"quota","code":"request_quota_exceeded"}"#;

    /// Fixture from the task brief's own reproduction: a reasoning model
    /// (gpt-oss-120b) cut off by max_tokens before emitting `content` at all.
    const CEREBRAS_TRUNCATED_REASONING_BODY: &str = r#"{"choices":[{"finish_reason":"length","message":{"reasoning":"The user","role":"assistant"}}]}"#;

    #[test]
    fn classify_cerebras_error_maps_flat_429_body_to_rate_limit() {
        let err = classify_cerebras_error(429, CEREBRAS_RATE_LIMIT_BODY.as_bytes());
        assert!(
            matches!(err, LlmError::RateLimit { .. }),
            "expected RateLimit, got {err:?}"
        );
    }

    #[test]
    fn classify_cerebras_error_preserves_message_for_unmapped_status() {
        let err = classify_cerebras_error(404, CEREBRAS_RATE_LIMIT_BODY.as_bytes());
        let LlmError::Other(e) = err else {
            panic!("expected Other, got a different variant");
        };
        assert!(e.to_string().contains("Requests per minute limit exceeded"));
    }

    /// Locks in a delta from the `from_http` delegation refactor (commit
    /// 11864833): the pre-refactor `classify_cerebras_error` had no explicit
    /// 529 arm, so it fell into the generic catch-all and returned `Other`.
    /// `from_http` (`provider.rs:76`) does have an explicit 529 arm, so this
    /// now correctly returns `Overloaded` instead. Not a regression — the new
    /// result is more accurate — but it was untested, so pin it down.
    #[test]
    fn classify_cerebras_error_maps_529_to_overloaded() {
        let err = classify_cerebras_error(529, b"{\"message\":\"overloaded\"}");
        assert!(
            matches!(err, LlmError::Overloaded),
            "expected Overloaded, got {err:?}"
        );
    }

    /// Locks in a delta from the `from_http` delegation refactor (commit
    /// 11864833): the pre-refactor `classify_cerebras_error` had no
    /// context-overflow arm for 400, so it fell into the generic catch-all
    /// and returned `Other`. `from_http` (`provider.rs:81-89`) matches 400
    /// bodies containing context-overflow keywords, so this now correctly
    /// returns `ContextOverflow`. Not a regression — the new result is more
    /// accurate — but it was untested, so pin it down.
    #[test]
    fn classify_cerebras_error_maps_400_context_overflow_to_context_overflow() {
        let body = br#"{"message":"This model's maximum context length is 8192 tokens."}"#;
        let err = classify_cerebras_error(400, body);
        assert!(
            matches!(err, LlmError::ContextOverflow { .. }),
            "expected ContextOverflow, got {err:?}"
        );
    }

    #[test]
    fn parse_flexible_response_handles_reasoning_truncated_before_content() {
        let json: serde_json::Value =
            serde_json::from_str(CEREBRAS_TRUNCATED_REASONING_BODY).unwrap();
        let message = parse_flexible_response(&json).expect("must not error on truncated body");
        // No `content` key at all: falls back to the `reasoning` field rather
        // than failing outright.
        assert_eq!(message.content, "The user");
    }
}
