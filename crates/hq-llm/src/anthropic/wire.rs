//! Messages API request construction, buffered response parsing, and error classification.

use anyhow::{Context, Result};

use hq_core::types::{ChatMessage, MessageRole, ToolCall};

use crate::provider::{ChatRequest, LlmError, mentions_context_overflow, truncate_message};

/// Build the Anthropic Messages request body from a generic [`ChatRequest`].
///
/// Pure and deterministic so it can be unit-tested without a network. System
/// turns are hoisted into the top-level `system` field (joined in order); the
/// remaining turns keep their relative order. Consecutive turns that map to the
/// same Anthropic role are coalesced into one message with multiple content
/// blocks — this is what lets several `tool_result`s follow a multi-tool
/// assistant turn as a single `user` message, as the API expects.
pub(crate) fn build_messages_body(
    req: &ChatRequest,
    default_max_tokens: u32,
    stream: bool,
) -> serde_json::Value {
    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<serde_json::Value> = Vec::new();

    for m in &req.messages {
        match m.role {
            MessageRole::System => {
                if !m.content.is_empty() {
                    system_parts.push(m.content.clone());
                }
            }
            MessageRole::User => {
                let mut blocks = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(serde_json::json!({"type": "text", "text": m.content}));
                }
                append_message(&mut messages, "user", blocks);
            }
            MessageRole::Assistant => {
                let mut blocks = Vec::new();
                if !m.content.is_empty() {
                    blocks.push(serde_json::json!({"type": "text", "text": m.content}));
                }
                for tc in &m.tool_calls {
                    blocks.push(serde_json::json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                append_message(&mut messages, "assistant", blocks);
            }
            MessageRole::Tool => {
                let block = serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                    "content": m.content,
                });
                append_message(&mut messages, "user", vec![block]);
            }
        }
    }

    let mut body = serde_json::json!({
        "model": req.model,
        "max_tokens": req.max_tokens.unwrap_or(default_max_tokens),
        "messages": messages,
    });
    let obj = body.as_object_mut().expect("json object literal");

    if !system_parts.is_empty() {
        obj.insert(
            "system".to_string(),
            serde_json::json!(system_parts.join("\n\n")),
        );
    }
    if let Some(temp) = req.temperature {
        // The Messages API accepts temperature in [0.0, 1.0]; clamp so callers
        // tuned for OpenAI's [0.0, 2.0] range don't hard-fail with a 400.
        obj.insert(
            "temperature".to_string(),
            serde_json::json!(temp.clamp(0.0, 1.0)),
        );
    }
    if !req.tools.is_empty() {
        let tools: Vec<serde_json::Value> = req
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
        obj.insert("tools".to_string(), serde_json::json!(tools));
    }
    if stream {
        obj.insert("stream".to_string(), serde_json::json!(true));
    }

    body
}

/// Append `blocks` to `messages` for `role`, coalescing into the previous
/// message when it shares the role (the Messages API groups content blocks).
fn append_message(
    messages: &mut Vec<serde_json::Value>,
    role: &str,
    blocks: Vec<serde_json::Value>,
) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut()
        && last.get("role").and_then(|r| r.as_str()) == Some(role)
        && let Some(arr) = last.get_mut("content").and_then(|c| c.as_array_mut())
    {
        arr.extend(blocks);
        return;
    }
    messages.push(serde_json::json!({ "role": role, "content": blocks }));
}

// ─── Buffered response parsing (pure) ───────────────────────────

/// Parse a non-streaming Messages response body into a [`ChatMessage`].
pub(crate) fn parse_messages_response(json: &serde_json::Value) -> Result<ChatMessage> {
    let blocks = json
        .get("content")
        .and_then(|c| c.as_array())
        .context("Anthropic response is missing a content array")?;

    let mut text = String::new();
    let mut thinking = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "text" => text.push_str(block.get("text").and_then(|t| t.as_str()).unwrap_or("")),
            "thinking" => {
                thinking.push_str(block.get("thinking").and_then(|t| t.as_str()).unwrap_or(""))
            }
            "tool_use" => {
                tool_calls.push(ToolCall {
                    id: block
                        .get("id")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    name: block
                        .get("name")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    arguments: block
                        .get("input")
                        .cloned()
                        .unwrap_or(serde_json::Value::Object(Default::default())),
                });
            }
            _ => {}
        }
    }

    Ok(ChatMessage {
        image_parts: Vec::new(),
        role: MessageRole::Assistant,
        content: text,
        tool_calls,
        tool_call_id: None,
        reasoning_content: (!thinking.is_empty()).then_some(thinking),
    })
}

/// Extract normalized `(input, output, cache_read, cache_write)` token counts
/// from a Messages `usage` object. Missing fields default to zero.
///
/// # Normalization
///
/// Anthropic reports `input_tokens` as the *fresh* (uncached) prompt tokens
/// only — cache reads (`cache_read_input_tokens`) and cache writes
/// (`cache_creation_input_tokens`) are billed separately and are **not**
/// included in `input_tokens`. [`calculate_cost_with_cache`](hq_llm::models::calculate_cost_with_cache),
/// by contrast, assumes `input_tokens` already includes the cache-read portion
/// (it subtracts it back out to find the fresh count, mirroring OpenAI's
/// `prompt_tokens`). To make the two agree — and to guarantee the cache ratio
/// (`cache_read / input`) can never exceed 1 — the cache-read count is folded
/// into `input`. Cache writes stay separate (billed additively at the write
/// rate), exactly as the cost helper expects.
pub(crate) fn parse_usage(usage: Option<&serde_json::Value>) -> (u32, u32, u32, u32) {
    let field = |key: &str| -> u32 {
        usage
            .and_then(|u| u.get(key))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32
    };
    let fresh_input = field("input_tokens");
    let output = field("output_tokens");
    let cache_read = field("cache_read_input_tokens");
    let cache_write = field("cache_creation_input_tokens");
    // Fold cache reads into the input total so downstream cost accounting
    // (which does `fresh = input - cache_read`) recovers the true fresh count
    // and the cache ratio stays in [0, 1]. `saturating_add` guards the (never
    // observed) overflow case.
    let input = fresh_input.saturating_add(cache_read);
    (input, output, cache_read, cache_write)
}

// ─── Error classification ───────────────────────────────────────

/// Whether a 4xx message body indicates the prompt/context is too large.
/// Case-insensitive, with Anthropic's own phrasings on top of the shared ones.
fn is_context_overflow_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    mentions_context_overflow(&lower)
        || lower.contains("prompt is too long")
        || lower.contains("context length")
        || lower.contains("context window")
        || lower.contains("exceeds the maximum")
}

/// Classify a non-2xx Anthropic response into a structured [`LlmError`].
///
/// Anthropic error bodies are `{"type":"error","error":{"type","message"}}`.
/// Uses the HTTP status first, then the typed error/message for the 400 and
/// 413 capacity cases the status alone can't disambiguate.
pub(crate) fn classify_anthropic_error(status: u16, body: &[u8]) -> LlmError {
    let parsed: Option<serde_json::Value> = serde_json::from_slice(body).ok();
    let err = parsed.as_ref().and_then(|v| v.get("error"));
    let err_type = err
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let err_code = err
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let message = err
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| String::from_utf8_lossy(body).to_string());

    let truncated = || truncate_message(&message).to_string();
    match status {
        // 413 (request too large) and context-overflow 400s should trigger
        // compaction rather than a permanent failure.
        413 => LlmError::ContextOverflow {
            message: truncated(),
        },
        400 if is_context_overflow_message(&message) => LlmError::ContextOverflow {
            message: truncated(),
        },
        // Copilot's raw-token integrator rejects models the credential isn't
        // entitled to with a plain 400 rather than 401/403. That's a
        // credential problem, not a permanent one — classify like Auth so
        // the chain fails over to the next backend instead of dead-ending.
        400 if err_code == "model_not_supported"
            || err_code == "model_not_available_for_integrator" =>
        {
            LlmError::Auth {
                status,
                message: truncated(),
            }
        }
        429 | 529 | 401 | 403 | 500..=599 => LlmError::from_http(status, &message),
        _ if err_type == "overloaded_error" => LlmError::Overloaded,
        _ if err_type == "rate_limit_error" => LlmError::RateLimit { retry_after: None },
        _ => LlmError::Other(anyhow::anyhow!("Anthropic HTTP {status}: {}", truncated())),
    }
}
