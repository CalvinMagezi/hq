use async_openai::{Client, config::OpenAIConfig};
use hq_core::types::MessageRole;
use std::time::Duration;

use crate::provider::ChatRequest;

mod stream;
mod transport;
mod wire;

pub(crate) use wire::parse_flexible_response;
pub use wire::{build_request, classify_openai_error, parse_assistant_message};


/// Google AI's OpenAI-compatible endpoint.
pub const GEMINI_OPENAI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai";

/// The only temperature Kimi Code's coding endpoint accepts.
const KIMI_CODING_TEMPERATURE: f32 = 1.0;

/// GitHub's entitlement check bucketed the *raw* `gh`-CLI-minted Copilot
/// token under a narrower `copilot-4-cli` integrator non-deterministically
/// per request — live-verified with 10 identical requests (same token, 5
/// with Copilot's editor headers, 5 without) against `gemini-3.8-flash`:
/// both groups mixed 200s and 400s (`model_not_available_for_integrator`)
/// back to back, seconds apart, with no header or content difference between
/// attempts. Root-caused and fixed by exchanging the raw token for a
/// short-lived session token before every request (see
/// `copilot_aware_credential`, wired to `crate::copilot`'s exchange); this
/// retry loop stays as a belt-and-suspenders fallback for whatever residual
/// flap the exchanged token doesn't eliminate, and for the case where the
/// exchange itself isn't available for a given account/token type and this
/// endpoint is back to sending the raw token. A generic 400 (an actual
/// malformed request) is never retried.
const COPILOT_INTEGRATOR_FLAP_MAX_ATTEMPTS: u32 = 3;
const COPILOT_INTEGRATOR_FLAP_RETRY_DELAY: Duration = Duration::from_millis(800);

/// Generic OpenAI-compatible provider. Works with any API that follows the
/// OpenAI chat completions format: DeepSeek, Novita, SiliconFlow, MiniMax,
/// Kimi/Moonshot, Zhipu/GLM, Fireworks, Google AI, OpenRouter, etc.
///
/// Uses `reqwest` for non-streaming requests (handles non-standard response
/// fields from models like Qwen, Kimi, DeepSeek) and `async-openai` for
/// streaming (more tolerant of extra fields).
pub struct OpenRouterProvider {
    client: Client<OpenAIConfig>,
    http: reqwest::Client,
    api_key: String,
    api_base: String,
    /// Reasoning effort forwarded as `thinking: {"effort": ...}` on endpoints
    /// that support it (Kimi K3: low|high|max). None = server default.
    thinking_effort: Option<String>,
    /// Speak the Responses API (`/responses`) instead of chat completions.
    responses_api: bool,
}

impl OpenRouterProvider {
    pub fn new(api_key: &str) -> Self {
        Self::new_with_base(api_key, "https://openrouter.ai/api/v1")
    }

    /// Create a provider with a custom API base URL.
    /// Works with OpenAI-compatible APIs, including gateways for Anthropic or
    /// Google models exposed over `/chat/completions`. For the native Anthropic
    /// Messages API, use [`AnthropicProvider`](crate::anthropic::AnthropicProvider)
    /// instead.
    pub fn new_with_base(api_key: &str, base_url: &str) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(base_url);

        let client = Client::with_config(config);
        let api_base = base_url.trim_end_matches('/').to_string();
        Self {
            client,
            http: crate::http::SHARED_HTTP_CLIENT.clone(),
            api_key: api_key.to_string(),
            api_base,
            thinking_effort: None,
            responses_api: false,
        }
    }

    /// Use the Responses API wire format (see [`crate::responses`]).
    pub fn with_responses_api(mut self, enabled: bool) -> Self {
        self.responses_api = enabled;
        self
    }

    /// Set the reasoning effort sent to thinking-capable endpoints.
    pub fn with_thinking_effort(mut self, effort: Option<String>) -> Self {
        self.thinking_effort = effort.filter(|e| !e.trim().is_empty());
        self
    }

    fn is_kimi_family(&self) -> bool {
        self.api_base.contains("moonshot") || self.api_base.contains("kimi")
    }

    /// GitHub Copilot's `/chat/completions` proxy (used for non-Claude models
    /// like Gemini, since Claude models route through the native Messages
    /// API instead): its streamed chunks omit the `object` field entirely,
    /// which async-openai's typed client treats as a hard deserialize error
    /// on every chunk. Routed through [`Self::raw_sse_stream`] instead, whose
    /// parser patches the field in (see its doc comment).
    fn is_copilot_endpoint(&self) -> bool {
        self.api_base.contains("githubcopilot.com")
    }

    /// Attach GitHub's editor-attribution headers when posting to a Copilot
    /// endpoint. Live-verified this is not cosmetic: GitHub scopes *model
    /// entitlement itself* to these headers — a request with only
    /// `Authorization`/`Content-Type` gets bucketed under a narrower default
    /// integrator (`copilot-4-cli`) whose model list excludes Gemini
    /// entirely, failing with `model_not_available_for_integrator` and (as of
    /// this fix) pointing straight at this header set as the remedy. A no-op
    /// for every other endpoint.
    fn attach_copilot_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if !self.is_copilot_endpoint() {
            return builder;
        }
        builder
            .header("Editor-Version", crate::copilot::EDITOR_VERSION)
            .header(
                "Copilot-Integration-Id",
                crate::copilot::COPILOT_INTEGRATION_ID,
            )
            .header("Openai-Intent", crate::copilot::OPENAI_INTENT)
            .header("x-initiator", "agent")
    }

    /// Resolve the `(api_base, bearer_token)` pair to actually send. For a
    /// Copilot endpoint, this is the exchanged short-lived session token and
    /// its account-scoped API base rather than `self.api_key`/`self.api_base`
    /// straight through — see `copilot.rs`'s module doc comment for why
    /// sending the raw token directly flaps with
    /// `model_not_available_for_integrator`. A no-op for every other
    /// endpoint. Never fails: [`crate::copilot::cached_session_token_for`]
    /// falls back to the raw token itself when the exchange doesn't work for
    /// this account/token type, so this endpoint keeps working exactly as it
    /// did before the exchange existed in that case.
    async fn copilot_aware_credential(&self) -> (String, String) {
        if !self.is_copilot_endpoint() {
            return (self.api_base.clone(), self.api_key.clone());
        }
        match crate::copilot::cached_session_token_for(&self.api_key, false).await {
            Ok(session) => (
                session.api_base.unwrap_or_else(|| self.api_base.clone()),
                session.jwt,
            ),
            Err(_) => (self.api_base.clone(), self.api_key.clone()),
        }
    }

    /// Endpoint-specific request-body fixups shared by the buffered and
    /// streaming paths: DeepSeek reasoning replay, Kimi thinking control, and
    /// the Kimi Code temperature clamp.
    fn mutate_body_for_endpoint(&self, body: &mut serde_json::Value, request: &ChatRequest) {
        // For DeepSeek tool-call turns: replay reasoning_content from history or DeepSeek
        // returns 400 on the next request. Non-tool turns omit it to save tokens.
        if let Some(msgs) = body["messages"].as_array_mut() {
            for (i, chat_msg) in request.messages.iter().enumerate() {
                if chat_msg.role == MessageRole::Assistant
                    && !chat_msg.tool_calls.is_empty()
                    && let Some(ref rc) = chat_msg.reasoning_content
                    && let Some(m) = msgs.get_mut(i)
                {
                    m["reasoning_content"] = serde_json::json!(rc);
                }
            }
        }

        if self.is_kimi_family()
            && let Some(obj) = body.as_object_mut()
        {
            match &self.thinking_effort {
                // Kimi K3 effort control (validated live: `thinking.effort` is
                // parsed and rejected on invalid values; the sibling forms
                // `reasoning_effort`/`think_effort` are silently ignored).
                Some(effort) => {
                    obj.insert(
                        "thinking".to_string(),
                        serde_json::json!({"effort": effort}),
                    );
                }
                None => {
                    // Coding-endpoint models are thinking-only: omit the field
                    // and let the server default (high) apply. Other Kimi/
                    // Moonshot endpoints keep the K2.5 disable workaround
                    // ("reasoning_content missing" errors).
                    if !self.api_base.contains("kimi.com/coding") {
                        obj.insert(
                            "thinking".to_string(),
                            serde_json::json!({"type": "disabled"}),
                        );
                    }
                }
            }
        }

        // Kimi Code's coding endpoint rejects any temperature but 1 ("invalid
        // temperature: only 1 is allowed for this model", confirmed live against
        // k3-256k, k3 and k2p7-code on 2026-07-26; the earlier 0.6 constraint
        // applied to K2.5 and no longer holds for any served model). Force it
        // here so every caller is correct regardless of what they pass.
        if self.api_base.contains("kimi.com/coding")
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert(
                "temperature".to_string(),
                serde_json::json!(KIMI_CODING_TEMPERATURE),
            );
        }
    }
}

#[cfg(test)]
mod integrator_flap_retry_tests {
    use super::OpenRouterProvider;
    use crate::provider::LlmProvider;
    use hq_core::types::{ChatMessage, MessageRole};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn user_request() -> crate::provider::ChatRequest {
        crate::provider::ChatRequest {
            model: "gemini-3.8-flash".to_string(),
            messages: vec![ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: "hi".to_string(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            }],
            tools: vec![],
            temperature: None,
            max_tokens: None,
        }
    }

    /// Serve `responses` in order, one per accepted connection, each as a
    /// full HTTP response then close.
    async fn serve_sequence(listener: TcpListener, responses: Vec<(u16, &'static str)>) {
        for (status, body) in responses {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut scratch = [0u8; 8192];
            let _ = sock.read(&mut scratch).await;
            let reason = if status == 200 { "OK" } else { "Bad Request" };
            let resp = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        }
    }

    const FLAP_BODY: &str = r#"{"error":{"message":"The requested model is not available for integrator \"copilot-4-cli\".","code":"model_not_available_for_integrator","type":"invalid_request_error"}}"#;
    const OK_BODY: &str = r#"{"id":"x","model":"gemini-3.8-flash","choices":[{"index":0,"message":{"role":"assistant","content":"pong"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;

    /// Two integrator-entitlement flaps, then success — the retry must
    /// recover and return the eventual successful answer, not surface the
    /// earlier flaps as the final error.
    #[tokio::test]
    async fn buffered_chat_retries_integrator_flap_then_succeeds() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(serve_sequence(
            listener,
            vec![(400, FLAP_BODY), (400, FLAP_BODY), (200, OK_BODY)],
        ));

        let provider = OpenRouterProvider::new_with_base("tok", &format!("http://{addr}"));
        let response = provider.chat(&user_request()).await.unwrap();
        assert_eq!(response.message.content, "pong");
        server.await.unwrap();
    }

    /// A genuine 400 (any code other than the known flap) must never be
    /// retried — retrying a real client-error would just burn three requests
    /// on a request that will never succeed.
    #[tokio::test]
    async fn buffered_chat_does_not_retry_an_unrelated_400() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let other_400 = r#"{"error":{"message":"invalid request","code":"invalid_request_error"}}"#;
        let server = tokio::spawn(serve_sequence(listener, vec![(400, other_400)]));

        let provider = OpenRouterProvider::new_with_base("tok", &format!("http://{addr}"));
        let err = provider.chat(&user_request()).await.unwrap_err();
        assert!(format!("{err:#}").contains("invalid request"));
        server.await.unwrap();
    }
}

#[cfg(test)]
mod tests;
