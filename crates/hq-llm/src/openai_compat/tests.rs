use super::stream::{
    finalize_openai_stream, parse_stream_chunk, stream_delta_to_chunks, stream_response_to_chunks,
    usage_to_chunk,
};
use super::*;
use crate::provider::{LlmProvider, StreamChunk};
use anyhow::Result;
use async_openai::types::ChatCompletionStreamResponseDelta;
use async_openai::types::{
    ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContent,
    ChatCompletionRequestUserMessageContentPart, CompletionUsage,
    CreateChatCompletionStreamResponse,
};
use futures::StreamExt;
use hq_core::types::ChatMessage;

/// Regression: GitHub Copilot's `/chat/completions` proxy omits `object`
/// on every streamed chunk (live-verified shape, minus unrelated fields).
/// async-openai's typed struct requires it, so this must be patched in
/// rather than failing every chunk from that endpoint.
#[test]
fn parses_a_stream_chunk_missing_the_object_field() {
    let payload = r#"{"choices":[{"index":0,"delta":{"content":"hi","role":"assistant"}}],
        "created":1789629918,"id":"abc","model":"gemini-3.8-flash",
        "usage":{"prompt_tokens":5,"completion_tokens":1,"total_tokens":6}}"#;
    let chunk = parse_stream_chunk(payload).expect("missing `object` must be tolerated");
    assert_eq!(chunk.model, "gemini-3.8-flash");
    assert_eq!(chunk.object, "chat.completion.chunk");
}

/// A compliant payload that already carries `object` round-trips
/// unchanged — the patch must be a no-op, not an overwrite.
#[test]
fn parses_a_compliant_stream_chunk_unchanged() {
    let payload = r#"{"choices":[],"created":0,"id":"x","model":"gpt-x",
        "object":"chat.completion.chunk"}"#;
    let chunk = parse_stream_chunk(payload).unwrap();
    assert_eq!(chunk.object, "chat.completion.chunk");
}

/// Deserialize a stream delta from the OpenAI wire shape (avoids touching the
/// deprecated `function_call` field directly).
fn delta(value: serde_json::Value) -> ChatCompletionStreamResponseDelta {
    serde_json::from_value(value).expect("valid stream delta")
}

/// Regression: a single upstream chunk that carries two parallel tool calls
/// must flatten into two `ToolCallDelta`s. The previous `map`-based
/// translation returned only the first entry, silently dropping the rest.
#[test]
fn parallel_tool_calls_in_one_chunk_all_flatten() {
    let d = delta(serde_json::json!({
        "tool_calls": [
            {
                "index": 0,
                "id": "call_a",
                "type": "function",
                "function": {"name": "search", "arguments": "{\"q\":\"a\"}"}
            },
            {
                "index": 1,
                "id": "call_b",
                "type": "function",
                "function": {"name": "fetch", "arguments": "{\"url\":\"b\"}"}
            }
        ]
    }));

    let chunks = stream_delta_to_chunks(&d, false, "");
    let calls: Vec<(usize, Option<String>, Option<String>, String)> = chunks
        .into_iter()
        .filter_map(|c| match c {
            Ok(StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            }) => Some((index, id, name, arguments_delta)),
            _ => None,
        })
        .collect();

    assert_eq!(calls.len(), 2, "both parallel tool calls must survive");
    assert_eq!(calls[0].0, 0);
    assert_eq!(calls[0].1.as_deref(), Some("call_a"));
    assert_eq!(calls[0].2.as_deref(), Some("search"));
    assert!(calls[0].3.contains("\"q\""));
    assert_eq!(calls[1].0, 1);
    assert_eq!(calls[1].1.as_deref(), Some("call_b"));
    assert_eq!(calls[1].2.as_deref(), Some("fetch"));
    assert!(calls[1].3.contains("\"url\""));
}

/// Order is preserved: tool deltas, then text, then model info on the finish
/// chunk. The terminal `Done` is appended by the stream layer, not here.
#[test]
fn tool_text_and_finish_preserve_order() {
    let d = delta(serde_json::json!({
        "content": "hi",
        "tool_calls": [
            {"index": 0, "id": "c", "type": "function", "function": {"name": "t", "arguments": ""}}
        ]
    }));

    let kinds: Vec<&str> = stream_delta_to_chunks(&d, true, "gpt-x")
        .iter()
        .map(|c| match c {
            Ok(StreamChunk::ToolCallDelta { .. }) => "tool",
            Ok(StreamChunk::Text(_)) => "text",
            Ok(StreamChunk::ModelInfo(_)) => "model",
            Ok(StreamChunk::Usage { .. }) => "usage",
            Ok(StreamChunk::Done) => "done",
            _ => "other",
        })
        .collect();

    // No terminal Done here — the delta mapper never emits it.
    assert_eq!(kinds, vec!["tool", "text", "model"]);
}

/// A keep-alive chunk (no content, no tool calls, no finish) emits nothing.
#[test]
fn empty_delta_emits_nothing() {
    let d = delta(serde_json::json!({}));
    assert!(stream_delta_to_chunks(&d, false, "").is_empty());
}

/// Deserialize a full stream response chunk from the OpenAI wire shape.
fn response(value: serde_json::Value) -> CreateChatCompletionStreamResponse {
    serde_json::from_value(value).expect("valid stream response")
}

/// The trailing usage-only chunk (empty `choices`, populated `usage`) maps to
/// a `StreamChunk::Usage` — and no `Done` (that is appended at stream end).
#[test]
fn usage_only_chunk_maps_to_usage_before_done() {
    let r = response(serde_json::json!({
        "id": "x",
        "created": 0,
        "model": "gpt-x",
        "object": "chat.completion.chunk",
        "choices": [],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "total_tokens": 120,
            "prompt_tokens_details": {"cached_tokens": 40}
        }
    }));

    let chunks = stream_response_to_chunks(&r, "gpt-x");
    assert!(
        !chunks.iter().any(|c| matches!(c, Ok(StreamChunk::Done))),
        "usage chunk must not carry a terminal Done"
    );
    match chunks.as_slice() {
        [
            Ok(StreamChunk::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            }),
        ] => {
            assert_eq!(*input_tokens, 100);
            assert_eq!(*output_tokens, 20);
            assert_eq!(*cache_read_tokens, 40);
            assert_eq!(*cache_write_tokens, 0);
        }
        other => panic!("expected a single Usage chunk, got {other:?}"),
    }
}

/// Cache split is preserved from `prompt_tokens_details.cached_tokens`.
#[test]
fn usage_to_chunk_reads_cache_split() {
    let usage: CompletionUsage = serde_json::from_value(serde_json::json!({
        "prompt_tokens": 500,
        "completion_tokens": 60,
        "total_tokens": 560,
        "prompt_tokens_details": {"cached_tokens": 320}
    }))
    .unwrap();
    assert!(matches!(
        usage_to_chunk(&usage),
        StreamChunk::Usage {
            input_tokens: 500,
            output_tokens: 60,
            cache_read_tokens: 320,
            cache_write_tokens: 0,
        }
    ));
}

/// Providers that omit usage (and cache details) still map cleanly: the
/// finish chunk yields text + model info, and no Usage is fabricated.
#[test]
fn response_without_usage_omits_usage_chunk() {
    let r = response(serde_json::json!({
        "id": "x",
        "created": 0,
        "model": "gpt-x",
        "object": "chat.completion.chunk",
        "choices": [
            {"index": 0, "delta": {"content": "done"}, "finish_reason": "stop"}
        ]
    }));
    let kinds: Vec<&str> = stream_response_to_chunks(&r, "gpt-x")
        .iter()
        .map(|c| match c {
            Ok(StreamChunk::Text(_)) => "text",
            Ok(StreamChunk::ModelInfo(_)) => "model",
            Ok(StreamChunk::Usage { .. }) => "usage",
            Ok(StreamChunk::Done) => "done",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["text", "model"]);
}

/// A usage chunk without a cache breakdown reports zero cache reads.
#[test]
fn usage_to_chunk_without_cache_details_is_zero() {
    let usage: CompletionUsage = serde_json::from_value(serde_json::json!({
        "prompt_tokens": 10,
        "completion_tokens": 3,
        "total_tokens": 13
    }))
    .unwrap();
    assert!(matches!(
        usage_to_chunk(&usage),
        StreamChunk::Usage {
            input_tokens: 10,
            output_tokens: 3,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    ));
}

// ─── Streaming terminal semantics (finish_reason / truncation) ──────

/// Label a normalized chunk for order assertions.
fn kind(chunk: &Result<StreamChunk>) -> &'static str {
    match chunk {
        Ok(StreamChunk::Text(_)) => "text",
        Ok(StreamChunk::Reasoning(_)) => "reasoning",
        Ok(StreamChunk::ToolCallDelta { .. }) => "tool",
        Ok(StreamChunk::ModelInfo(_)) => "model",
        Ok(StreamChunk::Usage { .. }) => "usage",
        Ok(StreamChunk::Billing { .. }) => "billing",
        Ok(StreamChunk::Done) => "done",
        Err(_) => "error",
    }
}

/// A normal stream — content, a `finish_reason` chunk, then a trailing
/// usage-only chunk, then EOF — must emit `Done` exactly once, after the
/// usage chunk, and never strand usage behind an early `Done`.
#[tokio::test]
async fn stream_emits_single_done_after_usage_on_real_finish() {
    use async_openai::error::OpenAIError;

    let content = response(serde_json::json!({
        "id": "x", "created": 0, "model": "gpt-x", "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"content": "hi"}, "finish_reason": null}]
    }));
    let finish = response(serde_json::json!({
        "id": "x", "created": 0, "model": "gpt-x", "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
    }));
    let usage = response(serde_json::json!({
        "id": "x", "created": 0, "model": "gpt-x", "object": "chat.completion.chunk",
        "choices": [],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    }));

    let inner = tokio_stream::iter(vec![Ok::<_, OpenAIError>(content), Ok(finish), Ok(usage)]);
    let chunks: Vec<Result<StreamChunk>> =
        finalize_openai_stream(inner, String::new()).collect().await;
    let kinds: Vec<&str> = chunks.iter().map(kind).collect();

    // Text, then model info on the finish chunk, then usage, then Done last.
    assert_eq!(kinds, vec!["text", "model", "usage", "done"]);
    assert_eq!(
        chunks
            .iter()
            .filter(|c| matches!(c, Ok(StreamChunk::Done)))
            .count(),
        1,
        "Done must be emitted exactly once"
    );
    // Usage precedes the single terminal Done.
    let usage_idx = kinds.iter().position(|k| *k == "usage").unwrap();
    let done_idx = kinds.iter().position(|k| *k == "done").unwrap();
    assert!(usage_idx < done_idx, "usage must arrive before Done");
    assert!(!kinds.contains(&"error"), "a clean finish must not error");
}

/// Premature EOF — the upstream stream ends after partial content but before
/// any `finish_reason` — must surface a typed transport error and must NOT
/// synthesize a terminal `Done`, so the session resolves to `Failed` while
/// preserving the partial text.
#[tokio::test]
async fn stream_premature_eof_yields_error_not_synthetic_done() {
    use async_openai::error::OpenAIError;

    let partial = response(serde_json::json!({
        "id": "x", "created": 0, "model": "gpt-x", "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {"content": "partial"}, "finish_reason": null}]
    }));
    // Stream ends here — no finish chunk, no `[DONE]`.
    let inner = tokio_stream::iter(vec![Ok::<_, OpenAIError>(partial)]);
    let chunks: Vec<Result<StreamChunk>> =
        finalize_openai_stream(inner, String::new()).collect().await;

    assert!(
        chunks
            .iter()
            .any(|c| matches!(c, Ok(StreamChunk::Text(t)) if t == "partial")),
        "partial text must survive"
    );
    assert!(
        !chunks.iter().any(|c| matches!(c, Ok(StreamChunk::Done))),
        "must not synthesize Done on premature EOF"
    );
    assert!(
        matches!(chunks.last(), Some(Err(_))),
        "a truncated stream must end with a typed error"
    );
}

fn effort_test_request() -> ChatRequest {
    ChatRequest {
        model: "k3-256k".to_string(),
        messages: vec![],
        tools: vec![],
        temperature: None,
        max_tokens: None,
    }
}

#[test]
fn kimi_coding_body_gets_effort_and_temperature() {
    let provider = OpenRouterProvider::new_with_base("k", "https://api.kimi.com/coding/v1")
        .with_thinking_effort(Some("high".to_string()));
    // A caller-supplied temperature the endpoint would reject must be
    // overwritten, not merely defaulted when absent.
    let mut body = serde_json::json!({"model": "k3-256k", "messages": [], "temperature": 0.6});
    provider.mutate_body_for_endpoint(&mut body, &effort_test_request());
    assert_eq!(body["thinking"]["effort"], "high");
    assert_eq!(body["temperature"], 1.0);
}

#[test]
fn kimi_coding_without_effort_omits_thinking_field() {
    let provider = OpenRouterProvider::new_with_base("k", "https://api.kimi.com/coding/v1");
    let mut body = serde_json::json!({"model": "k3-256k", "messages": []});
    provider.mutate_body_for_endpoint(&mut body, &effort_test_request());
    assert!(body.get("thinking").is_none());
}

#[test]
fn moonshot_without_effort_keeps_k25_disable_workaround() {
    let provider = OpenRouterProvider::new_with_base("k", "https://api.moonshot.ai/v1");
    let mut body = serde_json::json!({"model": "kimi-k2.5", "messages": []});
    provider.mutate_body_for_endpoint(&mut body, &effort_test_request());
    assert_eq!(body["thinking"]["type"], "disabled");
}

#[test]
fn non_kimi_endpoints_are_untouched() {
    let provider = OpenRouterProvider::new_with_base("k", "https://openrouter.ai/api/v1")
        .with_thinking_effort(Some("max".to_string()));
    let mut body = serde_json::json!({"model": "gpt-x", "messages": []});
    provider.mutate_body_for_endpoint(&mut body, &effort_test_request());
    assert!(body.get("thinking").is_none());
    assert!(body.get("temperature").is_none());
}

/// Regression: GitHub scopes model entitlement to these headers, not just
/// attribution — omitting them on Copilot requests silently narrows the
/// account to a model set that excludes Gemini
/// (`model_not_available_for_integrator`, live-verified against a real
/// 400 response that named this exact header as the fix).
#[test]
fn attaches_copilot_headers_only_for_a_copilot_endpoint() {
    let copilot = OpenRouterProvider::new_with_base("tok", "https://api.githubcopilot.com");
    let req = copilot
        .attach_copilot_headers(
            copilot
                .http
                .post("https://api.githubcopilot.com/chat/completions"),
        )
        .build()
        .unwrap();
    let headers = req.headers();
    assert_eq!(
        headers.get("Copilot-Integration-Id").unwrap(),
        crate::copilot::COPILOT_INTEGRATION_ID
    );
    assert_eq!(
        headers.get("Editor-Version").unwrap(),
        crate::copilot::EDITOR_VERSION
    );
    assert_eq!(
        headers.get("Openai-Intent").unwrap(),
        crate::copilot::OPENAI_INTENT
    );
    assert_eq!(headers.get("x-initiator").unwrap(), "agent");

    let other = OpenRouterProvider::new_with_base("tok", "https://api.deepseek.com/v1");
    let req = other
        .attach_copilot_headers(
            other
                .http
                .post("https://api.deepseek.com/v1/chat/completions"),
        )
        .build()
        .unwrap();
    assert!(req.headers().get("Copilot-Integration-Id").is_none());
}

// ─── FR-017: vision content-part wiring ──────────────────────

fn user_message_with_image(path: &std::path::Path) -> ChatMessage {
    ChatMessage {
        role: MessageRole::User,
        content: "what is this?".to_string(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
        image_parts: vec![hq_core::types::ImageAttachment {
            path: path.to_path_buf(),
            mime_type: "image/png".to_string(),
        }],
    }
}

#[test]
fn vision_model_with_image_parts_builds_array_content() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"not a real png, just bytes").unwrap();

    let req = ChatRequest {
        model: "deepseek-flash".to_string(),
        messages: vec![user_message_with_image(tmp.path())],
        tools: vec![],
        temperature: None,
        max_tokens: None,
    };
    let built = build_request(&req).unwrap();
    let ChatCompletionRequestMessage::User(user_msg) = &built.messages[0] else {
        panic!("expected a User message");
    };
    match &user_msg.content {
        ChatCompletionRequestUserMessageContent::Array(parts) => {
            assert_eq!(parts.len(), 2, "expected one text part and one image part");
            assert!(matches!(
                parts[0],
                ChatCompletionRequestUserMessageContentPart::Text(_)
            ));
            assert!(matches!(
                parts[1],
                ChatCompletionRequestUserMessageContentPart::ImageUrl(_)
            ));
        }
        ChatCompletionRequestUserMessageContent::Text(_) => {
            panic!("expected Array content for a vision model with image_parts set")
        }
    }
}

#[test]
fn non_vision_model_with_image_parts_falls_back_to_text_only() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"bytes").unwrap();

    let req = ChatRequest {
        model: "some-text-only-model".to_string(),
        messages: vec![user_message_with_image(tmp.path())],
        tools: vec![],
        temperature: None,
        max_tokens: None,
    };
    let built = build_request(&req).unwrap();
    let ChatCompletionRequestMessage::User(user_msg) = &built.messages[0] else {
        panic!("expected a User message");
    };
    assert!(
        matches!(
            user_msg.content,
            ChatCompletionRequestUserMessageContent::Text(_)
        ),
        "a model not on the vision allowlist must fall back to Text-only, not send an \
         Array payload the provider may reject"
    );
}

/// Regression guard: a message with no image attachments must still
/// serialize to the exact pre-FR-017 Text variant, for every model.
#[test]
fn text_only_message_is_unaffected_by_the_image_parts_field() {
    let req = ChatRequest {
        model: "deepseek-flash".to_string(),
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: "hello".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
            image_parts: vec![],
        }],
        tools: vec![],
        temperature: None,
        max_tokens: None,
    };
    let built = build_request(&req).unwrap();
    let ChatCompletionRequestMessage::User(user_msg) = &built.messages[0] else {
        panic!("expected a User message");
    };
    assert!(matches!(
        user_msg.content,
        ChatCompletionRequestUserMessageContent::Text(ref t) if t == "hello"
    ));
}

/// Diagnostic, not a regression test: exercises the REAL provider path
/// (build_request + serde_json::to_value + reqwest POST) against
/// DeepSeek's live API with a real image, timing it, to isolate whether
/// a live-reported "vision doesn't work" hang is in our request
/// construction or somewhere else in the stack.
/// Run: DEEPSEEK_API_KEY=sk-... VISION_TEST_IMAGE=/path/to.jpg cargo test
///   -p hq-llm -- --ignored --nocapture live_vision_request_against_real_deepseek
#[tokio::test]
#[ignore = "requires DEEPSEEK_API_KEY and VISION_TEST_IMAGE, hits the real network"]
async fn live_vision_request_against_real_deepseek() {
    let key = std::env::var("DEEPSEEK_API_KEY").expect("set DEEPSEEK_API_KEY");
    let image_path = std::env::var("VISION_TEST_IMAGE").expect("set VISION_TEST_IMAGE");

    let provider = OpenRouterProvider::new_with_base(&key, "https://api.deepseek.com/v1");
    let req = ChatRequest {
        model: "deepseek-flash".to_string(),
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: "Describe this image in one sentence.".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
            image_parts: vec![hq_core::types::ImageAttachment {
                path: std::path::PathBuf::from(&image_path),
                mime_type: "image/jpeg".to_string(),
            }],
        }],
        tools: vec![],
        temperature: None,
        max_tokens: None,
    };

    // Print exactly what build_request produces, same as the real chat() path.
    let oai_request = build_request(&req).unwrap();
    let body = serde_json::to_value(&oai_request).unwrap();
    let body_str = serde_json::to_string(&body).unwrap();
    eprintln!("request body length: {}", body_str.len());
    eprintln!(
        "request body head: {}",
        &body_str[..body_str.len().min(500)]
    );

    let start = std::time::Instant::now();
    let result = provider.chat(&req).await;
    eprintln!("elapsed: {:?}", start.elapsed());
    match result {
        Ok(resp) => eprintln!("SUCCESS: {}", resp.message.content),
        Err(e) => eprintln!("ERROR: {e:#}"),
    }
}

// ─── Provider-billed cost on streams ────────────────────────────────

const BILLED_USD: f64 = 0.00123;
const REASONING_TOKENS: u32 = 7;

fn sse(payloads: &[serde_json::Value]) -> String {
    let mut body: String = payloads.iter().map(|p| format!("data: {p}\n\n")).collect();
    body.push_str("data: [DONE]\n\n");
    body
}

fn billed_stream_body() -> String {
    let chunk = |choices: serde_json::Value, extra: serde_json::Value| {
        let mut v = serde_json::json!({
            "id": "x", "created": 0, "model": "anthropic/claude-haiku-5.5",
            "object": "chat.completion.chunk", "choices": choices,
        });
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        v
    };
    sse(&[
        chunk(
            serde_json::json!([{"index": 0, "delta": {"content": "hi"}, "finish_reason": null}]),
            serde_json::json!({}),
        ),
        chunk(
            serde_json::json!([{"index": 0, "delta": {}, "finish_reason": "stop"}]),
            serde_json::json!({}),
        ),
        chunk(
            serde_json::json!([]),
            serde_json::json!({"usage": {
                "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
                "cost": BILLED_USD,
                "completion_tokens_details": {"reasoning_tokens": REASONING_TOKENS}
            }}),
        ),
    ])
}

/// OpenRouter reports what it charged in the final chunk's `usage.cost`; the typed client drops
/// it, so the raw stream hands it over as a `Billing` chunk right after `Usage`.
#[tokio::test]
async fn an_openrouter_stream_surfaces_the_billed_cost_and_reasoning_tokens() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/proxy/openrouter.ai/api/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(billed_stream_body()),
        )
        .mount(&server)
        .await;
    let base = format!("{}/proxy/openrouter.ai/api/v1", server.uri());
    let provider = OpenRouterProvider::new_with_base("k", &base);
    let request = ChatRequest {
        model: "anthropic/claude-haiku-5.5".into(),
        messages: vec![ChatMessage {
            image_parts: Vec::new(),
            role: hq_core::types::MessageRole::User,
            content: "hi".into(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }],
        ..Default::default()
    };

    let chunks: Vec<Result<StreamChunk>> = provider
        .chat_stream(&request)
        .await
        .unwrap()
        .collect()
        .await;
    let kinds: Vec<&str> = chunks.iter().map(kind).collect();
    assert_eq!(kinds, vec!["text", "model", "usage", "billing", "done"]);
    let billing = chunks.iter().find_map(|c| match c {
        Ok(StreamChunk::Billing {
            cost_usd,
            reasoning_tokens,
        }) => Some((*cost_usd, *reasoning_tokens)),
        _ => None,
    });
    assert_eq!(billing, Some((Some(BILLED_USD), REASONING_TOKENS)));
}

/// A stream with no cost and no reasoning tokens sends no `Billing` chunk at all.
#[tokio::test]
async fn a_plain_usage_chunk_adds_no_billing_chunk() {
    use async_openai::error::OpenAIError;

    let usage = response(serde_json::json!({
        "id": "x", "created": 0, "model": "gpt-x", "object": "chat.completion.chunk",
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
    }));
    let inner = tokio_stream::iter(vec![Ok::<_, OpenAIError>(usage)]);
    let chunks: Vec<Result<StreamChunk>> =
        finalize_openai_stream(inner, String::new()).collect().await;
    assert!(
        chunks
            .iter()
            .all(|c| !matches!(c, Ok(StreamChunk::Billing { .. })))
    );
}

#[tokio::test]
async fn a_buffered_response_carries_the_billed_cost_and_reasoning_tokens() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/proxy/openrouter.ai/api/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "x", "model": "anthropic/claude-haiku-5.5",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "pong"}}],
            "usage": {
                "prompt_tokens": 100, "completion_tokens": 20, "cost": BILLED_USD,
                "completion_tokens_details": {"reasoning_tokens": REASONING_TOKENS}
            }
        })))
        .mount(&server)
        .await;
    let provider =
        OpenRouterProvider::new_with_base("k", &format!("{}/proxy/openrouter.ai/api/v1", server.uri()));
    let request = ChatRequest {
        model: "anthropic/claude-haiku-5.5".into(),
        ..Default::default()
    };

    let resp = provider.chat(&request).await.unwrap();

    assert_eq!(resp.provider_cost_usd, Some(BILLED_USD));
    assert_eq!(resp.reasoning_tokens, REASONING_TOKENS);
}

/// A transient failure to open the stream is retried once, as the typed client's first-item error
/// used to be.
#[tokio::test]
async fn an_openrouter_stream_that_fails_to_open_is_retried_once() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let route = "/proxy/openrouter.ai/api/v1/chat/completions";
    Mock::given(method("POST"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(route))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(billed_stream_body()),
        )
        .mount(&server)
        .await;
    let base = format!("{}/proxy/openrouter.ai/api/v1", server.uri());
    let provider = OpenRouterProvider::new_with_base("k", &base);
    let request = ChatRequest {
        model: "anthropic/claude-haiku-5.5".into(),
        ..Default::default()
    };

    let chunks: Vec<Result<StreamChunk>> = provider
        .chat_stream(&request)
        .await
        .unwrap()
        .collect()
        .await;

    assert!(chunks.iter().any(|c| matches!(c, Ok(StreamChunk::Done))));
}

/// Another provider's `usage.cost` may not be dollars, so only OpenRouter's is trusted.
#[tokio::test]
async fn a_non_openrouter_cost_field_is_ignored() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "x", "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "pong"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "cost": 99}
        })))
        .mount(&server)
        .await;
    let provider = OpenRouterProvider::new_with_base("k", &server.uri());
    let request = ChatRequest {
        model: "m".into(),
        ..Default::default()
    };

    assert_eq!(provider.chat(&request).await.unwrap().provider_cost_usd, None);
}

/// The quota a provider reports on an ordinary response is remembered, with no extra request.
#[tokio::test]
async fn a_completion_remembers_the_quota_headers_it_came_with() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-ratelimit-remaining-requests", "41")
                .insert_header("x-ratelimit-limit-requests", "50")
                .insert_header("x-ratelimit-remaining-tokens", "9000")
                .set_body_json(serde_json::json!({
                    "id": "x", "model": "m",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "pong"}}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1}
                })),
        )
        .mount(&server)
        .await;
    let provider = OpenRouterProvider::new_with_base("k", &server.uri());
    let request = ChatRequest {
        model: "m".into(),
        ..Default::default()
    };

    provider.chat(&request).await.unwrap();

    let seen = crate::ratelimit::latest_for(&server.uri()).expect("a reading");
    assert_eq!((seen.requests_remaining, seen.requests_limit, seen.tokens_remaining), (Some(41), Some(50), Some(9000)));
}
