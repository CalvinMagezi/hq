//! OpenAI Responses API wire format (`POST {base}/responses`).
//!
//! Some models are served only over this API, for example GitHub Copilot's
//! `gpt-6-luna`, which answers `/chat/completions` with
//! `unsupported_api_for_model`. `OpenRouterProvider` switches to these helpers
//! when its backend entry sets `wire: responses`; transport, auth and the
//! Copilot token exchange stay in `openai_compat`.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use anyhow::Result;
use hq_core::types::{ChatMessage, MessageRole, ToolCall};
use serde_json::{Value, json};

use crate::provider::{ChatRequest, ChatResponse, LlmError, StreamChunk};

/// The Responses API rejects a `call_id` longer than this.
const MAX_CALL_ID_LEN: usize = 64;

/// Error codes meaning "this backend can't serve this model", which should
/// fail over to the next backend instead of failing the turn.
const MODEL_UNAVAILABLE_CODES: [&str; 3] = [
    "unsupported_api_for_model",
    "model_not_supported",
    "model_not_available_for_integrator",
];

/// Keep a `call_id` within the API cap. Deterministic, so a replayed
/// `function_call` and its `function_call_output` still agree.
fn clamp_call_id(id: &str) -> String {
    if id.len() <= MAX_CALL_ID_LEN {
        return id.to_string();
    }
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    format!("call_{:016x}", hasher.finish())
}

fn user_item(msg: &ChatMessage) -> Value {
    let mut parts = vec![json!({"type": "input_text", "text": msg.content})];
    for image in &msg.image_parts {
        match image.to_data_url() {
            Ok(url) => parts.push(json!({"type": "input_image", "image_url": url})),
            Err(e) => {
                tracing::warn!(path = %image.path.display(), "skipping unreadable image: {e}")
            }
        }
    }
    json!({"role": "user", "content": parts})
}

fn push_assistant_items(msg: &ChatMessage, input: &mut Vec<Value>) {
    if !msg.content.is_empty() {
        input.push(json!({
            "role": "assistant",
            "content": [{"type": "output_text", "text": msg.content}],
        }));
    }
    for call in &msg.tool_calls {
        input.push(json!({
            "type": "function_call",
            "call_id": clamp_call_id(&call.id),
            "name": call.name,
            "arguments": call.arguments.to_string(),
        }));
    }
}

/// Build a Responses request body. System messages become `instructions`.
pub fn build_body(request: &ChatRequest, stream: bool) -> Value {
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    for msg in &request.messages {
        match msg.role {
            MessageRole::System => instructions.push(msg.content.as_str()),
            MessageRole::User => input.push(user_item(msg)),
            MessageRole::Assistant => push_assistant_items(msg, &mut input),
            MessageRole::Tool => input.push(json!({
                "type": "function_call_output",
                "call_id": clamp_call_id(msg.tool_call_id.as_deref().unwrap_or_default()),
                "output": msg.content,
            })),
        }
    }

    let mut body = json!({
        "model": request.model,
        "input": input,
        "stream": stream,
        "store": false,
    });
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions.join("\n\n"));
    }
    if let Some(max) = request.max_tokens {
        body["max_output_tokens"] = json!(max);
    }
    if !request.tools.is_empty() {
        // strict defaults on at Copilot, and strict mode rejects the optional
        // parameters most HQ tools declare.
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                    "strict": false,
                })
            })
            .collect();
        body["tools"] = json!(tools);
    }
    body
}

/// Classify an error body from a non-2xx response or an `error` event.
pub fn classify_error(status: u16, body: &str) -> LlmError {
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let err = parsed.get("error").unwrap_or(&parsed);
    let code = err.get("code").and_then(Value::as_str).unwrap_or_default();
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or(body)
        .chars()
        .take(300)
        .collect::<String>();
    if MODEL_UNAVAILABLE_CODES.contains(&code) {
        return LlmError::Auth {
            status,
            message: format!("{code}: {message}"),
        };
    }
    match status {
        400 if code == "context_length_exceeded" => LlmError::ContextOverflow { message },
        // Any other 4xx from this path is most likely a wire-format problem on
        // our side; fail over rather than hard-failing every turn.
        400..=499 if status != 429 => LlmError::ServerError {
            status,
            message: format!("{code}: {message}"),
        },
        _ => LlmError::from_http(status, &message),
    }
}

fn usage_of(response: &Value) -> (u32, u32, u32, u32) {
    let usage = &response["usage"];
    let n = |v: &Value| v.as_u64().unwrap_or(0) as u32;
    (
        n(&usage["input_tokens"]),
        n(&usage["output_tokens"]),
        n(&usage["input_tokens_details"]["cached_tokens"]),
        n(&usage["input_tokens_details"]["cache_write_tokens"]),
    )
}

/// Parse a buffered (non-streaming) Responses reply.
pub fn parse_response(json: &Value, fallback_model: &str) -> Result<ChatResponse> {
    if let Some(err) = json.get("error").filter(|e| !e.is_null()) {
        return Err(classify_error(400, &json!({ "error": err }).to_string()).into());
    }
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for item in json["output"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("message") => {
                for part in item["content"].as_array().into_iter().flatten() {
                    if part["type"] == "output_text" {
                        text.push_str(part["text"].as_str().unwrap_or_default());
                    }
                }
            }
            Some("function_call") => tool_calls.push(ToolCall {
                id: item["call_id"].as_str().unwrap_or_default().to_string(),
                name: item["name"].as_str().unwrap_or_default().to_string(),
                arguments: serde_json::from_str(item["arguments"].as_str().unwrap_or("{}"))
                    .unwrap_or_else(|_| json!({})),
            }),
            _ => {}
        }
    }
    let (input_tokens, output_tokens, cache_read_tokens, cache_write_tokens) = usage_of(json);
    Ok(ChatResponse {
        message: ChatMessage {
            role: MessageRole::Assistant,
            content: text,
            tool_calls,
            tool_call_id: None,
            reasoning_content: None,
            image_parts: Vec::new(),
        },
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        model: json["model"].as_str().unwrap_or(fallback_model).to_string(),
    })
}

/// Turns Responses SSE events into [`StreamChunk`]s.
///
/// Deltas are keyed by `output_index`: Copilot re-encrypts `item_id` on every
/// frame, so it can't identify an item across events.
#[derive(Default)]
pub struct StreamState {
    tool_index_by_output: HashMap<u64, usize>,
    completed: bool,
}

impl StreamState {
    pub fn completed(&self) -> bool {
        self.completed
    }

    fn tool_delta(&self, event: &Value) -> Option<StreamChunk> {
        let output_index = event["output_index"].as_u64()?;
        let index = *self.tool_index_by_output.get(&output_index)?;
        Some(StreamChunk::ToolCallDelta {
            index,
            id: None,
            name: None,
            arguments_delta: event["delta"].as_str().unwrap_or_default().to_string(),
        })
    }

    fn item_added(&mut self, event: &Value) -> Option<StreamChunk> {
        let item = &event["item"];
        if item["type"] != "function_call" {
            return None;
        }
        let index = self.tool_index_by_output.len();
        self.tool_index_by_output.insert(
            event["output_index"].as_u64().unwrap_or(index as u64),
            index,
        );
        Some(StreamChunk::ToolCallDelta {
            index,
            id: item["call_id"].as_str().map(str::to_string),
            name: item["name"].as_str().map(str::to_string),
            arguments_delta: item["arguments"].as_str().unwrap_or_default().to_string(),
        })
    }

    fn finish(&mut self, response: &Value) -> Vec<StreamChunk> {
        self.completed = true;
        let (input_tokens, output_tokens, cache_read_tokens, cache_write_tokens) =
            usage_of(response);
        let mut chunks = Vec::new();
        if let Some(model) = response["model"].as_str() {
            chunks.push(StreamChunk::ModelInfo(model.to_string()));
        }
        chunks.push(StreamChunk::Usage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        });
        chunks.push(StreamChunk::Done);
        chunks
    }

    /// Handle one SSE `data:` payload.
    pub fn on_event(&mut self, payload: &str) -> Result<Vec<StreamChunk>> {
        let event: Value = serde_json::from_str(payload)
            .map_err(|e| LlmError::Network(format!("bad Responses SSE frame: {e}")))?;
        let text = |key: &str| event[key].as_str().unwrap_or_default().to_string();
        let chunk = match event["type"].as_str().unwrap_or_default() {
            "response.output_text.delta" => Some(StreamChunk::Text(text("delta"))),
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                Some(StreamChunk::Reasoning(text("delta")))
            }
            "response.output_item.added" => self.item_added(&event),
            "response.function_call_arguments.delta" => self.tool_delta(&event),
            "response.completed" | "response.incomplete" => {
                return Ok(self.finish(&event["response"]));
            }
            "response.failed" => {
                let err = json!({ "error": event["response"]["error"] }).to_string();
                return Err(classify_error(500, &err).into());
            }
            "error" => return Err(classify_error(500, &event.to_string()).into()),
            _ => None,
        };
        Ok(chunk.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::ToolDefinition;

    /// Real Copilot `gpt-6-luna` frames captured 2026-09-23, trimmed to the
    /// fields the parser reads. `item_id` differs on every frame upstream.
    const TOOL_TURN: &str = include_str!("../tests/fixtures/responses_tool_turn.sse");
    const TEXT_TURN: &str = include_str!("../tests/fixtures/responses_text_turn.sse");

    fn run(sse: &str) -> (Vec<StreamChunk>, bool) {
        let mut state = StreamState::default();
        let mut chunks = Vec::new();
        for line in sse.lines() {
            if let Some(payload) = line.strip_prefix("data:") {
                chunks.extend(state.on_event(payload.trim()).unwrap());
            }
        }
        (chunks, state.completed())
    }

    fn msg(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: content.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
            image_parts: Vec::new(),
        }
    }

    #[test]
    fn tool_turn_assembles_one_call_with_its_call_id() {
        let (chunks, completed) = run(TOOL_TURN);
        assert!(completed);
        let mut id = None;
        let mut args = String::new();
        for c in &chunks {
            if let StreamChunk::ToolCallDelta {
                index,
                id: i,
                name,
                arguments_delta,
            } = c
            {
                assert_eq!(*index, 0);
                if i.is_some() {
                    id = i.clone();
                    assert_eq!(name.as_deref(), Some("get_weather"));
                }
                args.push_str(arguments_delta);
            }
        }
        assert_eq!(id.as_deref(), Some("call_RVqN1fvHsYXp71f5FJQxxdB0"));
        assert_eq!(args, r#"{"city":"London"}"#);
        assert!(matches!(chunks.last(), Some(StreamChunk::Done)));
        assert!(chunks.iter().any(|c| matches!(
            c,
            StreamChunk::Usage {
                input_tokens: 55,
                output_tokens: 20,
                ..
            }
        )));
    }

    #[test]
    fn text_turn_streams_text_then_usage_then_done() {
        let (chunks, completed) = run(TEXT_TURN);
        assert!(completed);
        let text: String = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "London: 24°C with light rain.");
        assert!(matches!(chunks.last(), Some(StreamChunk::Done)));
    }

    #[test]
    fn stream_without_completed_is_not_complete() {
        let cut: String = TEXT_TURN
            .lines()
            .filter(|l| !l.contains("response.completed"))
            .collect::<Vec<_>>()
            .join("\n");
        let (chunks, completed) = run(&cut);
        assert!(!completed);
        assert!(!chunks.iter().any(|c| matches!(c, StreamChunk::Done)));
    }

    #[test]
    fn body_maps_history_to_responses_items() {
        let mut assistant = msg(MessageRole::Assistant, "");
        assistant.tool_calls.push(ToolCall {
            id: "x".repeat(80),
            name: "get_weather".into(),
            arguments: json!({"city": "London"}),
        });
        let mut tool = msg(MessageRole::Tool, "24C");
        tool.tool_call_id = Some("x".repeat(80));
        let request = ChatRequest {
            model: "gpt-6-luna".into(),
            messages: vec![
                msg(MessageRole::System, "be terse"),
                msg(MessageRole::User, "weather?"),
                assistant,
                tool,
            ],
            tools: vec![ToolDefinition {
                name: "get_weather".into(),
                description: "Get weather".into(),
                parameters: json!({"type": "object"}),
            }],
            temperature: Some(0.2),
            max_tokens: Some(100),
        };
        let body = build_body(&request, true);
        assert_eq!(body["instructions"], "be terse");
        assert_eq!(body["max_output_tokens"], 100);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["tools"][0]["name"], "get_weather");
        assert_eq!(body["tools"][0]["strict"], false);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["arguments"], r#"{"city":"London"}"#);
        assert_eq!(input[2]["type"], "function_call_output");
        let call_id = input[1]["call_id"].as_str().unwrap();
        assert!(call_id.len() <= MAX_CALL_ID_LEN);
        assert_eq!(call_id, input[2]["call_id"].as_str().unwrap());
    }

    #[test]
    fn unsupported_model_errors_fail_over() {
        let body = r#"{"error":{"message":"model \"gpt-6-luna\" is not accessible via the /chat/completions endpoint","code":"unsupported_api_for_model"}}"#;
        assert!(matches!(classify_error(400, body), LlmError::Auth { .. }));
        let other = r#"{"error":{"message":"bad input","code":"invalid_value"}}"#;
        assert!(classify_error(400, other).is_transient());
    }

    #[test]
    fn buffered_reply_parses_text_tools_and_cache_usage() {
        let reply = json!({
            "model": "gpt-6-luna",
            "output": [
                {"type": "function_call", "call_id": "call_1", "name": "f", "arguments": "{\"a\":1}"},
                {"type": "message", "content": [{"type": "output_text", "text": "hi"}]}
            ],
            "usage": {"input_tokens": 86, "output_tokens": 12,
                      "input_tokens_details": {"cached_tokens": 40, "cache_write_tokens": 0}}
        });
        let r = parse_response(&reply, "fallback").unwrap();
        assert_eq!(r.message.content, "hi");
        assert_eq!(r.message.tool_calls[0].arguments, json!({"a": 1}));
        assert_eq!((r.input_tokens, r.cache_read_tokens), (86, 40));
        assert_eq!(r.model, "gpt-6-luna");
    }
}

/// Live round trip against Copilot's `gpt-6-luna`. Opt in with
/// `COPILOT_GITHUB_TOKEN=... cargo test -p hq-llm responses_live -- --ignored`.
#[cfg(test)]
mod live {
    use super::*;
    use crate::openai_compat::OpenRouterProvider;
    use crate::provider::LlmProvider;
    use futures::StreamExt;
    use hq_core::types::ToolDefinition;

    fn message(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: content.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
            image_parts: Vec::new(),
        }
    }

    #[tokio::test]
    #[ignore = "needs COPILOT_GITHUB_TOKEN and network"]
    async fn responses_live_tool_round_trip() {
        let token = std::env::var("COPILOT_GITHUB_TOKEN").expect("COPILOT_GITHUB_TOKEN");
        let provider = OpenRouterProvider::new_with_base(&token, "https://api.githubcopilot.com")
            .with_responses_api(true);
        let mut request = ChatRequest {
            model: "gpt-6-luna".into(),
            messages: vec![
                message(
                    MessageRole::System,
                    "Be terse. Always use tools when asked.",
                ),
                message(
                    MessageRole::User,
                    "What's the weather in London? Use the tool.",
                ),
            ],
            tools: vec![ToolDefinition {
                name: "get_weather".into(),
                description: "Get the weather for a city".into(),
                parameters: json!({"type": "object",
                    "properties": {"city": {"type": "string"}, "units": {"type": "string"}},
                    "required": ["city"]}),
            }],
            temperature: None,
            max_tokens: Some(300),
        };

        let mut stream = provider.chat_stream(&request).await.unwrap();
        let (mut id, mut name, mut args, mut done) =
            (String::new(), String::new(), String::new(), false);
        while let Some(chunk) = stream.next().await {
            match chunk.unwrap() {
                StreamChunk::ToolCallDelta {
                    id: i,
                    name: n,
                    arguments_delta,
                    ..
                } => {
                    id = i.unwrap_or(id);
                    name = n.unwrap_or(name);
                    args.push_str(&arguments_delta);
                }
                StreamChunk::Done => done = true,
                _ => {}
            }
        }
        assert!(done, "stream ended without Done");
        assert_eq!(name, "get_weather");

        let mut assistant = message(MessageRole::Assistant, "");
        assistant.tool_calls.push(ToolCall {
            id: id.clone(),
            name,
            arguments: serde_json::from_str(&args).unwrap(),
        });
        let mut result = message(MessageRole::Tool, "24C, light rain");
        result.tool_call_id = Some(id);
        request.messages.extend([assistant, result]);

        let reply = provider.chat(&request).await.unwrap();
        assert!(
            reply.message.content.contains("24"),
            "{}",
            reply.message.content
        );
        assert!(reply.input_tokens > 0);
    }
}
