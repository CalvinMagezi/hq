//! Request building, error classification, and response parsing.

use anyhow::{Context, Result};
use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestMessageContentPartText,
    ChatCompletionRequestSystemMessage, ChatCompletionRequestToolMessage,
    ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
    ChatCompletionRequestUserMessageContentPart, ChatCompletionTool, ChatCompletionToolType,
    CreateChatCompletionRequest, FunctionObject, ImageUrl,
};
use hq_core::types::{ChatMessage, MessageRole, ToolCall};

use crate::provider::{ChatRequest, LlmError, mentions_context_overflow, truncate_message};

pub fn build_request(req: &ChatRequest) -> Result<CreateChatCompletionRequest> {
    let messages: Vec<ChatCompletionRequestMessage> = req
        .messages
        .iter()
        .map(|m| match m.role {
            MessageRole::System => {
                ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                    content: async_openai::types::ChatCompletionRequestSystemMessageContent::Text(
                        m.content.clone(),
                    ),
                    name: None,
                })
            }
            MessageRole::User => {
                // FR-017: forward attached images only when the routed model
                // is known vision-capable. Otherwise (or when there are no
                // attachments) this is byte-identical to the pre-FR-017
                // Text-only path, so plain-text requests are unaffected.
                let content = if !m.image_parts.is_empty()
                    && crate::models::model_supports_vision(&req.model)
                {
                    let mut parts = vec![ChatCompletionRequestUserMessageContentPart::Text(
                        ChatCompletionRequestMessageContentPartText {
                            text: m.content.clone(),
                        },
                    )];
                    for img in &m.image_parts {
                        match img.to_data_url() {
                            Ok(url) => {
                                parts.push(ChatCompletionRequestUserMessageContentPart::ImageUrl(
                                    ChatCompletionRequestMessageContentPartImage {
                                        image_url: ImageUrl { url, detail: None },
                                    },
                                ))
                            }
                            Err(e) => tracing::warn!(
                                error = %e,
                                path = %img.path.display(),
                                "openai_compat: failed to read image attachment, skipping"
                            ),
                        }
                    }
                    ChatCompletionRequestUserMessageContent::Array(parts)
                } else {
                    ChatCompletionRequestUserMessageContent::Text(m.content.clone())
                };
                ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                    content,
                    name: None,
                })
            }
            MessageRole::Assistant => {
                ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                    content: if m.content.is_empty() && !m.tool_calls.is_empty() {
                        None
                    } else {
                        Some(
                            async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(
                                m.content.clone(),
                            ),
                        )
                    },
                    name: None,
                    tool_calls: if m.tool_calls.is_empty() {
                        None
                    } else {
                        Some(
                            m.tool_calls
                                .iter()
                                .map(|tc| async_openai::types::ChatCompletionMessageToolCall {
                                    id: tc.id.clone(),
                                    r#type: ChatCompletionToolType::Function,
                                    function: async_openai::types::FunctionCall {
                                        name: tc.name.clone(),
                                        arguments: tc.arguments.to_string(),
                                    },
                                })
                                .collect(),
                        )
                    },
                    ..Default::default()
                })
            }
            MessageRole::Tool => {
                ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                    content: async_openai::types::ChatCompletionRequestToolMessageContent::Text(
                        m.content.clone(),
                    ),
                    tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                })
            }
        })
        .collect();

    let tools: Option<Vec<ChatCompletionTool>> = if req.tools.is_empty() {
        None
    } else {
        Some(
            req.tools
                .iter()
                .map(|t| ChatCompletionTool {
                    r#type: ChatCompletionToolType::Function,
                    function: FunctionObject {
                        name: t.name.clone(),
                        description: Some(t.description.clone()),
                        parameters: Some(t.parameters.clone()),
                        strict: None,
                    },
                })
                .collect(),
        )
    };

    Ok(CreateChatCompletionRequest {
        model: req.model.clone(),
        messages,
        tools,
        temperature: req.temperature,
        max_completion_tokens: req.max_tokens,
        stream: Some(false),
        ..Default::default()
    })
}

/// Classify an async-openai error into a structured LlmError.
///
/// Uses pattern matching on the error enum where possible, falling back to
/// message content matching only for the API error code field and reqwest errors.
pub fn classify_openai_error(e: &async_openai::error::OpenAIError) -> LlmError {
    use async_openai::error::OpenAIError;

    match e {
        // Structured API errors: use the code field for classification
        OpenAIError::ApiError(api_err) => {
            let code = api_err.code.as_deref().unwrap_or("");
            let msg = &api_err.message;

            // Rate limit / overloaded
            if code == "429"
                || code == "rate_limit_exceeded"
                || msg.contains("rate limit")
                || msg.contains("Rate limit")
            {
                return LlmError::RateLimit { retry_after: None };
            }
            if code == "529" || msg.contains("overloaded") {
                return LlmError::Overloaded;
            }

            // Auth / payment
            if code == "401"
                || code == "402"
                || code == "403"
                || code == "invalid_api_key"
                || msg.contains("Unauthorized")
                || msg.contains("Payment Required")
                || msg.contains("insufficient credits")
            {
                return LlmError::Auth {
                    status: code.parse().unwrap_or(401),
                    message: truncate_message(msg).to_string(),
                };
            }

            // Context overflow, checked after rate limit so a rate-limit
            // message that mentions tokens still fails over.
            if code == "context_length_exceeded" || mentions_context_overflow(msg) {
                return LlmError::ContextOverflow {
                    message: truncate_message(msg).to_string(),
                };
            }

            // Server errors (5xx codes)
            if let Ok(status) = code.parse::<u16>()
                && (500..600).contains(&status)
            {
                return LlmError::from_http(status, msg);
            }

            LlmError::Other(anyhow::anyhow!("{}", api_err))
        }

        // Network / HTTP transport errors
        OpenAIError::Reqwest(req_err) => {
            if req_err.is_timeout() {
                return LlmError::Network(format!("request timeout: {req_err}"));
            }
            if req_err.is_connect() {
                return LlmError::Network(format!("connection error: {req_err}"));
            }
            if let Some(status) = req_err.status() {
                let code = status.as_u16();
                if code == 429 {
                    return LlmError::RateLimit { retry_after: None };
                }
                if code == 402 {
                    return LlmError::Auth {
                        status: 402,
                        message: "Payment required: insufficient OpenRouter credits or model requires payment".to_string(),
                    };
                }
                if (500..600).contains(&code) {
                    return LlmError::ServerError {
                        status: code,
                        message: format!("{req_err}"),
                    };
                }
            }
            LlmError::Network(format!("{req_err}"))
        }

        // JSON deserialization errors — often caused by non-200 responses from
        // OpenRouter that async-openai tries to parse as a normal response.
        // Check the error message for status code hints.
        OpenAIError::JSONDeserialize(serde_err) => {
            let msg = format!("{serde_err}");
            if msg.contains("402") {
                return LlmError::Auth {
                    status: 402,
                    message: "Model requires payment or insufficient credits. Try a free model."
                        .to_string(),
                };
            }
            if msg.contains("missing field") {
                return LlmError::Other(anyhow::anyhow!(
                    "API returned an unexpected response body ({}). \
                     The provider likely returned an error body instead of a chat completion — \
                     check the provider's status page or try again.",
                    serde_err
                ));
            }
            LlmError::Other(anyhow::anyhow!("failed to parse API response: {serde_err}"))
        }

        // Stream errors
        OpenAIError::StreamError(msg) => LlmError::Network(msg.clone()),

        // Everything else
        _ => LlmError::Other(anyhow::anyhow!("{}", e)),
    }
}

/// Parse a response from any OpenRouter model, handling non-standard fields
/// like Qwen's `reasoning`, Kimi's `reasoning_details`, null `content`, etc.
pub(crate) fn parse_flexible_response(json: &serde_json::Value) -> Result<ChatMessage> {
    let choice = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .context("no choices in response")?;

    let msg = choice.get("message").context("no message in choice")?;

    // Content: may be a string, null, or missing (reasoning models)
    let content = msg
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    // DeepSeek reasoning scratchpad — preserved separately so it can be replayed
    // verbatim in subsequent tool-call turns (omitting it causes a 400 error).
    let reasoning_content: Option<String> = msg
        .get("reasoning_content")
        .and_then(|r| r.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // If content is empty but reasoning exists, surface it as the display content.
    // Different models use different field names: "reasoning", "reasoning_content"
    let content = if content.is_empty() {
        reasoning_content
            .as_deref()
            .or_else(|| msg.get("reasoning").and_then(|r| r.as_str()))
            .unwrap_or("")
            .to_string()
    } else {
        content
    };

    // Parse tool calls
    let tool_calls: Vec<ToolCall> = msg
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .map(|tcs| {
            tcs.iter()
                .filter_map(|tc| {
                    let id = tc.get("id")?.as_str()?.to_string();
                    let func = tc.get("function")?;
                    let name = func.get("name")?.as_str()?.to_string();
                    let args_str = func.get("arguments")?.as_str().unwrap_or("{}");
                    let arguments = serde_json::from_str(args_str)
                        .unwrap_or(serde_json::Value::String(args_str.to_string()));
                    Some(ToolCall {
                        id,
                        name,
                        arguments,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ChatMessage {
        image_parts: Vec::new(),
        role: MessageRole::Assistant,
        content,
        tool_calls,
        tool_call_id: None,
        reasoning_content,
    })
}

pub fn parse_assistant_message(choice: &async_openai::types::ChatChoice) -> Result<ChatMessage> {
    let msg = &choice.message;

    let tool_calls: Vec<ToolCall> = msg
        .tool_calls
        .as_ref()
        .map(|tcs| {
            tcs.iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::String(tc.function.arguments.clone())),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ChatMessage {
        image_parts: Vec::new(),
        role: MessageRole::Assistant,
        content: msg.content.clone().unwrap_or_default(),
        tool_calls,
        tool_call_id: None,
        reasoning_content: None,
    })
}
