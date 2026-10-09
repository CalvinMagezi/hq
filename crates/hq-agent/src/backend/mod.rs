//! Unified API/CLI session backend contract.
//!
//! A [`SessionBackend`] is the single abstraction the session engine talks to,
//! whether the work is served by an HTTP LLM API (OpenRouter, Kimi Code, a
//! generic OpenAI-compatible endpoint, or the native Anthropic Messages API) or
//! by a local CLI harness
//! subprocess (GitHub Copilot CLI). Every backend produces the same
//! [normalized event stream](BackendEvent), so higher layers never branch on
//! "is this streaming or buffered, API or CLI".
//!
//! # Pieces
//! - [`SessionBackend`] — the trait: `name`, `capabilities`, `start`.
//! - [`BackendCapabilities`] — what a backend *honestly* supports (streaming,
//!   tools, reasoning, …). A buffered CLI advertises limited capabilities
//!   rather than pretending to stream tokens.
//! - [`BackendRequest`] — a normalized turn request (mirrors [`ChatRequest`]
//!   plus a `stream` flag).
//! - [`BackendEvent`] — the normalized output event. Streaming API backends
//!   emit token deltas; a buffered backend emits lifecycle/progress plus a
//!   single final [`Message`](BackendEvent::Message) — never fake token deltas.
//! - [`BackendError`] — a typed, classifiable error with failover semantics.
//! - [`ProviderChain`] — an explicit primary + ordered fallback wrapper that is
//!   itself a [`SessionBackend`]. It owns the stream to enforce "fail over only
//!   before any output".
//! - [`BackendRegistry`] — builds backends and the chain from
//!   [`BackendsConfig`](hq_core::config::BackendsConfig).
//!
//! This module is the foundation for the session-engine migration; it does not
//! yet replace the existing session loop.

pub mod api;
pub mod chain;
pub mod cli;
pub mod registry;
pub mod utility_provider;

pub use api::ApiBackend;
pub use chain::ProviderChain;
pub use cli::CopilotCliBackend;
pub use registry::BackendRegistry;
pub use utility_provider::BackendUtilityProvider;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use hq_core::types::{ChatMessage, ToolDefinition};
use hq_llm::provider::{ChatRequest, LlmError, LlmProvider, StreamChunk};
use tokio_stream::Stream;

/// Capabilities a backend honestly advertises.
///
/// The provider chain uses these to skip backends that cannot satisfy a
/// request (e.g. a buffered CLI cannot drive our tool-calling loop, so it is
/// skipped for requests that carry tool definitions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    /// Emits incremental token deltas (vs. one buffered final message).
    pub streaming: bool,
    /// Can accept [`ToolDefinition`]s and drive our tool-calling loop by
    /// emitting [`BackendEvent::ToolCallDelta`].
    pub tools: bool,
    /// Surfaces reasoning/thinking deltas separately from the answer.
    pub reasoning: bool,
    /// Honors a distinct system role/prompt.
    pub system_prompt: bool,
    /// Reports token usage.
    pub usage_accounting: bool,
}

impl BackendCapabilities {
    /// A fully featured HTTP LLM API backend.
    pub const fn full_api() -> Self {
        Self {
            streaming: true,
            tools: true,
            reasoning: true,
            system_prompt: true,
            usage_accounting: true,
        }
    }

    /// A buffered CLI harness: no token streaming, cannot drive our tool loop,
    /// no separate reasoning channel, no usage accounting. It does absorb a
    /// composed prompt (including system text), so `system_prompt` is `true`.
    pub const fn buffered_cli() -> Self {
        Self {
            streaming: false,
            tools: false,
            reasoning: false,
            system_prompt: true,
            usage_accounting: false,
        }
    }

    /// Whether these capabilities can satisfy the request's minimum requirements.
    pub fn satisfies(&self, request: &BackendRequest) -> bool {
        let req = request.requirements();
        if req.tools && !self.tools {
            return false;
        }
        if req.streaming && !self.streaming {
            return false;
        }
        true
    }

    /// The capability union of two profiles (used to describe a chain).
    pub fn union(self, other: Self) -> Self {
        Self {
            streaming: self.streaming || other.streaming,
            tools: self.tools || other.tools,
            reasoning: self.reasoning || other.reasoning,
            system_prompt: self.system_prompt || other.system_prompt,
            usage_accounting: self.usage_accounting || other.usage_accounting,
        }
    }
}

/// The minimum capabilities a request needs from a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendRequirements {
    /// The request carries tool definitions and needs tool-calling support.
    pub tools: bool,
    /// The caller asked for incremental token streaming.
    pub streaming: bool,
}

/// A normalized request to a [`SessionBackend`].
///
/// Mirrors [`ChatRequest`] with an explicit `stream` flag so a backend can
/// choose its streaming vs. buffered path and the chain can capability-check.
#[derive(Debug, Clone, Default)]
pub struct BackendRequest {
    /// Requested model identifier.
    pub model: String,
    /// Conversation so far.
    pub messages: Vec<ChatMessage>,
    /// Tool definitions offered to the model.
    pub tools: Vec<ToolDefinition>,
    /// Sampling temperature.
    pub temperature: Option<f32>,
    /// Max output tokens.
    pub max_tokens: Option<u32>,
    /// Caller wants incremental token streaming (vs. a single buffered message).
    pub stream: bool,
}

impl BackendRequest {
    /// Build from a [`ChatRequest`], choosing streaming vs. buffered delivery.
    pub fn from_chat(request: &ChatRequest, stream: bool) -> Self {
        Self {
            model: request.model.clone(),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream,
        }
    }

    /// Convert back to a [`ChatRequest`] for an [`LlmProvider`](hq_llm::LlmProvider)-backed adapter.
    pub fn to_chat_request(&self) -> ChatRequest {
        ChatRequest {
            model: self.model.clone(),
            messages: self.messages.clone(),
            tools: self.tools.clone(),
            temperature: self.temperature,
            max_tokens: self.max_tokens,
        }
    }

    /// The minimum capabilities this request needs.
    pub fn requirements(&self) -> BackendRequirements {
        BackendRequirements {
            tools: !self.tools.is_empty(),
            streaming: self.stream,
        }
    }
}

/// A normalized output event from a backend.
///
/// Streaming API backends emit token-level deltas ([`TextDelta`](Self::TextDelta),
/// [`ReasoningDelta`](Self::ReasoningDelta), [`ToolCallDelta`](Self::ToolCallDelta)).
/// Buffered backends (a CLI harness) emit [`Progress`](Self::Progress) lifecycle
/// notes and exactly one [`Message`](Self::Message) with the full answer — they
/// never fabricate token deltas.
#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// Incremental answer text (streaming backends).
    TextDelta(String),
    /// Incremental reasoning/thinking text (streaming backends).
    ReasoningDelta(String),
    /// Incremental tool-call construction (streaming backends).
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    /// A human-readable lifecycle/progress note from a buffered/CLI backend.
    /// Not committed output — the chain may still fail over after a `Progress`.
    Progress(String),
    /// A complete assistant message. Buffered backends emit exactly one of
    /// these instead of `TextDelta` tokens.
    Message(String),
    /// Token usage report.
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        /// Prompt tokens served from the provider's cache (billed cheaper).
        /// Zero for streaming/CLI backends that don't report cache accounting.
        cache_read_tokens: u32,
        /// Prompt tokens written into the provider's cache this turn.
        cache_write_tokens: u32,
    },
    /// What the provider says it billed for the turn and how many reasoning tokens it spent.
    /// Sent after `Usage` by providers that report either.
    Billing {
        cost_usd: Option<f64>,
        reasoning_tokens: u32,
    },
    /// The concrete model identifier the backend actually used.
    ModelInfo(String),
    /// The identity of the backend that a [`ProviderChain`] committed to for this
    /// turn. Emitted once, before any output, so the session can tag backend-origin
    /// events with the *selected* backend (which may be a fallback, not the
    /// primary) rather than the chain's own name. A single backend never needs to
    /// emit this — the session defaults to the root backend's name.
    BackendSelected(String),
    /// A [`ProviderChain`] backend errored before producing any output and the
    /// chain is trying the next one. Carries the name of the backend that just
    /// failed, so a caller that only sees the eventual `BackendSelected` winner
    /// can still tell the turn didn't run cleanly on the declared primary.
    Failover(String),
    /// Terminal marker — the backend has finished this turn.
    Done,
}

impl BackendEvent {
    /// Whether this event carries committed assistant output.
    ///
    /// Once a backend emits an output event, [`ProviderChain`] will not fail
    /// over — any subsequent error propagates to the caller.
    pub fn is_output(&self) -> bool {
        match self {
            BackendEvent::TextDelta(text) | BackendEvent::Message(text) => !text.is_empty(),
            BackendEvent::ReasoningDelta(_) | BackendEvent::ToolCallDelta { .. } => true,
            BackendEvent::Progress(_)
            | BackendEvent::Usage { .. }
            | BackendEvent::Billing { .. }
            | BackendEvent::ModelInfo(_)
            | BackendEvent::BackendSelected(_)
            | BackendEvent::Failover(_)
            | BackendEvent::Done => false,
        }
    }

    /// Whether this is the terminal [`Done`](Self::Done) marker.
    pub fn is_done(&self) -> bool {
        matches!(self, BackendEvent::Done)
    }

    /// Translate an [`LlmProvider`](hq_llm::LlmProvider) [`StreamChunk`] into a normalized event.
    pub fn from_chunk(chunk: StreamChunk) -> Self {
        match chunk {
            StreamChunk::Text(t) => BackendEvent::TextDelta(t),
            StreamChunk::Reasoning(r) => BackendEvent::ReasoningDelta(r),
            StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => BackendEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            },
            StreamChunk::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => BackendEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            },
            StreamChunk::Billing {
                cost_usd,
                reasoning_tokens,
            } => BackendEvent::Billing {
                cost_usd,
                reasoning_tokens,
            },
            StreamChunk::ModelInfo(m) => BackendEvent::ModelInfo(m),
            StreamChunk::Done => BackendEvent::Done,
        }
    }
}

/// A typed, classifiable backend error.
///
/// The chain uses [`is_failoverable`](Self::is_failoverable) to decide whether
/// to try the next backend (only ever before any output is produced).
#[derive(Debug, Clone, thiserror::Error)]
pub enum BackendError {
    /// The backend is not usable right now (binary missing, not authenticated,
    /// disabled). Failoverable.
    #[error("backend unavailable: {0}")]
    Unavailable(String),

    /// The backend's capabilities cannot satisfy the request. Not failoverable
    /// on its own — the chain skips incompatible backends *before* calling them.
    #[error("backend incompatible: {0}")]
    Incompatible(String),

    /// A transient upstream failure (rate limit, overload, network, 5xx).
    /// Failoverable.
    #[error("backend transient error: {0}")]
    Transient(String),

    /// Authentication/authorization failure. Failoverable across backends
    /// (a different backend with different credentials may work).
    #[error("backend auth error: {0}")]
    Auth(String),

    /// The request exceeded the model's context window. Not failoverable
    /// (handled by compaction upstream, not by switching backends).
    #[error("backend context overflow: {0}")]
    ContextOverflow(String),

    /// The backend's event stream ended without a terminal
    /// [`BackendEvent::Done`] marker — a truncated turn. Synthesized by the
    /// unified session consumer when a stream is exhausted early so a cut-off
    /// response is never mistaken for a clean completion. Not failoverable: it
    /// is detected *after* the chain has already committed to a backend.
    #[error("backend truncated: {0}")]
    Truncated(String),

    /// A permanent or unclassified failure. Not failoverable.
    #[error("backend error: {0}")]
    Other(String),

    /// No backend in the chain could serve the request.
    #[error("no backend available: {0}")]
    NoBackendAvailable(String),
}

impl BackendError {
    /// Whether the provider chain should try the next backend after this error,
    /// assuming no output has been produced yet. Agrees with
    /// [`LlmError::fails_over`]; unlike hq-llm's background chain, an
    /// unclassified error stops here so the user sees it.
    pub fn is_failoverable(&self) -> bool {
        matches!(
            self,
            BackendError::Unavailable(_) | BackendError::Transient(_) | BackendError::Auth(_)
        )
    }

    /// Classify a structured [`LlmError`] into a backend error.
    pub fn from_llm(err: &LlmError) -> Self {
        match err {
            LlmError::RateLimit { .. }
            | LlmError::Overloaded
            | LlmError::ServerError { .. }
            | LlmError::Network(_) => BackendError::Transient(err.to_string()),
            LlmError::Auth { .. } => BackendError::Auth(err.to_string()),
            LlmError::ContextOverflow { .. } => BackendError::ContextOverflow(err.to_string()),
            LlmError::Other(_) => BackendError::Other(err.to_string()),
        }
    }

    /// Classify an [`anyhow::Error`] from an LLM provider, downcasting to
    /// [`LlmError`] when possible for precise classification.
    pub fn from_anyhow(err: anyhow::Error) -> Self {
        if let Some(llm) = err.downcast_ref::<LlmError>() {
            BackendError::from_llm(llm)
        } else {
            Self::from_text(err.to_string())
        }
    }

    /// Conservatively classify errors whose concrete transport type was erased
    /// by an adapter. This is intentionally limited to common status codes and
    /// stable provider/process messages so unknown failures remain permanent.
    pub fn from_text(message: String) -> Self {
        let lower = message.to_ascii_lowercase();

        if lower.contains("context length")
            || lower.contains("context window")
            || lower.contains("context overflow")
            || lower.contains("too many tokens")
            || lower.contains("maximum context")
            || lower.contains("token limit")
        {
            return BackendError::ContextOverflow(message);
        }
        if lower.contains("401")
            || lower.contains("403")
            || lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("authentication")
            || lower.contains("not authenticated")
            || lower.contains("invalid api key")
            || lower.contains("invalid token")
            || lower.contains("token expired")
            || lower.contains("login required")
            || lower.contains("not logged in")
            || lower.contains("insufficient credits")
            || lower.contains("payment required")
        {
            return BackendError::Auth(message);
        }
        if lower.contains("429")
            || lower.contains("rate limit")
            || lower.contains("too many requests")
            || lower.contains("quota exceeded")
        {
            return BackendError::Transient(message);
        }
        if lower.contains("failed to spawn")
            || lower.contains("no such file")
            || lower.contains("command not found")
        {
            return BackendError::Unavailable(message);
        }
        if lower.contains("timeout")
            || lower.contains("timed out")
            || lower.contains("connection")
            || lower.contains("network")
            || lower.contains("dns")
            || lower.contains("temporarily unavailable")
            || lower.contains("service unavailable")
            || lower.contains("bad gateway")
            || lower.contains("gateway timeout")
            || lower.contains("overloaded")
            || lower.contains("internal server error")
            || lower.contains("http 5")
            || lower.contains("status 5")
        {
            return BackendError::Transient(message);
        }

        BackendError::Other(message)
    }
}

/// A stream of normalized backend events.
pub type BackendEventStream =
    Pin<Box<dyn Stream<Item = Result<BackendEvent, BackendError>> + Send>>;

/// The unified API/CLI session backend contract.
///
/// Implementors translate a [`BackendRequest`] into a normalized
/// [`BackendEventStream`]. `start` returning `Err` means "could not begin"
/// (a startup failure eligible for failover); once it returns `Ok`, all further
/// problems arrive as `Err` items inside the stream.
#[async_trait]
pub trait SessionBackend: Send + Sync {
    /// Stable identifier for logs and diagnostics.
    fn name(&self) -> &str;

    /// The capabilities this backend honestly supports.
    fn capabilities(&self) -> BackendCapabilities;

    /// Begin a turn, returning a normalized event stream.
    async fn start(&self, request: &BackendRequest) -> Result<BackendEventStream, BackendError>;

    /// Capabilities of the *root* backend the session will actually drive.
    ///
    /// For a single backend this equals [`capabilities`](Self::capabilities). For
    /// a [`ProviderChain`] it is the **primary** backend's capabilities — *not*
    /// the union — because root backend selection is explicit, not adaptive. The
    /// session engine reads this once per run to decide whether to attach HQ tool
    /// schemas and whether to request token streaming: a CLI primary yields
    /// tool-free, buffered, terminal turns even when an API fallback exists.
    fn root_capabilities(&self) -> BackendCapabilities {
        self.capabilities()
    }

    /// Whether dropping this backend's [`BackendEventStream`] aborts in-flight
    /// work (e.g. a spawned CLI child killed via `kill_on_drop`, or a dropped
    /// HTTP body).
    ///
    /// The session engine relies on this for cooperative cancellation: when the
    /// root cancel flag trips mid-stream it stops polling and drops the stream,
    /// which must not orphan a subprocess. Default: `true` (both the API and CLI
    /// backends here are drop-safe). A backend that spawns detached work should
    /// override this to `false`.
    fn aborts_on_drop(&self) -> bool {
        true
    }

    /// A [`LlmProvider`] derived from this backend for utility/compaction calls
    /// (context summaries, preemptive summarization), when one exists.
    ///
    /// The session runs these internal, non-turn calls through a plain provider
    /// handle. An [`ApiBackend`] exposes its wrapped provider here; a buffered
    /// CLI backend has none (returns `None`). A [`ProviderChain`] delegates to
    /// the first constituent backend that offers one, so a standalone `backends`
    /// config needs no separate legacy provider. When this returns `None` the
    /// caller routes utility calls through the backend itself (see
    /// [`BackendUtilityProvider`](crate::backend::BackendUtilityProvider)).
    fn utility_provider(&self) -> Option<Arc<dyn LlmProvider>> {
        None
    }

    /// The model this backend is configured to serve, when it pins one.
    fn pinned_model(&self) -> Option<String> {
        None
    }

    /// Whether this backend already owns pre-output failover across alternatives
    /// (true only for [`ProviderChain`]).
    ///
    /// The session engine reads this to avoid double-retrying: a chain fails over
    /// by advancing to its next backend, so the session must not also wrap it in
    /// legacy retry/backoff. A single backend returns `false`, so the session
    /// applies `max_retries`/`retry_base_delay`/`fallback_model` around it.
    fn owns_failover(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::MessageRole;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: text.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        }
    }

    #[test]
    fn capabilities_satisfy_respects_tools_and_streaming() {
        let req_plain = BackendRequest {
            messages: vec![user_msg("hi")],
            ..Default::default()
        };
        let req_tools = BackendRequest {
            messages: vec![user_msg("hi")],
            tools: vec![ToolDefinition {
                name: "t".into(),
                description: "d".into(),
                parameters: serde_json::json!({}),
            }],
            ..Default::default()
        };
        let req_stream = BackendRequest {
            stream: true,
            ..req_plain.clone()
        };

        let cli = BackendCapabilities::buffered_cli();
        let api = BackendCapabilities::full_api();

        assert!(cli.satisfies(&req_plain));
        assert!(!cli.satisfies(&req_tools)); // no tool support
        assert!(!cli.satisfies(&req_stream)); // no streaming
        assert!(api.satisfies(&req_tools));
        assert!(api.satisfies(&req_stream));
    }

    #[test]
    fn event_output_classification() {
        assert!(BackendEvent::TextDelta("x".into()).is_output());
        assert!(BackendEvent::Message("x".into()).is_output());
        assert!(!BackendEvent::TextDelta(String::new()).is_output());
        assert!(!BackendEvent::Message(String::new()).is_output());
        assert!(
            BackendEvent::ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                arguments_delta: String::new()
            }
            .is_output()
        );
        assert!(!BackendEvent::Progress("running".into()).is_output());
        assert!(
            !BackendEvent::Usage {
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }
            .is_output()
        );
        assert!(!BackendEvent::Done.is_output());
        assert!(BackendEvent::Done.is_done());
    }

    #[test]
    fn error_failover_classification() {
        assert!(BackendError::Unavailable("x".into()).is_failoverable());
        assert!(BackendError::Transient("x".into()).is_failoverable());
        assert!(BackendError::Auth("x".into()).is_failoverable());
        assert!(!BackendError::ContextOverflow("x".into()).is_failoverable());
        assert!(!BackendError::Other("x".into()).is_failoverable());
        assert!(!BackendError::Incompatible("x".into()).is_failoverable());
    }

    #[test]
    fn root_capabilities_default_mirrors_capabilities() {
        // A backend that doesn't override root_capabilities exposes its own.
        struct Plain;
        #[async_trait]
        impl SessionBackend for Plain {
            fn name(&self) -> &str {
                "plain"
            }
            fn capabilities(&self) -> BackendCapabilities {
                BackendCapabilities::buffered_cli()
            }
            async fn start(
                &self,
                _request: &BackendRequest,
            ) -> Result<BackendEventStream, BackendError> {
                Ok(Box::pin(tokio_stream::iter(vec![Ok(BackendEvent::Done)])))
            }
        }
        let plain = Plain;
        assert_eq!(plain.root_capabilities(), plain.capabilities());
        assert!(plain.aborts_on_drop());
    }

    #[test]
    fn error_maps_from_llm_error() {
        assert!(matches!(
            BackendError::from_llm(&LlmError::Overloaded),
            BackendError::Transient(_)
        ));
        assert!(matches!(
            BackendError::from_llm(&LlmError::Auth {
                status: 401,
                message: "no".into()
            }),
            BackendError::Auth(_)
        ));
        assert!(matches!(
            BackendError::from_llm(&LlmError::ContextOverflow {
                message: "big".into()
            }),
            BackendError::ContextOverflow(_)
        ));
    }

    // The turn chain and hq-llm's background chain share one failover policy.
    #[test]
    fn turn_failover_matches_the_shared_llm_policy() {
        let errors = [
            LlmError::RateLimit { retry_after: None },
            LlmError::Overloaded,
            LlmError::Auth {
                status: 401,
                message: "no".into(),
            },
            LlmError::ServerError {
                status: 503,
                message: "down".into(),
            },
            LlmError::ContextOverflow {
                message: "big".into(),
            },
            LlmError::Network("reset".into()),
            LlmError::Other(anyhow::anyhow!("odd")),
        ];
        for err in &errors {
            assert_eq!(
                BackendError::from_llm(err).is_failoverable(),
                err.fails_over(),
                "{err}"
            );
        }
    }

    #[test]
    fn error_classifies_erased_transport_failures() {
        assert!(matches!(
            BackendError::from_anyhow(anyhow::anyhow!("stream error: HTTP 429 too many requests")),
            BackendError::Transient(_)
        ));
        assert!(matches!(
            BackendError::from_anyhow(anyhow::anyhow!("authentication failed: not logged in")),
            BackendError::Auth(_)
        ));
        assert!(matches!(
            BackendError::from_anyhow(anyhow::anyhow!("connection reset by peer")),
            BackendError::Transient(_)
        ));
    }

    #[test]
    fn chunk_translation_roundtrips_variants() {
        assert!(matches!(
            BackendEvent::from_chunk(StreamChunk::Text("a".into())),
            BackendEvent::TextDelta(s) if s == "a"
        ));
        assert!(matches!(
            BackendEvent::from_chunk(StreamChunk::Done),
            BackendEvent::Done
        ));
        assert!(matches!(
            BackendEvent::from_chunk(StreamChunk::ModelInfo("m".into())),
            BackendEvent::ModelInfo(s) if s == "m"
        ));
    }
}
