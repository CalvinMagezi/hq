use anyhow::{Context, Result};
use async_openai::{Client, config::OpenAIConfig};
use async_trait::async_trait;
use futures::StreamExt;
use std::pin::Pin;
use tokio_stream::Stream;

use crate::openrouter::{build_request, classify_openai_error, parse_assistant_message};
use crate::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};

/// Local TurboQuant inference server provider.
/// Expects an OpenAI-compatible endpoint running locally, backed by TurboQuant
/// KV cache compression (3-bit, ~5x memory reduction).
///
/// Default base URL: http://localhost:14747/v1
/// Configurable via TURBOQUANT_BASE_URL env var.
pub struct TurboQuantProvider {
    client: Client<OpenAIConfig>,
}

impl TurboQuantProvider {
    pub fn new(base_url: &str) -> Self {
        let config = OpenAIConfig::new()
            .with_api_key("local")
            .with_api_base(base_url);
        let client = Client::with_config(config);
        Self {
            client,
        }
    }

    /// Create from TURBOQUANT_BASE_URL env var.
    /// Falls back to http://localhost:14747/v1 if not set.
    pub fn from_env() -> Result<Self> {
        let base = std::env::var("TURBOQUANT_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:14747/v1".to_string());
        Ok(Self::new(&base))
    }
}

#[async_trait]
impl LlmProvider for TurboQuantProvider {
    fn name(&self) -> &str {
        "turboquant"
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let oai_request = build_request(request)?;
        let response = self
            .client
            .chat()
            .create(oai_request)
            .await
            .map_err(|e| classify_openai_error(&e))?;

        let choice = response.choices.first().context("no choices in response")?;
        let message = parse_assistant_message(choice)?;

        let (input_tokens, output_tokens) = match response.usage {
            Some(usage) => (usage.prompt_tokens, usage.completion_tokens),
            None => (0, 0),
        };

        Ok(ChatResponse {
            message,
            input_tokens,
            output_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: response.model.clone(),
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

        let mapped = stream.map(|result| match result {
            Ok(response) => {
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
                    return Ok(StreamChunk::Done);
                }
                Ok(StreamChunk::Text(String::new()))
            }
            Err(e) => Err(anyhow::anyhow!("stream error: {}", e)),
        });

        Ok(Box::pin(mapped))
    }
}

/// Synchronous health check: TCP connect probe to the local server.
pub fn is_available_sync(base_url: &str) -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    let base = base_url.trim_end_matches("/v1");
    if let Some((host, port)) = parse_url_host_port(base) {
        let addr = format!("{}:{}", host, port);
        match std::net::ToSocketAddrs::to_socket_addrs(&addr) {
            Ok(mut addrs) => {
                if let Some(sock_addr) = addrs.next() {
                    TcpStream::connect_timeout(&sock_addr, Duration::from_millis(500)).is_ok()
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    } else {
        false
    }
}

fn parse_url_host_port(url: &str) -> Option<(String, u16)> {
    let stripped = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let parts: Vec<&str> = stripped.split('/').collect();
    let addr = parts.first()?;
    let mut seg = addr.split(':');
    let host = seg.next()?.to_string();
    let port = seg
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or_else(|| if url.starts_with("https") { 443 } else { 80 });
    Some((host, port))
}
