use anyhow::{Context, Result};
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

use crate::openrouter::{build_request, parse_assistant_message};
use crate::prompted_tools;
use crate::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};

const OLLAMA_BASE: &str = "http://localhost:11434/v1";
const OLLAMA_NATIVE: &str = "http://localhost:11434";

/// Ollama provider (uses OpenAI-compatible API).
///
/// Injects `options.num_ctx` into every request so Ollama allocates the
/// correct KV-cache size.  Without this, Ollama defaults to 2048 tokens —
/// far too small for sessions that carry 30+ tool schemas (~8-12 K tokens).
pub struct OllamaProvider;

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OllamaProvider {
    pub fn new() -> Self {
        Self
    }

    /// Look up the registered context window for this Ollama model and use it
    /// as `num_ctx`.  Falls back to 32 768 (safe for 14B models on 24 GB RAM).
    fn num_ctx(model: &str) -> u32 {
        let registered = crate::models::context_window(&format!("ollama/{model}"));
        // context_window() returns 128_000 for unknown models — cap at 32 768
        // here so we don't accidentally pre-allocate a 128 K KV-cache when the
        // model id hasn't been added to the registry.
        if registered == 128_000 {
            32_768
        } else {
            registered
        }
    }

    /// Build the JSON body for a chat completion request, injecting the
    /// Ollama-specific `options.num_ctx` field that `async-openai` can't carry.
    fn build_body(req: &ChatRequest, stream: bool) -> Result<serde_json::Value> {
        let oai_req = build_request(req)?;
        let mut body = serde_json::to_value(&oai_req).context("serialize OAI request")?;
        body["stream"] = serde_json::json!(stream);
        body["options"] = serde_json::json!({
            "num_ctx": Self::num_ctx(&req.model),
            // Disable extended thinking for qwen3.x models.
            // Without this the model wraps its plan in <think>...</think>, then
            // generates text after </think> instead of JSON tool calls.
            // Ollama strips <think> but not </think>, leaking it into content.
            "think": false,
        });
        // Keep the model resident for 2 minutes of inactivity. The post-response
        // unload call already evicts the model immediately; this is a fallback
        // if that call fails (e.g., network error during the unload spawn).
        body["keep_alive"] = serde_json::json!("2m");
        Ok(body)
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        // Strip provider prefix ("ollama/") — the router does this via resolve_actual_model,
        // but direct OllamaProvider calls (fallback path) pass the full prefixed name.
        let stripped;
        let request = if let Some(bare) = request.model.strip_prefix("ollama/") {
            stripped = {
                let mut r = request.clone();
                r.model = bare.to_string();
                r
            };
            &stripped
        } else {
            request
        };

        // Models lacking native function-calling (Gemma family) go through
        // the prompted-tools shim: tools move from the `tools` field into
        // the system prompt as XML, and we parse `<tool_call>` tags out of
        // the free-text response back into native tool_calls.
        let use_shim = prompted_tools::needs_prompted_tools(&request.model);
        if use_shim {
            tracing::debug!(
                model = %request.model,
                tools_advertised = request.tools.len(),
                "prompted_tools shim activated"
            );
        }
        let shimmed;
        let effective_request = if use_shim {
            shimmed = {
                let mut r = request.clone();
                prompted_tools::inject_tool_prompt(&mut r);
                r
            };
            &shimmed
        } else {
            request
        };

        // Build JSON body with injected num_ctx and POST via reqwest so Ollama
        // actually allocates the right KV-cache.  async-openai's typed structs
        // have no place for the `options` field, so we must go raw here.
        let body = Self::build_body(effective_request, false)?;
        let url = format!("{OLLAMA_BASE}/chat/completions");
        let resp = crate::http::SHARED_HTTP_CLIENT
            .post(&url)
            .header("Authorization", "Bearer ollama")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .context("Ollama HTTP request")?;
        // Schedule VRAM release via the native API after the response body lands.
        // The /v1/chat/completions endpoint silently ignores keep_alive in the
        // request body; the native /api/generate endpoint is the only reliable
        // way to force-unload a model.
        let unload_model = effective_request.model.clone();
        tokio::spawn(async move {
            let _ = unload_ollama_model(&unload_model).await;
        });

        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body_str = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Ollama error {status}: {body_str}"));
        }

        let json: serde_json::Value = resp.json().await.context("parse Ollama response")?;

        // Parse using async-openai type so we can reuse parse_assistant_message.
        let response: async_openai::types::CreateChatCompletionResponse =
            serde_json::from_value(json.clone()).context("deserialize Ollama response")?;

        let choice = response.choices.first().context("no choices in response")?;
        let mut message = parse_assistant_message(choice)?;

        // Strip leaked </think> closing tags. Ollama strips <think> from content
        // but can leave </think> when thinking mode bleeds through. Any content
        // before </think> is internal reasoning and should be discarded.
        if message.content.contains("</think>")
            && message.tool_calls.is_empty()
            && let Some(after) = message.content.split("</think>").last()
        {
            message.content = after.trim().to_string();
        }

        if use_shim {
            let (cleaned, calls) = prompted_tools::promote_calls(&message.content);
            message.content = cleaned;
            if !calls.is_empty() {
                message.tool_calls = calls;
            }
        }

        let (input_tokens, output_tokens) = match response.usage {
            Some(usage) => (usage.prompt_tokens, usage.completion_tokens),
            None => (0, 0),
        };

        tracing::debug!(
            model = %request.model,
            num_ctx = Self::num_ctx(&request.model),
            input_tokens,
            output_tokens,
            "Ollama chat complete"
        );

        Ok(ChatResponse {
            message,
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: response.model,
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        // Buffer through chat() for all Ollama models: this lets us inject
        // num_ctx consistently.  Real SSE streaming can be added later once
        // the byte-level parser is in place; for local inference (30-60 s/turn)
        // the buffering latency is negligible.
        let resp = self.chat(request).await?;
        let text = resp.message.content.clone();
        let tool_calls = resp.message.tool_calls.clone();
        let model = resp.model.clone();
        let mut chunks: Vec<Result<StreamChunk>> = Vec::new();
        if !text.is_empty() {
            chunks.push(Ok(StreamChunk::Text(text)));
        }
        for (i, tc) in tool_calls.into_iter().enumerate() {
            let args_str = serde_json::to_string(&tc.arguments).unwrap_or_default();
            chunks.push(Ok(StreamChunk::ToolCallDelta {
                index: i,
                id: Some(tc.id),
                name: Some(tc.name),
                arguments_delta: args_str,
            }));
        }
        if !model.is_empty() {
            chunks.push(Ok(StreamChunk::ModelInfo(model)));
        }
        chunks.push(Ok(StreamChunk::Done));
        Ok(Box::pin(tokio_stream::iter(chunks)))
    }
}

/// Force-unload a model from Ollama VRAM using the native API.
///
/// The OpenAI-compatible `/v1/chat/completions` endpoint ignores `keep_alive`
/// in the request body. The native `/api/generate` endpoint is the only
/// reliable way to release a model from VRAM immediately after use.
pub async fn unload_ollama_model(model: &str) -> anyhow::Result<()> {
    let body = serde_json::json!({"model": model, "keep_alive": 0});
    crate::http::SHARED_HTTP_CLIENT
        .post(format!("{OLLAMA_NATIVE}/api/generate"))
        .json(&body)
        .send()
        .await?;
    Ok(())
}
