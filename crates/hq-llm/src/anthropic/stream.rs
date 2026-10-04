//! Streaming SSE parsing for the Messages API.

use anyhow::Result;
use std::collections::HashMap;

use super::wire::parse_usage;
use crate::provider::{LlmError, StreamChunk};

/// Mutable state threaded across streamed SSE frames.
///
/// Anthropic content-block indices interleave text, thinking, and tool_use
/// blocks, but the downstream collector treats a [`StreamChunk::ToolCallDelta`]
/// `index` as a dense tool-call position. Remap each `tool_use` block to a
/// sequential 0-based tool index so argument deltas land on the right call.
#[derive(Debug, Default)]
pub(crate) struct AnthropicStreamState {
    pub(super) model_emitted: bool,
    pub(super) input_tokens: u32,
    pub(super) output_tokens: u32,
    /// Prompt tokens served from the KV cache, captured from `message_start`.
    pub(super) cache_read_tokens: u32,
    /// Prompt tokens written into the KV cache, captured from `message_start`.
    pub(super) cache_write_tokens: u32,
    pub(super) tool_index_by_block: HashMap<u64, usize>,
    pub(super) next_tool_index: usize,
    pub(super) done_emitted: bool,
}

impl AnthropicStreamState {
    fn assign_tool_index(&mut self, block_index: u64) -> usize {
        let index = self.next_tool_index;
        self.tool_index_by_block.insert(block_index, index);
        self.next_tool_index += 1;
        index
    }

    fn tool_index_for(&self, block_index: u64) -> Option<usize> {
        self.tool_index_by_block.get(&block_index).copied()
    }

    /// Whether a terminal `message_stop` frame has been seen.
    pub(crate) fn is_done(&self) -> bool {
        self.done_emitted
    }
}

/// Drain every complete `\n`-terminated line from `buf`, parse `data:` frames,
/// and translate them into [`StreamChunk`]s. Any partial trailing line is left
/// in `buf` for the next byte chunk.
pub(crate) fn drain_sse(
    buf: &mut Vec<u8>,
    state: &mut AnthropicStreamState,
) -> Vec<Result<StreamChunk>> {
    let mut out = Vec::new();
    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
        let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
        let line = String::from_utf8_lossy(&line_bytes);
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
            out.append(&mut handle_stream_json(&json, state));
        }
    }
    out
}

/// Translate a single decoded Anthropic stream event into zero or more
/// [`StreamChunk`]s. Pure: all cross-frame state lives in `state`.
pub(super) fn handle_stream_json(
    json: &serde_json::Value,
    state: &mut AnthropicStreamState,
) -> Vec<Result<StreamChunk>> {
    let mut out = Vec::new();
    match json.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "message_start" => {
            if let Some(msg) = json.get("message") {
                if let Some(model) = msg.get("model").and_then(|m| m.as_str())
                    && !model.is_empty()
                    && !state.model_emitted
                {
                    state.model_emitted = true;
                    out.push(Ok(StreamChunk::ModelInfo(model.to_string())));
                }
                if let Some(usage) = msg.get("usage") {
                    let (input, output, cache_read, cache_write) = parse_usage(Some(usage));
                    state.input_tokens = input;
                    state.output_tokens = output;
                    state.cache_read_tokens = cache_read;
                    state.cache_write_tokens = cache_write;
                }
            }
        }
        "content_block_start" => {
            let block_index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
            if let Some(block) = json.get("content_block")
                && block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
            {
                let id = block
                    .get("id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_index = state.assign_tool_index(block_index);
                out.push(Ok(StreamChunk::ToolCallDelta {
                    index: tool_index,
                    id: Some(id),
                    name: Some(name),
                    arguments_delta: String::new(),
                }));
            }
        }
        "content_block_delta" => {
            let block_index = json.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
            if let Some(delta) = json.get("delta") {
                match delta.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str())
                            && !text.is_empty()
                        {
                            out.push(Ok(StreamChunk::Text(text.to_string())));
                        }
                    }
                    "thinking_delta" => {
                        if let Some(thinking) = delta.get("thinking").and_then(|t| t.as_str())
                            && !thinking.is_empty()
                        {
                            out.push(Ok(StreamChunk::Reasoning(thinking.to_string())));
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(|p| p.as_str())
                            .unwrap_or("");
                        if let Some(tool_index) = state.tool_index_for(block_index) {
                            out.push(Ok(StreamChunk::ToolCallDelta {
                                index: tool_index,
                                id: None,
                                name: None,
                                arguments_delta: partial.to_string(),
                            }));
                        }
                    }
                    // signature_delta and any future block deltas are ignored.
                    _ => {}
                }
            }
        }
        "message_delta" => {
            if let Some(usage) = json.get("usage")
                && let Some(output) = usage.get("output_tokens").and_then(|t| t.as_u64())
            {
                state.output_tokens = output as u32;
            }
        }
        "message_stop" => {
            out.push(Ok(StreamChunk::Usage {
                input_tokens: state.input_tokens,
                output_tokens: state.output_tokens,
                cache_read_tokens: state.cache_read_tokens,
                cache_write_tokens: state.cache_write_tokens,
            }));
            out.push(Ok(StreamChunk::Done));
            state.done_emitted = true;
        }
        "error" => {
            let err = json.get("error");
            let err_type = err
                .and_then(|e| e.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let message = err
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("stream error")
                .to_string();
            out.push(Err(stream_error_to_llm(err_type, message).into()));
        }
        // "ping" and "content_block_stop" carry no payload we surface.
        _ => {}
    }
    out
}

/// Map a mid-stream Anthropic `error` event to a structured [`LlmError`].
fn stream_error_to_llm(err_type: &str, message: String) -> LlmError {
    match err_type {
        "overloaded_error" => LlmError::Overloaded,
        "rate_limit_error" => LlmError::RateLimit { retry_after: None },
        "authentication_error" | "permission_error" => LlmError::Auth {
            status: 401,
            message,
        },
        "api_error" => LlmError::ServerError {
            status: 500,
            message,
        },
        _ => LlmError::Other(anyhow::anyhow!("stream error: {message}")),
    }
}
