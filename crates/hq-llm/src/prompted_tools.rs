//! Prompted tool-calling shim for models that lack native function-calling.
//!
//! Some local models (notably the Gemma4 family served by Ollama) silently
//! ignore the OpenAI `tools` parameter. This module wraps a `ChatRequest` so
//! the model still "calls" tools via strict XML tags in free-text output,
//! which we parse back into the normal `tool_calls` path used by the session
//! loop.
//!
//! ## Activation
//!
//! Call `needs_prompted_tools(model_id)` to decide whether to route through
//! the shim. `OllamaProvider` checks this at request time.
//!
//! ## Format
//!
//! The model is told to emit:
//!
//! ```text
//! <tool_call name="spawn_subagent"><args>{"prompt":"..."}</args></tool_call>
//! ```
//!
//! XML tags are chosen over pure JSON because Gemma-family models are more
//! reliable at tag boundaries and the tags survive streaming chunking.
//! Tool results are fed back in the next user turn as:
//!
//! ```text
//! <tool_result name="spawn_subagent">{output}</tool_result>
//! ```

use hq_core::types::{ChatMessage, MessageRole, ToolCall, ToolDefinition};

use crate::provider::ChatRequest;

/// Model-id prefixes that require the shim. Kept small and conservative:
/// adding a prefix here means "treat this model as lacking native tools".
const SHIM_PREFIXES: &[&str] = &["gemma", "gemma2", "gemma4", "phi"];

/// True if the model should be routed through the prompted-tools shim.
pub fn needs_prompted_tools(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    let id = id.strip_prefix("ollama/").unwrap_or(&id);
    SHIM_PREFIXES.iter().any(|p| id.starts_with(p))
}

/// Rewrite a `ChatRequest` so the tool catalog is embedded in the system
/// prompt instead of the `tools` field. Safe to call on any request — no-op
/// if `tools` is empty.
pub fn inject_tool_prompt(request: &mut ChatRequest) {
    if request.tools.is_empty() {
        return;
    }
    let tool_block = render_tool_block(&request.tools);
    request.tools.clear();

    // Find or insert the leading system message.
    if let Some(msg) = request
        .messages
        .iter_mut()
        .find(|m| matches!(m.role, MessageRole::System))
    {
        msg.content = format!("{}\n\n{}", msg.content, tool_block);
    } else {
        request.messages.insert(
            0,
            ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::System,
                content: tool_block,
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: None,
            },
        );
    }

    flatten_tool_results(&mut request.messages);
}

/// Replace `role: Tool` messages with `<tool_result>` blocks folded into
/// the next user turn. Gemma doesn't understand the OpenAI tool role, so
/// we inline the outputs as text and drop the tool-role turns.
fn flatten_tool_results(messages: &mut Vec<ChatMessage>) {
    let mut out = Vec::with_capacity(messages.len());
    let mut pending: Vec<String> = Vec::new();

    for msg in messages.drain(..) {
        match msg.role {
            MessageRole::Tool => {
                let tag = format!(
                    "<tool_result id=\"{}\">{}</tool_result>",
                    msg.tool_call_id.as_deref().unwrap_or(""),
                    msg.content
                );
                pending.push(tag);
            }
            MessageRole::Assistant => {
                // Carry assistant text but strip tool_calls (we re-emit them
                // as free text in the same format the model produced). If
                // there are tool_calls but no content, synthesize the XML so
                // the assistant's own prior outputs look consistent in the
                // transcript.
                let mut content = msg.content;
                for tc in &msg.tool_calls {
                    let rendered = format!(
                        "<tool_call name=\"{}\"><args>{}</args></tool_call>",
                        tc.name,
                        serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string())
                    );
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&rendered);
                }
                out.push(ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::Assistant,
                    content,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                });
            }
            _ => {
                if !pending.is_empty() {
                    // Prepend accumulated tool results to this user turn.
                    let mut content = pending.join("\n");
                    if !msg.content.is_empty() {
                        content.push_str("\n\n");
                        content.push_str(&msg.content);
                    }
                    out.push(ChatMessage {
                        image_parts: Vec::new(),
                        role: msg.role,
                        content,
                        tool_calls: msg.tool_calls,
                        tool_call_id: None,
                        reasoning_content: None,
                    });
                    pending.clear();
                } else {
                    out.push(msg);
                }
            }
        }
    }

    if !pending.is_empty() {
        // Trailing tool results with no following user turn — append as
        // a synthetic user message so the model sees them.
        out.push(ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: pending.join("\n"),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        });
    }

    *messages = out;
}

fn render_tool_block(tools: &[ToolDefinition]) -> String {
    let mut out = String::from(
        "# Tool Calling\n\n\
         You have access to tools. To call a tool, emit EXACTLY one line in \
         this format (no extra text on that line):\n\n\
         <tool_call name=\"TOOL_NAME\"><args>COMPACT_JSON_ARGS</args></tool_call>\n\n\
         - `COMPACT_JSON_ARGS` must be valid JSON on ONE line, no newlines inside.\n\
         - You may write regular text before the call to explain your reasoning, \
         but the call itself must be one line.\n\
         - Tool results come back as `<tool_result id=\"...\">{output}</tool_result>` \
         in the next turn; read them and continue.\n\
         - Do not invent tools. Only call tools from the list below.\n\n\
         ## Available Tools\n\n",
    );
    for t in tools {
        out.push_str(&format!("### {}\n\n{}\n\n", t.name, t.description));
        let schema = serde_json::to_string(&t.parameters).unwrap_or_else(|_| "{}".to_string());
        out.push_str("Parameters (JSON schema): `");
        out.push_str(&schema);
        out.push_str("`\n\n");
    }
    out
}

/// A tool call extracted from free-text model output.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedCall {
    pub name: String,
    pub arguments: String,
}

/// Parse all `<tool_call>` tags out of the assistant's content. Returns the
/// cleaned content (tags stripped) and the list of extracted calls.
pub fn extract_calls(content: &str) -> (String, Vec<ExtractedCall>) {
    let mut cleaned = String::with_capacity(content.len());
    let mut calls = Vec::new();
    let mut cursor = 0usize;

    while let Some(start) = content[cursor..].find("<tool_call") {
        let abs_start = cursor + start;
        cleaned.push_str(&content[cursor..abs_start]);

        // Find the end of the opening tag.
        let open_end = match content[abs_start..].find('>') {
            Some(idx) => abs_start + idx + 1,
            None => {
                cursor = abs_start;
                break;
            }
        };

        // Extract name attribute.
        let open_tag = &content[abs_start..open_end];
        let name = match extract_attr(open_tag, "name") {
            Some(n) => n,
            None => {
                // Malformed: skip the char and continue.
                cleaned.push_str("<tool_call");
                cursor = abs_start + "<tool_call".len();
                continue;
            }
        };

        // Find closing tag.
        let close_marker = "</tool_call>";
        let close_start = match content[open_end..].find(close_marker) {
            Some(idx) => open_end + idx,
            None => {
                // Unterminated — drop the rest as malformed.
                cursor = content.len();
                break;
            }
        };

        let inner = &content[open_end..close_start];
        let arguments = extract_args(inner).unwrap_or_else(|| inner.trim().to_string());

        calls.push(ExtractedCall { name, arguments });
        cursor = close_start + close_marker.len();
    }

    cleaned.push_str(&content[cursor..]);
    (cleaned.trim().to_string(), calls)
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_args(inner: &str) -> Option<String> {
    let open = inner.find("<args>")? + "<args>".len();
    let close = inner[open..].find("</args>")?;
    Some(inner[open..open + close].trim().to_string())
}

/// Monotonic counter used to produce unique tool-call ids across all turns
/// and sessions in the process. Using just `(turn_index, call_index)` is not
/// enough — a later turn starting again from index 0 would collide with a
/// stored tool-result correlation. A process-wide counter sidesteps that.
static PROMPTED_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Convert an assistant message's free-text content into native `tool_calls`.
/// Returns (cleaned_content, tool_calls). If no tags found, the content is
/// returned unchanged with an empty call list.
pub fn promote_calls(content: &str) -> (String, Vec<ToolCall>) {
    let (cleaned, extracted) = extract_calls(content);
    let calls = extracted
        .into_iter()
        .map(|c| {
            let args: serde_json::Value = serde_json::from_str(&c.arguments)
                .unwrap_or(serde_json::Value::String(c.arguments));
            let seq = PROMPTED_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            ToolCall {
                id: format!("prompted-{}", seq),
                name: c.name,
                arguments: args,
            }
        })
        .collect();
    (cleaned, calls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("The {} tool", name),
            parameters: json!({"type": "object"}),
        }
    }

    #[test]
    fn promote_returns_toolcall_name_and_args() {
        let (_cleaned, calls) =
            promote_calls("<tool_call name=\"ping\"><args>{\"k\":1}</args></tool_call>");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "ping");
        assert_eq!(calls[0].arguments, json!({"k": 1}));
    }

    #[test]
    fn shim_activates_for_gemma() {
        assert!(needs_prompted_tools("gemma4:e4b"));
        assert!(needs_prompted_tools("ollama/gemma4:e2b"));
        assert!(needs_prompted_tools("GEMMA-7B"));
        assert!(!needs_prompted_tools("anthropic/claude-sonnet-4"));
        assert!(!needs_prompted_tools("kimi-k2"));
    }

    #[test]
    fn inject_moves_tools_into_system_prompt() {
        let mut req = ChatRequest {
            model: "gemma4:e4b".into(),
            messages: vec![ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: "hello".into(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            }],
            tools: vec![tool("spawn_subagent"), tool("call_imagegen")],
            temperature: None,
            max_tokens: None,
        };
        inject_tool_prompt(&mut req);
        assert!(req.tools.is_empty());
        assert_eq!(req.messages.len(), 2);
        assert!(matches!(req.messages[0].role, MessageRole::System));
        assert!(req.messages[0].content.contains("spawn_subagent"));
        assert!(req.messages[0].content.contains("call_imagegen"));
        assert!(req.messages[0].content.contains("<tool_call"));
    }

    #[test]
    fn extract_simple_call() {
        let content = "Let me search.\n<tool_call name=\"grep\"><args>{\"q\":\"foo\"}</args></tool_call>\nDone.";
        let (cleaned, calls) = extract_calls(content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
        assert_eq!(calls[0].arguments, r#"{"q":"foo"}"#);
        assert!(cleaned.contains("Let me search"));
        assert!(cleaned.contains("Done."));
        assert!(!cleaned.contains("<tool_call"));
    }

    #[test]
    fn extract_multiple_calls() {
        let content = "<tool_call name=\"a\"><args>{}</args></tool_call>\n<tool_call name=\"b\"><args>{\"x\":1}</args></tool_call>";
        let (_cleaned, calls) = extract_calls(content);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "a");
        assert_eq!(calls[1].name, "b");
    }

    #[test]
    fn extract_unterminated_is_dropped() {
        let content = "<tool_call name=\"broken\"><args>{";
        let (_cleaned, calls) = extract_calls(content);
        assert!(calls.is_empty());
    }

    #[test]
    fn promoted_ids_are_unique_across_calls() {
        let (_, a) = promote_calls("<tool_call name=\"x\"><args>{}</args></tool_call>");
        let (_, b) = promote_calls("<tool_call name=\"x\"><args>{}</args></tool_call>");
        assert_ne!(
            a[0].id, b[0].id,
            "promoted ids must not collide across calls"
        );
    }

    #[test]
    fn tool_role_messages_become_result_tags() {
        let mut msgs = vec![
            ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Assistant,
                content: "calling".into(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            },
            ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Tool,
                content: "42".into(),
                tool_calls: vec![],
                tool_call_id: Some("call_7".into()),
                reasoning_content: None,
            },
            ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: "what next?".into(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            },
        ];
        flatten_tool_results(&mut msgs);
        assert_eq!(msgs.len(), 2);
        assert!(matches!(msgs[0].role, MessageRole::Assistant));
        assert!(matches!(msgs[1].role, MessageRole::User));
        assert!(msgs[1].content.contains("<tool_result"));
        assert!(msgs[1].content.contains("call_7"));
        assert!(msgs[1].content.contains("42"));
        assert!(msgs[1].content.contains("what next?"));
    }
}
