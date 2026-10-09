//! LLM bridge: memory LLM calls go through the configured `LlmProvider`.

use crate::json_recovery::parse_llm_json;
use anyhow::Result;
use hq_core::types::{ChatMessage, MessageRole};
use hq_llm::provider::{ChatRequest, LlmProvider};
use serde::de::DeserializeOwned;
use std::sync::Arc;

/// Bridge for memory LLM operations over one provider and model alias.
#[derive(Clone)]
pub struct MemoryLlm {
    provider: Arc<dyn LlmProvider>,
    model: String,
}

impl MemoryLlm {
    pub fn with_provider(provider: Arc<dyn LlmProvider>, model: String) -> Self {
        Self { provider, model }
    }

    /// Generate a typed JSON response.
    pub async fn json<T: DeserializeOwned>(
        &self,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<T> {
        self.provider_json(&self.provider, system_prompt, user_prompt)
            .await
    }

    async fn provider_json<T: DeserializeOwned>(
        &self,
        provider: &Arc<dyn LlmProvider>,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<T> {
        let augmented = format!(
            "{system_prompt}\n\nRespond ONLY with valid JSON. No markdown fences, no explanation."
        );
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::System,
                    content: augmented,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                },
                ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::User,
                    content: user_prompt.to_string(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                },
            ],
            tools: Vec::new(),
            temperature: Some(0.3),
            max_tokens: Some(2048),
        };
        let response =
            hq_llm::with_default_origin(hq_llm::origin::MEMORY, provider.chat(&request)).await?;
        parse_llm_json::<T>(response.message.content.trim())
            .map_err(|e| anyhow::anyhow!("provider JSON parse failed: {e}"))
    }

    /// Simple chat (non-JSON, returns raw text).
    pub async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::System,
                    content: system_prompt.to_string(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                },
                ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::User,
                    content: user_prompt.to_string(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                },
            ],
            tools: Vec::new(),
            temperature: Some(0.3),
            max_tokens: Some(2048),
        };
        let resp =
            hq_llm::with_default_origin(hq_llm::origin::MEMORY, self.provider.chat(&request))
                .await?;
        Ok(resp.message.content.trim().to_string())
    }
}
