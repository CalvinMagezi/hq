//! One real request to the configured default model, so a setup that only looks configured is
//! caught before the first chat turn fails. Shared by `hq doctor` and `hq env`.

use std::time::Duration;

use hq_core::config::HqConfig;
use hq_core::types::{ChatMessage, MessageRole};
use hq_llm::provider::ChatRequest;

/// Long enough for a cold provider chain that tries more than one backend.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_MAX_TOKENS: u32 = 1;
const PROBE_PROMPT: &str = "Reply with one word.";

/// Ask the default model for one token. `Err` carries the provider's own error.
pub async fn probe_default_model(config: &HqConfig) -> Result<(), String> {
    let provider = hq_agent::builder::provider_for(config).map_err(|e| format!("{e:#}"))?;
    let request = ChatRequest {
        model: config.default_model.clone(),
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: PROBE_PROMPT.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
            image_parts: Vec::new(),
        }],
        tools: Vec::new(),
        temperature: None,
        max_tokens: Some(PROBE_MAX_TOKENS),
    };
    match tokio::time::timeout(PROBE_TIMEOUT, provider.chat(&request)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => Err(format!(
            "no answer within {} seconds",
            PROBE_TIMEOUT.as_secs()
        )),
    }
}

/// The line `hq doctor` and `hq env` print for a probe result.
pub fn describe(model: &str, result: &Result<(), String>) -> (&'static str, String) {
    match result {
        Ok(()) => (
            "ok",
            format!("default model `{model}` answered a test request"),
        ),
        Err(e) => (
            "FAIL",
            format!("default model `{model}` did not answer a test request: {e}"),
        ),
    }
}
