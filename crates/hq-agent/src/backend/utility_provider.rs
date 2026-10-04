//! [`BackendUtilityProvider`] — adapt a [`SessionBackend`] into an [`LlmProvider`]
//! for utility/compaction calls.
//!
//! The session runs internal, non-turn LLM calls (context compaction, preemptive
//! summarization) through a plain [`LlmProvider`] handle. When a configured
//! `backends` chain exposes no API backend to borrow a provider from (a pure-CLI
//! chain), this adapter routes those calls through the backend's own buffered
//! path instead of silently falling back to an unrelated legacy provider.
//!
//! It always drives a **buffered, tool-free** turn: a summary needs one plain
//! completion, never token streaming or tool calls.

use std::pin::Pin;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use hq_core::types::{ChatMessage, MessageRole};
use hq_llm::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};
use tokio_stream::{Stream, StreamExt};

use super::{BackendEvent, BackendRequest, SessionBackend};

/// An [`LlmProvider`] that fulfils utility/compaction calls by running a buffered
/// turn through a [`SessionBackend`].
pub struct BackendUtilityProvider {
    backend: Arc<dyn SessionBackend>,
    name: String,
}

impl BackendUtilityProvider {
    /// Wrap a backend for utility/compaction use.
    pub fn new(backend: Arc<dyn SessionBackend>) -> Self {
        let name = format!("backend-utility:{}", backend.name());
        Self { backend, name }
    }
}

#[async_trait]
impl LlmProvider for BackendUtilityProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        // Buffered, tool-free turn: a summary is a single plain completion.
        let mut backend_request = BackendRequest::from_chat(request, false);
        backend_request.tools.clear();

        let mut stream = self
            .backend
            .start(&backend_request)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;
        let mut cache_read_tokens = 0u32;
        let mut cache_write_tokens = 0u32;
        let mut model = request.model.clone();

        while let Some(item) = stream.next().await {
            match item.map_err(|e| anyhow!(e.to_string()))? {
                BackendEvent::TextDelta(t) | BackendEvent::Message(t) => content.push_str(&t),
                BackendEvent::ReasoningDelta(r) => reasoning.push_str(&r),
                BackendEvent::Usage {
                    input_tokens: it,
                    output_tokens: ot,
                    cache_read_tokens: cr,
                    cache_write_tokens: cw,
                } => {
                    input_tokens = it;
                    output_tokens = ot;
                    cache_read_tokens = cr;
                    cache_write_tokens = cw;
                }
                BackendEvent::ModelInfo(m) => model = m,
                BackendEvent::Done => break,
                // Progress / tool-call deltas / selection markers are irrelevant
                // for a buffered utility completion.
                _ => {}
            }
        }

        Ok(ChatResponse {
            message: ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Assistant,
                content,
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
            },
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
        // Utility calls are inherently buffered; surface the one completion as a
        // single text chunk followed by usage/model/done.
        let response = self.chat(request).await?;
        let mut chunks: Vec<Result<StreamChunk>> = Vec::new();
        if !response.message.content.is_empty() {
            chunks.push(Ok(StreamChunk::Text(response.message.content.clone())));
        }
        chunks.push(Ok(StreamChunk::Usage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            cache_read_tokens: response.cache_read_tokens,
            cache_write_tokens: response.cache_write_tokens,
        }));
        if !response.model.is_empty() {
            chunks.push(Ok(StreamChunk::ModelInfo(response.model)));
        }
        chunks.push(Ok(StreamChunk::Done));
        Ok(Box::pin(tokio_stream::iter(chunks)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendCapabilities, BackendError, BackendEventStream};

    /// A buffered backend that returns one message — stands in for a CLI harness.
    struct BufferedBackend;

    #[async_trait]
    impl SessionBackend for BufferedBackend {
        fn name(&self) -> &str {
            "buffered"
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::buffered_cli()
        }
        async fn start(
            &self,
            request: &BackendRequest,
        ) -> Result<BackendEventStream, BackendError> {
            // A utility call must be buffered and tool-free.
            assert!(!request.stream);
            assert!(request.tools.is_empty());
            Ok(Box::pin(tokio_stream::iter(vec![
                Ok(BackendEvent::Progress("thinking".into())),
                Ok(BackendEvent::Message("compact summary".into())),
                Ok(BackendEvent::Usage {
                    input_tokens: 40,
                    output_tokens: 8,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                }),
                Ok(BackendEvent::ModelInfo("cli-model".into())),
                Ok(BackendEvent::Done),
            ])))
        }
    }

    #[tokio::test]
    async fn routes_compaction_through_the_backend() {
        let provider = BackendUtilityProvider::new(Arc::new(BufferedBackend));
        let request = ChatRequest {
            model: "alias".into(),
            ..Default::default()
        };
        let response = provider.chat(&request).await.unwrap();
        assert_eq!(response.message.content, "compact summary");
        assert_eq!(response.input_tokens, 40);
        assert_eq!(response.output_tokens, 8);
        assert_eq!(response.model, "cli-model");
    }
}
