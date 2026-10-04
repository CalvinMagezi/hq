use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::{ChatMessage, ToolDefinition};
use std::pin::Pin;
use std::time::Duration;
use tokio_stream::Stream;

// ─── Structured LLM error types ─────────────────────────────

/// Structured error type for LLM API calls.
/// Enables targeted retry logic instead of string matching.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// 429 — rate limited. May include a retry-after duration.
    #[error("rate limited{}", .retry_after.map(|d| format!(" (retry after {}s)", d.as_secs())).unwrap_or_default())]
    RateLimit { retry_after: Option<Duration> },

    /// 529 or "overloaded" — provider is temporarily overloaded.
    #[error("provider overloaded")]
    Overloaded,

    /// 401/403 — authentication or authorization failure.
    #[error("auth error ({status}): {message}")]
    Auth { status: u16, message: String },

    /// 5xx — server error.
    #[error("server error ({status}): {message}")]
    ServerError { status: u16, message: String },

    /// 400 with context overflow indication.
    #[error("context overflow: {message}")]
    ContextOverflow { message: String },

    /// Network / connection error (ECONNRESET, timeout, DNS, etc.)
    #[error("network error: {0}")]
    Network(String),

    /// Any other error.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl LlmError {
    /// Maps into the shared Agent-HQ middleware taxonomy for retries and adapters.
    #[inline]
    pub fn runtime_error_kind(&self) -> hq_core::middleware::RuntimeErrorKind {
        use hq_core::middleware::RuntimeErrorKind::*;
        match self {
            LlmError::RateLimit { .. }
            | LlmError::Overloaded
            | LlmError::ServerError { .. }
            | LlmError::Network(_) => Transient,
            LlmError::Auth { .. } => AuthN,
            LlmError::ContextOverflow { .. } => Capacity,
            LlmError::Other(_) => Permanent,
        }
    }

    /// Whether this error is transient and the request should be retried.
    pub fn is_transient(&self) -> bool {
        self.runtime_error_kind().is_retryable()
    }

    /// Whether a backend chain should try its next backend after this error,
    /// before any output. The one failover policy both chains share: outages,
    /// rate limits and auth move on; an overflow would overflow the next
    /// backend too; an unclassified error is each chain's own call.
    pub fn fails_over(&self) -> bool {
        use hq_core::middleware::RuntimeErrorKind::{AuthN, Transient};
        matches!(self.runtime_error_kind(), Transient | AuthN)
    }

    /// Whether this is a context overflow that should trigger compaction.
    pub fn is_context_overflow(&self) -> bool {
        matches!(self, LlmError::ContextOverflow { .. })
    }

    /// Construct from an HTTP status code and response body. The shared
    /// classifier; each wire's own classifier handles only its extra cases
    /// and defers here for the rest.
    pub fn from_http(status: u16, body: &str) -> Self {
        match status {
            429 => LlmError::RateLimit { retry_after: None },
            529 => LlmError::Overloaded,
            401 | 403 => LlmError::Auth {
                status,
                message: truncate_message(body).to_string(),
            },
            400 if body.contains("context") || mentions_context_overflow(body) => {
                LlmError::ContextOverflow {
                    message: truncate_message(body).to_string(),
                }
            }
            500..=599 => LlmError::ServerError {
                status,
                message: truncate_message(body).to_string(),
            },
            _ => LlmError::Other(anyhow::anyhow!(
                "HTTP {}: {}",
                status,
                truncate_message(body)
            )),
        }
    }

    /// Construct from a reqwest or network-level error.
    pub fn from_request_error(e: &reqwest::Error) -> Self {
        if e.is_timeout() {
            return LlmError::Network(format!("request timeout: {e}"));
        }
        if e.is_connect() {
            return LlmError::Network(format!("connection failed: {e}"));
        }
        LlmError::Network(e.to_string())
    }
}

/// Case-sensitive phrases every wire treats as a context overflow.
const CONTEXT_OVERFLOW_PATTERNS: &[&str] = &["too many tokens", "maximum context", "token limit"];

pub(crate) fn mentions_context_overflow(message: &str) -> bool {
    CONTEXT_OVERFLOW_PATTERNS
        .iter()
        .any(|p| message.contains(p))
}

/// The first 200 bytes of an error message, cut on a char boundary.
pub(crate) fn truncate_message(message: &str) -> &str {
    &message[..message.floor_char_boundary(200)]
}

/// A chunk from a streaming LLM response.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Text content delta
    Text(String),
    /// Reasoning/thinking delta from reasoning-capable models (OpenRouter `reasoning`,
    /// DeepSeek/Kimi `reasoning_content`). Surfaced separately from the final answer.
    Reasoning(String),
    /// Tool call being built (id, name, argument delta)
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    /// Stream finished
    Done,
    /// Usage information
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        /// Prompt tokens served from the provider's cache (billed cheaper).
        /// Zero for providers/streams that don't report a cache split.
        cache_read_tokens: u32,
        /// Prompt tokens written into the provider's cache this turn.
        /// Zero for providers/streams that don't report a cache split.
        cache_write_tokens: u32,
    },
    /// Actual model name from the provider response.
    ModelInfo(String),
}

/// Request to the LLM.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// Non-streaming response from the LLM.
#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub message: ChatMessage,
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Prompt tokens served from the KV cache (billed at a reduced rate by Anthropic).
    /// Zero for providers that don't support prompt caching.
    /// Distilled from claude-code's QueryEngine token accumulation.
    pub cache_read_tokens: u32,
    /// Prompt tokens written into the KV cache this turn.
    /// Zero for providers that don't support prompt caching.
    pub cache_write_tokens: u32,
    pub model: String,
}

/// Trait for LLM providers (OpenRouter, Anthropic, Google, Ollama).
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Provider name (e.g., "openrouter", "anthropic")
    fn name(&self) -> &str;

    /// Non-streaming chat completion.
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse>;

    /// Streaming chat completion.
    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>;
}

#[cfg(test)]
mod tests {
    use super::LlmError;

    #[test]
    fn from_http_classifies_by_status_and_overflow_phrases() {
        let cases: &[(u16, &str, &str)] = &[
            (429, "slow down", "RateLimit"),
            (529, "busy", "Overloaded"),
            (401, "bad key", "Auth"),
            (403, "forbidden", "Auth"),
            (400, "context too big", "ContextOverflow"),
            (400, "too many tokens", "ContextOverflow"),
            (400, "maximum context length is 8192", "ContextOverflow"),
            (400, "token limit reached", "ContextOverflow"),
            (400, "bad request", "Other"),
            (500, "boom", "ServerError"),
            (503, "unavailable", "ServerError"),
            (418, "teapot", "Other"),
        ];
        for &(status, body, want) in cases {
            let got = match LlmError::from_http(status, body) {
                LlmError::RateLimit { .. } => "RateLimit",
                LlmError::Overloaded => "Overloaded",
                LlmError::Auth { .. } => "Auth",
                LlmError::ContextOverflow { .. } => "ContextOverflow",
                LlmError::ServerError { .. } => "ServerError",
                LlmError::Network(_) => "Network",
                LlmError::Other(_) => "Other",
            };
            assert_eq!(got, want, "{status} {body:?}");
        }
    }

    #[test]
    fn from_http_truncates_on_a_char_boundary() {
        let body = "é".repeat(150);
        let LlmError::Auth { message, .. } = LlmError::from_http(401, &body) else {
            panic!("expected Auth");
        };
        assert_eq!(message.len(), 200);
    }
}
