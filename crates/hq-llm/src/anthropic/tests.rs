use super::stream::handle_stream_json;
use super::*;
use hq_core::types::{ChatMessage, MessageRole, ToolCall, ToolDefinition};
use serde_json::json;

fn msg(role: MessageRole, content: &str) -> ChatMessage {
    ChatMessage {
        image_parts: Vec::new(),
        role,
        content: content.to_string(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    }
}

// ─── Request construction ───────────────────────────────────

#[test]
fn system_turns_are_hoisted_and_ordering_is_preserved() {
    let req = ChatRequest {
        model: "claude-sonnet-4-6".to_string(),
        messages: vec![
            msg(MessageRole::System, "you are helpful"),
            msg(MessageRole::User, "hello"),
            msg(MessageRole::System, "be terse"),
            msg(MessageRole::Assistant, "hi there"),
            msg(MessageRole::User, "thanks"),
        ],
        tools: vec![],
        temperature: None,
        max_tokens: None,
    };

    let body = build_messages_body(&req, DEFAULT_MAX_TOKENS, false);

    // Both system turns joined in order, hoisted out of `messages`.
    assert_eq!(body["system"], json!("you are helpful\n\nbe terse"));
    assert_eq!(body["max_tokens"], json!(DEFAULT_MAX_TOKENS));

    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"][0]["text"], "hello");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"][0]["text"], "hi there");
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(messages[2]["content"][0]["text"], "thanks");
    // No system message leaks into the messages array.
    assert!(messages.iter().all(|m| m["role"] != "system"));
}

#[test]
fn assistant_tool_calls_map_to_tool_use_blocks() {
    let assistant = ChatMessage {
        image_parts: Vec::new(),
        role: MessageRole::Assistant,
        content: "calling".to_string(),
        tool_calls: vec![ToolCall {
            id: "toolu_1".to_string(),
            name: "search".to_string(),
            arguments: json!({"q": "rust"}),
        }],
        tool_call_id: None,
        reasoning_content: None,
    };
    let req = ChatRequest {
        model: "claude".to_string(),
        messages: vec![msg(MessageRole::User, "find"), assistant],
        tools: vec![],
        temperature: None,
        max_tokens: Some(1024),
    };

    let body = build_messages_body(&req, DEFAULT_MAX_TOKENS, false);
    assert_eq!(body["max_tokens"], json!(1024));

    let content = &body["messages"][1]["content"];
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "calling");
    assert_eq!(content[1]["type"], "tool_use");
    assert_eq!(content[1]["id"], "toolu_1");
    assert_eq!(content[1]["name"], "search");
    assert_eq!(content[1]["input"], json!({"q": "rust"}));
}

#[test]
fn consecutive_tool_results_coalesce_into_one_user_message() {
    let tool_result = |id: &str, out: &str| ChatMessage {
        image_parts: Vec::new(),
        role: MessageRole::Tool,
        content: out.to_string(),
        tool_calls: vec![],
        tool_call_id: Some(id.to_string()),
        reasoning_content: None,
    };
    let req = ChatRequest {
        model: "claude".to_string(),
        messages: vec![
            msg(MessageRole::User, "go"),
            ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Assistant,
                content: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "a".into(),
                        name: "t1".into(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "b".into(),
                        name: "t2".into(),
                        arguments: json!({}),
                    },
                ],
                tool_call_id: None,
                reasoning_content: None,
            },
            tool_result("a", "one"),
            tool_result("b", "two"),
        ],
        tools: vec![],
        temperature: None,
        max_tokens: None,
    };

    let body = build_messages_body(&req, DEFAULT_MAX_TOKENS, false);
    let messages = body["messages"].as_array().unwrap();
    // user(go) | assistant(2 tool_use) | user(2 tool_result coalesced)
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2]["role"], "user");
    let results = messages[2]["content"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["type"], "tool_result");
    assert_eq!(results[0]["tool_use_id"], "a");
    assert_eq!(results[0]["content"], "one");
    assert_eq!(results[1]["tool_use_id"], "b");
    assert_eq!(results[1]["content"], "two");
}

#[test]
fn tools_and_stream_and_temperature_serialize() {
    let req = ChatRequest {
        model: "claude".to_string(),
        messages: vec![msg(MessageRole::User, "hi")],
        tools: vec![ToolDefinition {
            name: "get_weather".to_string(),
            description: "look up weather".to_string(),
            parameters: json!({"type": "object", "properties": {}}),
        }],
        temperature: Some(1.7),
        max_tokens: None,
    };

    let body = build_messages_body(&req, DEFAULT_MAX_TOKENS, true);
    assert_eq!(body["stream"], json!(true));
    // Clamped from OpenAI's 2.0 ceiling to Anthropic's 1.0.
    assert_eq!(body["temperature"], json!(1.0));
    let tool = &body["tools"][0];
    assert_eq!(tool["name"], "get_weather");
    assert_eq!(tool["description"], "look up weather");
    assert_eq!(
        tool["input_schema"],
        json!({"type": "object", "properties": {}})
    );
}

// ─── Buffered response parsing ──────────────────────────────

#[test]
fn parse_response_collects_text_thinking_and_tool_use() {
    let json = json!({
        "model": "claude-sonnet-4-6",
        "content": [
            {"type": "thinking", "thinking": "let me think"},
            {"type": "text", "text": "Here is "},
            {"type": "text", "text": "the answer"},
            {"type": "tool_use", "id": "toolu_9", "name": "calc", "input": {"x": 1}}
        ],
        "usage": {"input_tokens": 12, "output_tokens": 5, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 7}
    });

    let message = parse_messages_response(&json).unwrap();
    assert_eq!(message.content, "Here is the answer");
    assert_eq!(message.reasoning_content.as_deref(), Some("let me think"));
    assert_eq!(message.tool_calls.len(), 1);
    assert_eq!(message.tool_calls[0].id, "toolu_9");
    assert_eq!(message.tool_calls[0].name, "calc");
    assert_eq!(message.tool_calls[0].arguments, json!({"x": 1}));

    let (input, output, cache_read, cache_write) = parse_usage(json.get("usage"));
    // Normalized: fresh input (12) folds in cache reads (3) so cost accounting
    // and the cache ratio line up with `calculate_cost_with_cache`.
    assert_eq!((input, output, cache_read, cache_write), (15, 5, 3, 7));
}

#[test]
fn parse_usage_folds_cache_reads_into_input_and_bounds_the_ratio() {
    // Actual Anthropic shape: input_tokens is the *fresh* count; cache reads
    // and creations are reported alongside, not inside it.
    let usage = json!({
        "input_tokens": 1000,
        "output_tokens": 200,
        "cache_read_input_tokens": 4000,
        "cache_creation_input_tokens": 500
    });
    let (input, output, cache_read, cache_write) = parse_usage(Some(&usage));
    // Cache reads folded in: 1000 fresh + 4000 cached = 5000 total input.
    assert_eq!(input, 5000);
    assert_eq!(output, 200);
    assert_eq!(cache_read, 4000);
    assert_eq!(cache_write, 500);
    // The cache ratio can never exceed 1 after normalization.
    assert!(cache_read <= input);
    // And the fresh count the cost helper recovers matches the raw report.
    assert_eq!(input - cache_read, 1000);
}

#[test]
fn parse_usage_without_cache_fields_is_unchanged() {
    // Providers/turns with no prompt caching report only input/output.
    let usage = json!({"input_tokens": 42, "output_tokens": 7});
    assert_eq!(parse_usage(Some(&usage)), (42, 7, 0, 0));
}

// ─── SSE streaming ──────────────────────────────────────────

#[test]
fn stream_message_start_emits_model_info_once() {
    let mut state = AnthropicStreamState::default();
    let frame = json!({
        "type": "message_start",
        "message": {"model": "claude-sonnet-4-6", "usage": {"input_tokens": 20, "output_tokens": 1}}
    });
    let chunks = handle_stream_json(&frame, &mut state);
    assert!(
        matches!(chunks.as_slice(), [Ok(StreamChunk::ModelInfo(m))] if m == "claude-sonnet-4-6")
    );
    assert_eq!(state.input_tokens, 20);
    // A second message_start does not re-emit the model.
    assert!(handle_stream_json(&frame, &mut state).is_empty());
}

#[test]
fn stream_text_and_thinking_deltas_translate() {
    let mut state = AnthropicStreamState::default();
    let text = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {"type": "text_delta", "text": "Hello"}
    });
    assert!(matches!(
        handle_stream_json(&text, &mut state).as_slice(),
        [Ok(StreamChunk::Text(t))] if t == "Hello"
    ));

    let thinking = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {"type": "thinking_delta", "thinking": "hmm"}
    });
    assert!(matches!(
        handle_stream_json(&thinking, &mut state).as_slice(),
        [Ok(StreamChunk::Reasoning(t))] if t == "hmm"
    ));
}

#[test]
fn stream_tool_use_start_and_input_json_delta_assemble_by_index() {
    let mut state = AnthropicStreamState::default();

    // A leading text block occupies content index 0.
    let text = json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": {"type": "text_delta", "text": "ok"}
    });
    let _ = handle_stream_json(&text, &mut state);

    // tool_use starts at content-block index 1 → remapped to tool index 0.
    let start = json!({
        "type": "content_block_start",
        "index": 1,
        "content_block": {"type": "tool_use", "id": "toolu_1", "name": "search"}
    });
    let chunks = handle_stream_json(&start, &mut state);
    match chunks.as_slice() {
        [
            Ok(StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            }),
        ] => {
            assert_eq!(*index, 0);
            assert_eq!(id.as_deref(), Some("toolu_1"));
            assert_eq!(name.as_deref(), Some("search"));
            assert!(arguments_delta.is_empty());
        }
        other => panic!("unexpected: {other:?}"),
    }

    // Argument deltas arrive on content-block index 1 and map to tool index 0.
    let delta = json!({
        "type": "content_block_delta",
        "index": 1,
        "delta": {"type": "input_json_delta", "partial_json": "{\"q\":"}
    });
    match handle_stream_json(&delta, &mut state).as_slice() {
        [
            Ok(StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            }),
        ] => {
            assert_eq!(*index, 0);
            assert!(id.is_none());
            assert!(name.is_none());
            assert_eq!(arguments_delta, "{\"q\":");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn stream_message_stop_emits_usage_then_done() {
    let mut state = AnthropicStreamState {
        input_tokens: 10,
        ..Default::default()
    };
    let md = json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 42}});
    assert!(handle_stream_json(&md, &mut state).is_empty());
    assert_eq!(state.output_tokens, 42);

    let stop = json!({"type": "message_stop"});
    let chunks = handle_stream_json(&stop, &mut state);
    assert!(matches!(
        chunks.as_slice(),
        [
            Ok(StreamChunk::Usage {
                input_tokens: 10,
                output_tokens: 42,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }),
            Ok(StreamChunk::Done)
        ]
    ));
    assert!(state.done_emitted);
}

#[test]
fn stream_message_start_captures_cache_split_into_usage() {
    // The `message_start` usage object carries the prompt-cache split; it must
    // be threaded into the terminal `StreamChunk::Usage` so streaming callers
    // account cache tokens identically to the buffered path.
    let mut state = AnthropicStreamState::default();
    let start = json!({
        "type": "message_start",
        "message": {
            "model": "claude-sonnet-4-6",
            "usage": {
                "input_tokens": 30,
                "output_tokens": 1,
                "cache_read_input_tokens": 25,
                "cache_creation_input_tokens": 4
            }
        }
    });
    let _ = handle_stream_json(&start, &mut state);
    assert_eq!(state.cache_read_tokens, 25);
    assert_eq!(state.cache_write_tokens, 4);
    // input_tokens folds in the cache-read count (30 fresh + 25 cached = 55)
    // so streaming cost accounting matches the buffered path.
    assert_eq!(state.input_tokens, 55);

    let stop = json!({"type": "message_stop"});
    let chunks = handle_stream_json(&stop, &mut state);
    assert!(matches!(
        chunks.as_slice(),
        [
            Ok(StreamChunk::Usage {
                input_tokens: 55,
                cache_read_tokens: 25,
                cache_write_tokens: 4,
                ..
            }),
            Ok(StreamChunk::Done)
        ]
    ));
}

#[test]
fn stream_error_event_becomes_structured_error() {
    let mut state = AnthropicStreamState::default();
    let frame =
        json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}});
    let chunks = handle_stream_json(&frame, &mut state);
    assert_eq!(chunks.len(), 1);
    let err = chunks.into_iter().next().unwrap().unwrap_err();
    assert!(
        err.downcast_ref::<LlmError>()
            .is_some_and(|e| matches!(e, LlmError::Overloaded))
    );
}

#[test]
fn drain_sse_buffers_frames_split_across_byte_chunks() {
    let mut state = AnthropicStreamState::default();
    let mut buf: Vec<u8> = Vec::new();

    // First byte chunk ends mid-frame (no trailing newline on the data line).
    buf.extend_from_slice(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-x\",\"usage\":{\"input_tokens\":5}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel");
    let first = drain_sse(&mut buf, &mut state);
    assert!(matches!(first.as_slice(), [Ok(StreamChunk::ModelInfo(m))] if m == "claude-x"));

    // Second byte chunk completes the split text frame.
    buf.extend_from_slice(b"lo\"}}\n\n");
    let second = drain_sse(&mut buf, &mut state);
    assert!(matches!(second.as_slice(), [Ok(StreamChunk::Text(t))] if t == "Hello"));
}

// ─── HTTP error classification ──────────────────────────────

#[test]
fn classify_error_maps_statuses_and_bodies() {
    let rate = classify_anthropic_error(
        429,
        br#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#,
    );
    assert!(matches!(rate, LlmError::RateLimit { .. }));

    let auth = classify_anthropic_error(
        401,
        br#"{"type":"error","error":{"type":"authentication_error","message":"bad key"}}"#,
    );
    assert!(matches!(auth, LlmError::Auth { status: 401, .. }));

    let overloaded = classify_anthropic_error(
        529,
        br#"{"type":"error","error":{"type":"overloaded_error","message":"overloaded"}}"#,
    );
    assert!(matches!(overloaded, LlmError::Overloaded));

    let server = classify_anthropic_error(
        500,
        br#"{"type":"error","error":{"type":"api_error","message":"boom"}}"#,
    );
    assert!(matches!(server, LlmError::ServerError { status: 500, .. }));

    let overflow = classify_anthropic_error(
        400,
        br#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 250000 tokens > 200000 maximum"}}"#,
    );
    assert!(matches!(overflow, LlmError::ContextOverflow { .. }));

    let too_large = classify_anthropic_error(
        413,
        br#"{"type":"error","error":{"type":"request_too_large","message":"body too big"}}"#,
    );
    assert!(matches!(too_large, LlmError::ContextOverflow { .. }));
}

// ─── Streaming transport (truncation) ───────────────────────

#[tokio::test]
async fn stream_premature_eof_yields_error_not_synthetic_done() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // A local server that emits `message_start` + one text delta, then closes
    // the connection WITHOUT a `message_stop` frame — a truncated turn.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut scratch = [0u8; 2048];
        let _ = sock.read(&mut scratch).await; // drain request headers
        let body = "event: message_start\n\
             data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n\
             event: content_block_delta\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Partial\"}}\n\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
        );
        sock.write_all(response.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
        // Socket dropped here → EOF with no `message_stop`.
    });

    let provider = AnthropicProvider::new_with_base("test-key", &format!("http://{addr}"));
    let request = ChatRequest {
        model: "claude-sonnet-4-6".into(),
        messages: vec![msg(MessageRole::User, "hi")],
        ..Default::default()
    };
    let mut stream = provider.chat_stream(&request).await.unwrap();
    let mut chunks: Vec<Result<StreamChunk>> = Vec::new();
    while let Some(item) = stream.next().await {
        chunks.push(item);
    }
    server.await.unwrap();

    // The partial text surfaced,
    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, Ok(StreamChunk::Text(t)) if t == "Partial"))
    );
    // no terminal `Done` was fabricated for the truncated stream,
    assert!(
        !chunks.iter().any(|c| matches!(c, Ok(StreamChunk::Done))),
        "must not synthesize Done on premature EOF"
    );
    // and the stream ends with a typed transport error.
    assert!(matches!(chunks.last(), Some(Err(_))));
}

#[tokio::test]
async fn dropping_the_stream_aborts_a_hung_transport_immediately() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // A local server that emits `message_start` + one text delta, then HANGS
    // mid-response: it never sends `message_stop` and never closes the socket.
    // After hanging, it waits to observe the *client* closing the connection
    // (a read returning 0 bytes) — which only happens if dropping the stream
    // tears the transport down instead of leaking the parked pump task.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut scratch = [0u8; 2048];
        let _ = sock.read(&mut scratch).await; // drain request headers
        let body = "event: message_start\n\
             data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1}}}\n\n\
             event: content_block_delta\n\
             data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Partial\"}}\n\n";
        let response = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n{body}");
        sock.write_all(response.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
        // Hang: never send `message_stop`. Block on reads waiting for the
        // client to close (EOF). Returns whether that close was observed
        // promptly — false means the pump task leaked the connection.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match sock.read(&mut scratch).await {
                    Ok(0) => return true,  // client closed the socket
                    Ok(_) => continue,     // unexpected extra bytes; keep waiting
                    Err(_) => return true, // torn-down connection also counts
                }
            }
        })
        .await
        .unwrap_or(false)
    });

    let provider = AnthropicProvider::new_with_base("test-key", &format!("http://{addr}"));
    let request = ChatRequest {
        model: "claude-sonnet-4-6".into(),
        messages: vec![msg(MessageRole::User, "hi")],
        ..Default::default()
    };
    let mut stream = provider.chat_stream(&request).await.unwrap();

    // Pull until the partial text arrives, proving the stream is live before
    // we cancel it while the server is still hung.
    let mut saw_partial = false;
    while let Some(item) = stream.next().await {
        if matches!(&item, Ok(StreamChunk::Text(t)) if t == "Partial") {
            saw_partial = true;
            break;
        }
    }
    assert!(saw_partial, "expected the partial text before cancelling");

    // Cancel by dropping the stream. The pump task must abort and drop the
    // transport now, not linger on the hung `byte_stream.next()`.
    drop(stream);

    let observed_close = server.await.unwrap();
    assert!(
        observed_close,
        "dropping the stream must tear down the hung transport promptly; \
         the pump task leaked the connection instead"
    );
}
