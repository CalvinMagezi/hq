//! API-backed [`SessionBackend`] adapter.
//!
//! Wraps any existing [`Arc<dyn LlmProvider>`](hq_llm::LlmProvider) and
//! translates its [`StreamChunk`](hq_llm::StreamChunk) output (or buffered
//! [`ChatResponse`](hq_llm::provider::ChatResponse)) into the normalized
//! [`BackendEvent`] stream. This is a thin translation layer — it reuses the
//! provider's HTTP client verbatim and never re-implements transport.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use hq_llm::provider::LlmProvider;

use super::{
    BackendCapabilities, BackendError, BackendEvent, BackendEventStream, BackendRequest,
    SessionBackend,
};

/// A [`SessionBackend`] backed by an HTTP LLM API provider.
pub struct ApiBackend {
    label: String,
    provider: Arc<dyn LlmProvider>,
    capabilities: BackendCapabilities,
    model_override: Option<String>,
}

impl ApiBackend {
    /// Wrap an existing provider as a fully featured (streaming, tools,
    /// reasoning, usage) API backend.
    pub fn new(label: impl Into<String>, provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            label: label.into(),
            provider,
            capabilities: BackendCapabilities::full_api(),
            model_override: None,
        }
    }

    /// Set the per-backend default model. A non-blank override replaces the
    /// caller's model immediately before dispatch, allowing chain fallbacks to
    /// use their own model identifiers.
    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.model_override = model.and_then(|model| {
            let model = model.trim();
            (!model.is_empty()).then(|| model.to_string())
        });
        self
    }

    #[cfg(test)]
    pub(crate) fn model_override(&self) -> Option<&str> {
        self.model_override.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn provider(&self) -> &Arc<dyn LlmProvider> {
        &self.provider
    }
}

#[async_trait]
impl SessionBackend for ApiBackend {
    fn name(&self) -> &str {
        &self.label
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
    }

    fn pinned_model(&self) -> Option<String> {
        self.model_override.clone()
    }

    fn utility_provider(&self) -> Option<std::sync::Arc<dyn LlmProvider>> {
        // An API backend's own provider is exactly what utility/compaction calls
        // need — expose it so a standalone `backends` config needs no separate
        // legacy provider. When this backend pins a model (chain fallbacks do),
        // wrap the provider so compaction/summarization runs against the
        // *configured* backend model rather than the caller's alias.
        match &self.model_override {
            Some(model) => Some(Arc::new(ModelOverrideProvider {
                inner: self.provider.clone(),
                model: model.clone(),
            })),
            None => Some(self.provider.clone()),
        }
    }

    async fn start(&self, request: &BackendRequest) -> Result<BackendEventStream, BackendError> {
        let mut chat = request.to_chat_request();
        if let Some(model) = &self.model_override {
            chat.model = model.clone();
        }

        if request.stream {
            let stream = self
                .provider
                .chat_stream(&chat)
                .await
                .map_err(BackendError::from_anyhow)?;
            let mapped = stream.map(|item| match item {
                Ok(chunk) => Ok(BackendEvent::from_chunk(chunk)),
                Err(e) => Err(BackendError::from_anyhow(e)),
            });
            Ok(Box::pin(mapped))
        } else {
            // Buffered path: one round-trip, then a normalized event burst.
            let response = self
                .provider
                .chat(&chat)
                .await
                .map_err(BackendError::from_anyhow)?;

            let mut events: Vec<Result<BackendEvent, BackendError>> = Vec::new();

            if let Some(reasoning) = response.message.reasoning_content.filter(|r| !r.is_empty()) {
                events.push(Ok(BackendEvent::ReasoningDelta(reasoning)));
            }

            // Surface any tool calls the model asked for as fully-formed deltas,
            // so a downstream collector reconstructs them uniformly with the
            // streaming path.
            for (index, call) in response.message.tool_calls.into_iter().enumerate() {
                events.push(Ok(BackendEvent::ToolCallDelta {
                    index,
                    id: Some(call.id),
                    name: Some(call.name),
                    arguments_delta: call.arguments.to_string(),
                }));
            }

            events.push(Ok(BackendEvent::Message(response.message.content)));
            events.push(Ok(BackendEvent::Usage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
                cache_read_tokens: response.cache_read_tokens,
                cache_write_tokens: response.cache_write_tokens,
            }));
            events.push(Ok(BackendEvent::ModelInfo(response.model)));
            events.push(Ok(BackendEvent::Done));

            Ok(Box::pin(futures::stream::iter(events)))
        }
    }
}

/// An [`LlmProvider`] that rewrites the request model to a fixed override before
/// delegating to an inner provider.
///
/// Utility/compaction calls borrow a backend's provider directly (bypassing the
/// backend's own [`start`](ApiBackend::start) model substitution). Without this
/// wrapper those calls would run against the caller's alias rather than the
/// backend's configured model — e.g. a chain fallback's compaction would silently
/// use the primary's model. Wrapping preserves the [`with_model`](ApiBackend::with_model)
/// override across the utility path.
struct ModelOverrideProvider {
    inner: Arc<dyn LlmProvider>,
    model: String,
}

impl ModelOverrideProvider {
    fn apply(&self, request: &hq_llm::provider::ChatRequest) -> hq_llm::provider::ChatRequest {
        let mut request = request.clone();
        request.model = self.model.clone();
        request
    }
}

#[async_trait]
impl LlmProvider for ModelOverrideProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn chat(
        &self,
        request: &hq_llm::provider::ChatRequest,
    ) -> anyhow::Result<hq_llm::provider::ChatResponse> {
        self.inner.chat(&self.apply(request)).await
    }

    async fn chat_stream(
        &self,
        request: &hq_llm::provider::ChatRequest,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<
                dyn tokio_stream::Stream<Item = anyhow::Result<hq_llm::provider::StreamChunk>>
                    + Send,
            >,
        >,
    > {
        self.inner.chat_stream(&self.apply(request)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use hq_core::types::{ChatMessage, MessageRole, ToolCall};
    use hq_llm::provider::{ChatRequest, ChatResponse, LlmError, StreamChunk};
    use std::pin::Pin;
    use std::sync::Mutex;
    use tokio_stream::Stream;

    struct ScriptedProvider {
        buffered: Mutex<Option<Result<ChatResponse>>>,
        stream: Mutex<Option<Vec<Result<StreamChunk>>>>,
        stream_start_err: Mutex<Option<LlmError>>,
    }

    struct RecordingProvider {
        seen_model: Mutex<Option<String>>,
    }

    impl ScriptedProvider {
        fn buffered(response: Result<ChatResponse>) -> Self {
            Self {
                buffered: Mutex::new(Some(response)),
                stream: Mutex::new(None),
                stream_start_err: Mutex::new(None),
            }
        }
        fn streaming(chunks: Vec<StreamChunk>) -> Self {
            Self {
                buffered: Mutex::new(None),
                stream: Mutex::new(Some(chunks.into_iter().map(Ok).collect())),
                stream_start_err: Mutex::new(None),
            }
        }
        fn streaming_results(chunks: Vec<Result<StreamChunk>>) -> Self {
            Self {
                buffered: Mutex::new(None),
                stream: Mutex::new(Some(chunks)),
                stream_start_err: Mutex::new(None),
            }
        }
        fn stream_startup_error(err: LlmError) -> Self {
            Self {
                buffered: Mutex::new(None),
                stream: Mutex::new(None),
                stream_start_err: Mutex::new(Some(err)),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn chat(&self, _request: &ChatRequest) -> Result<ChatResponse> {
            self.buffered
                .lock()
                .unwrap()
                .take()
                .expect("scripted buffered response")
        }
        async fn chat_stream(
            &self,
            _request: &ChatRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
            if let Some(err) = self.stream_start_err.lock().unwrap().take() {
                return Err(err.into());
            }
            let chunks = self.stream.lock().unwrap().take().expect("scripted stream");
            Ok(Box::pin(tokio_stream::iter(chunks)))
        }
    }

    #[async_trait]
    impl LlmProvider for RecordingProvider {
        fn name(&self) -> &str {
            "recording"
        }

        async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
            *self.seen_model.lock().unwrap() = Some(request.model.clone());
            Ok(response("ok", vec![]))
        }

        async fn chat_stream(
            &self,
            request: &ChatRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
            *self.seen_model.lock().unwrap() = Some(request.model.clone());
            Ok(Box::pin(tokio_stream::iter(vec![Ok(StreamChunk::Done)])))
        }
    }

    fn response(content: &str, tool_calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            message: ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Assistant,
                content: content.to_string(),
                tool_calls,
                tool_call_id: None,
                reasoning_content: None,
            },
            input_tokens: 5,
            output_tokens: 3,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: "scripted-model".to_string(),
        }
    }

    async fn collect(stream: BackendEventStream) -> Vec<Result<BackendEvent, BackendError>> {
        stream.collect::<Vec<_>>().await
    }

    #[tokio::test]
    async fn streaming_translates_chunks_one_for_one() {
        let provider = Arc::new(ScriptedProvider::streaming(vec![
            StreamChunk::Text("hel".into()),
            StreamChunk::Text("lo".into()),
            StreamChunk::Usage {
                input_tokens: 5,
                output_tokens: 2,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            },
            StreamChunk::ModelInfo("scripted-model".into()),
            StreamChunk::Done,
        ]));
        let backend = ApiBackend::new("api", provider);
        let req = BackendRequest {
            stream: true,
            ..Default::default()
        };
        let events = collect(backend.start(&req).await.unwrap()).await;
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                Ok(BackendEvent::TextDelta(t)) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "hello");
        assert!(events.iter().any(|e| matches!(e, Ok(BackendEvent::Done))));
    }

    #[tokio::test]
    async fn buffered_emits_message_then_usage_model_done() {
        let provider = Arc::new(ScriptedProvider::buffered(Ok(response(
            "final answer",
            vec![],
        ))));
        let backend = ApiBackend::new("api", provider);
        let req = BackendRequest {
            stream: false,
            ..Default::default()
        };
        let events = collect(backend.start(&req).await.unwrap()).await;

        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                Ok(BackendEvent::Message(_)) => "message",
                Ok(BackendEvent::Usage { .. }) => "usage",
                Ok(BackendEvent::ModelInfo(_)) => "model",
                Ok(BackendEvent::Done) => "done",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["message", "usage", "model", "done"]);
        // Buffered path produces exactly one output event.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Ok(ev) if ev.is_output()))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn buffered_tool_calls_become_tool_call_deltas() {
        let provider = Arc::new(ScriptedProvider::buffered(Ok(response(
            "",
            vec![ToolCall {
                id: "call-1".into(),
                name: "search".into(),
                arguments: serde_json::json!({"q": "rust"}),
            }],
        ))));
        let backend = ApiBackend::new("api", provider);
        let req = BackendRequest {
            stream: false,
            ..Default::default()
        };
        let events = collect(backend.start(&req).await.unwrap()).await;
        let tool = events.iter().find_map(|e| match e {
            Ok(BackendEvent::ToolCallDelta {
                id,
                name,
                arguments_delta,
                ..
            }) => Some((id.clone(), name.clone(), arguments_delta.clone())),
            _ => None,
        });
        let (id, name, args) = tool.expect("tool call delta");
        assert_eq!(id.as_deref(), Some("call-1"));
        assert_eq!(name.as_deref(), Some("search"));
        assert!(args.contains("rust"));
    }

    #[tokio::test]
    async fn streaming_start_error_maps_to_backend_error() {
        let provider = Arc::new(ScriptedProvider::stream_startup_error(LlmError::Auth {
            status: 401,
            message: "nope".into(),
        }));
        let backend = ApiBackend::new("api", provider);
        let req = BackendRequest {
            stream: true,
            ..Default::default()
        };
        let err = match backend.start(&req).await {
            Ok(_) => panic!("expected a startup error"),
            Err(e) => e,
        };
        assert!(matches!(err, BackendError::Auth(_)));
        assert!(err.is_failoverable());
    }

    #[tokio::test]
    async fn streaming_transport_error_is_classified_for_failover() {
        let provider = Arc::new(ScriptedProvider::streaming_results(vec![Err(
            anyhow::anyhow!("stream transport error: HTTP 429 too many requests"),
        )]));
        let backend = ApiBackend::new("api", provider);
        let events = collect(
            backend
                .start(&BackendRequest {
                    stream: true,
                    ..Default::default()
                })
                .await
                .unwrap(),
        )
        .await;

        assert!(matches!(
            events.as_slice(),
            [Err(BackendError::Transient(message))] if message.contains("429")
        ));
    }

    #[tokio::test]
    async fn configured_model_overrides_the_callers_model_before_dispatch() {
        let provider = Arc::new(RecordingProvider {
            seen_model: Mutex::new(None),
        });
        let backend =
            ApiBackend::new("api", provider.clone()).with_model(Some("fallback-model".to_string()));
        let req = BackendRequest {
            model: "caller-model".to_string(),
            stream: false,
            ..Default::default()
        };

        let _events = collect(backend.start(&req).await.unwrap()).await;

        assert_eq!(
            provider.seen_model.lock().unwrap().as_deref(),
            Some("fallback-model")
        );
    }

    #[test]
    fn utility_provider_exposes_the_wrapped_provider() {
        // An API backend must surface its own provider for utility/compaction
        // calls so a standalone `backends` config needs no legacy provider.
        let provider = Arc::new(RecordingProvider {
            seen_model: Mutex::new(None),
        });
        let backend = ApiBackend::new("api", provider);
        let util = backend
            .utility_provider()
            .expect("api backend exposes a utility provider");
        assert_eq!(util.name(), "recording");
    }

    #[tokio::test]
    async fn utility_provider_preserves_the_configured_model_override() {
        // A backend that pins a model (chain fallbacks do) must route utility /
        // compaction calls through that model, not the caller's alias.
        let provider = Arc::new(RecordingProvider {
            seen_model: Mutex::new(None),
        });
        let backend =
            ApiBackend::new("api", provider.clone()).with_model(Some("backend-model".to_string()));
        let util = backend
            .utility_provider()
            .expect("api backend exposes a utility provider");

        let request = ChatRequest {
            model: "caller-alias".to_string(),
            ..Default::default()
        };
        let _ = util.chat(&request).await.unwrap();

        assert_eq!(
            provider.seen_model.lock().unwrap().as_deref(),
            Some("backend-model"),
            "compaction must use the configured backend model"
        );
    }

    #[tokio::test]
    async fn utility_provider_without_override_uses_the_callers_model() {
        // No override: the borrowed provider sees the caller's model unchanged.
        let provider = Arc::new(RecordingProvider {
            seen_model: Mutex::new(None),
        });
        let backend = ApiBackend::new("api", provider.clone());
        let util = backend.utility_provider().unwrap();
        let request = ChatRequest {
            model: "caller-alias".to_string(),
            ..Default::default()
        };
        let _ = util.chat(&request).await.unwrap();
        assert_eq!(
            provider.seen_model.lock().unwrap().as_deref(),
            Some("caller-alias")
        );
    }
}
