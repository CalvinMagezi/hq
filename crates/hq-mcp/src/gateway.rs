//! Two-tool gateway: `hq_discover` browses the registry and `hq_call` invokes a
//! tool by name, so the MCP token footprint stays near 1K whatever the registry size.

use hq_core::microcompact::{MICROCOMPACT_THRESHOLD, MicrocompactStrategy, microcompact};
use hq_core::tokens::count_tokens_fast;
use hq_tools::registry::ToolRegistry;
use rmcp::ErrorData;
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;

/// Parameters for the `hq_discover` tool.
#[derive(Debug, Default, Deserialize)]
struct DiscoverArgs {
    category: Option<String>,
    query: Option<String>,
}

/// Parameters for the `hq_call` tool.
#[derive(Debug, Deserialize)]
struct CallArgs {
    tool: String,
    args: Option<Value>,
}

/// Tool names reachable by a Spark-scoped connection: read and status only.
/// No writes, deletes, publish, dispatch, or agent-to-agent messaging tools
/// are included. See
/// the Spark integration design notes.
pub const SPARK_READONLY_ALLOWLIST: &[&str] = &[
    "vault_search",
    "vault_find",
    "vault_read",
    "vault_read_section",
    "vault_outline",
    "vault_list",
    "vault_backlinks",
    "vault_links",
    "vault_tags",
    "vault_find_similar",
    "vault_context",
    "memory_entity_graph",
    "session_search",
    "harness_session_list",
    "harness_session_logs",
    "harness_session_status",
];

/// Tool names reachable by a handoff-scoped connection (`AGENTHQ_HANDOFF_API_KEY`):
/// the read tools plus filing and updating tasks and starting coding-agent
/// sessions. Starting a claude-code session runs it with permissions skipped,
/// so this key is code-execution equivalent on every configured host; bound it
/// with `herdr.handoff_cwd_allow`. It cannot send to or read the output of a
/// session (`harness_session_send`, `harness_session_logs`): the registry does
/// not record who created a session, so those would reach sessions this key
/// never started. No deletes, no vault writes, no session
/// stop/resume/link/goal changes, no config, no tools that reach other
/// services. It can ask HQ's chat agent a question (`hq_ask`) but only in
/// read-only mode, read back only the asks it made, and continue only threads those asks
/// started. The reply it gets is built without the session-log, Herdr and file-reading tools (see
/// `hq_tools::ask::HANDOFF_ASK_DENIED_PREFIXES`). An exact list, so a tool added to the registry later is denied
/// until someone adds it here.
pub const HANDOFF_ALLOWLIST: &[&str] = &[
    "vault_search",
    "vault_find",
    "vault_read",
    "vault_read_section",
    "vault_outline",
    "vault_list",
    "vault_backlinks",
    "vault_links",
    "vault_tags",
    "vault_find_similar",
    "vault_context",
    "memory_entity_graph",
    "session_search",
    "harness_session_list",
    "harness_session_status",
    "harness_session_wait",
    "task_list",
    "task_get",
    "task_comment_list",
    "task_create",
    "task_update",
    "task_comment_add",
    "harness_session_spawn",
    "harness_session_handoff",
    "hq_ask",
    "hq_ask_result",
];

/// What a launched agent may call with its own session token: the task thread
/// it works on, and nothing that reads the wider vault or changes settings.
/// An exact list, so a tool added later is denied until someone adds it here.
pub const SESSION_ALLOWLIST: &[&str] = &[
    "task_get",
    "task_list",
    "task_comment_list",
    "task_comment_add",
    "harness_session_status",
    "agent_message_send",
    "agent_delegate",
];

/// Build the two MCP `Tool` definitions for the gateway.
///
/// Accepts the registry so the discover tool description includes the real category list.
pub fn create_gateway_tools(registry: &ToolRegistry) -> Vec<Tool> {
    create_gateway_tools_scoped(registry, None)
}

/// Like [`create_gateway_tools`], but a scoped caller (`allowed` is `Some`) is
/// told only about the categories that hold a tool it may use, so the tool
/// description does not enumerate the rest of the catalog.
pub fn create_gateway_tools_scoped(
    registry: &ToolRegistry,
    allowed: Option<&[&str]>,
) -> Vec<Tool> {
    let categories = match allowed {
        None => registry.categories(),
        Some(allowed) => {
            let mut cats: Vec<String> = registry
                .discover(None, None)
                .into_iter()
                .filter(|t| allowed.contains(&t.name.as_str()))
                .map(|t| t.category)
                .collect();
            cats.sort();
            cats.dedup();
            cats
        }
    }
    .join(", ");

    let discover_schema = json!({
        "type": "object",
        "properties": {
            "category": {
                "type": "string",
                "description": format!("Filter by category: {categories}")
            },
            "query": {
                "type": "string",
                "description": "Free-text search across tool names and descriptions"
            }
        },
        "required": []
    });

    let call_schema = json!({
        "type": "object",
        "properties": {
            "tool": {
                "type": "string",
                "description": "Name of the tool to call (from hq_discover results)"
            },
            "args": {
                "type": "object",
                "description": "Arguments to pass to the tool (see tool's parameter schema)"
            }
        },
        "required": ["tool"]
    });

    vec![
        Tool::new(
            "hq_discover",
            "Discover available Agent-HQ tools. Returns names, descriptions, categories, and parameter schemas. Use category and/or query to filter.",
            rmcp::model::object(discover_schema),
        ),
        Tool::new(
            "hq_call",
            "Call any Agent-HQ tool by name with arguments. Use hq_discover first to find available tools and their parameter schemas.",
            rmcp::model::object(call_schema),
        ),
    ]
}

/// Handle the `hq_discover` tool call.
pub(crate) fn handle_discover(
    registry: &ToolRegistry,
    arguments: Option<&serde_json::Map<String, Value>>,
    allowed: Option<&[&str]>,
) -> Result<CallToolResult, ErrorData> {
    let args: DiscoverArgs = match arguments {
        Some(obj) => serde_json::from_value(Value::Object(obj.clone()))
            .map_err(|e| ErrorData::invalid_params(format!("invalid discover args: {e}"), None))?,
        None => DiscoverArgs::default(),
    };

    let mut results = registry.discover(args.category.as_deref(), args.query.as_deref());
    if let Some(allowed) = allowed {
        results.retain(|r| allowed.contains(&r.name.as_str()));
    }

    let json_str = serde_json::to_string_pretty(&results).unwrap_or_else(|_| "[]".to_string());

    Ok(CallToolResult::success(vec![ContentBlock::text(json_str)]))
}

/// Server instructions sent on `initialize`: how to use the two gateway tools
/// plus the tool catalog, so a client knows the tools without calling hq_discover.
pub fn server_instructions(registry: &ToolRegistry) -> String {
    let categories = registry.categories();
    let mut instructions = String::with_capacity(8192);
    instructions.push_str(&format!(
        "Agent-HQ: Local-first AI agent hub with {} tools across {} categories.\n\n",
        registry.len(),
        categories.len()
    ));
    instructions.push_str(
        "## How to use\n\
         - `hq_call(tool, args)` invokes any tool directly by name\n\
         - `hq_discover(category?, query?)` browses tools with filtering\n\n\
         Prefer `hq_call` when you know which tool to use. Use `hq_discover` to explore.\n\n",
    );
    // The full catalog lives only here: hq_call reaches every tool and no tool
    // schemas ship over MCP. Hints cost about a third of full descriptions.
    instructions.push_str(&registry.catalog_block());
    instructions.push('\n');
    instructions.push_str(&registry.behavioral_block(180));
    instructions
}

/// Route a gateway `tools/call` by name; the stdio server and the HTTP transport
/// both call this so the two cannot drift.
pub async fn dispatch(
    registry: &ToolRegistry,
    name: &str,
    arguments: Option<&serde_json::Map<String, Value>>,
    db: &hq_db::Database,
    allowed: Option<&[&str]>,
) -> Result<CallToolResult, ErrorData> {
    dispatch_as(registry, name, arguments, db, allowed, None).await
}

/// Like `dispatch`, for a caller that proved it is the launched session
/// `caller_session`. Tools see that id and nothing a caller wrote in its place.
pub async fn dispatch_as(
    registry: &ToolRegistry,
    name: &str,
    arguments: Option<&serde_json::Map<String, Value>>,
    db: &hq_db::Database,
    allowed: Option<&[&str]>,
    caller_session: Option<&str>,
) -> Result<CallToolResult, ErrorData> {
    let result = match name {
        "hq_discover" => handle_discover(registry, arguments, allowed),
        "hq_call" => handle_call_as(registry, arguments, db, allowed, caller_session).await,
        other => Err(ErrorData::invalid_params(
            format!("unknown tool: {other}. Use hq_discover or hq_call."),
            None,
        )),
    };
    result.map(without_result_type)
}

/// `resultType` is a 2026-07-28 field; hq-web serializes this result as-is for 2024-11-05 clients.
fn without_result_type(mut result: CallToolResult) -> CallToolResult {
    result.result_type = None;
    result
}

/// Handle the `hq_call` tool call for a caller with no attested session.
#[cfg(test)]
pub(crate) async fn handle_call(
    registry: &ToolRegistry,
    arguments: Option<&serde_json::Map<String, Value>>,
    db: &hq_db::Database,
    allowed: Option<&[&str]>,
) -> Result<CallToolResult, ErrorData> {
    handle_call_as(registry, arguments, db, allowed, None).await
}

async fn handle_call_as(
    registry: &ToolRegistry,
    arguments: Option<&serde_json::Map<String, Value>>,
    db: &hq_db::Database,
    allowed: Option<&[&str]>,
    caller_session: Option<&str>,
) -> Result<CallToolResult, ErrorData> {
    let session_id = format!("mcp-{}", std::process::id());

    let Some(obj) = arguments else {
        return Err(ErrorData::invalid_params(
            "hq_call requires arguments",
            None,
        ));
    };
    let args: CallArgs = serde_json::from_value(Value::Object(obj.clone()))
        .map_err(|e| ErrorData::invalid_params(format!("invalid call args: {e}"), None))?;

    if allowed.is_some_and(|allowed| !allowed.contains(&args.tool.as_str())) {
        return Err(ErrorData::invalid_params(
            format!("tool not permitted for this connection: {}", args.tool),
            None,
        ));
    }

    let tool = registry
        .get(&args.tool)
        .ok_or_else(|| ErrorData::invalid_params(format!("unknown tool: {}", args.tool), None))?;

    let mut call_args = args.args.unwrap_or(json!({}));
    if call_args.is_null() {
        call_args = json!({});
    }
    if MARKED_DENIED_TOOLS.contains(&args.tool.as_str()) && hq_tools::harness_session::spawned_session(&call_args).is_some() {
        return Err(ErrorData::invalid_params(
            format!("{}: {}", args.tool, hq_tools::harness_session::SPAWNED_REFUSAL),
            None,
        ));
    }
    mark_scope(&mut call_args, allowed);
    attest_caller(&mut call_args, caller_session);
    let call_result = hq_tools::registry::validate_and_execute(tool, call_args).await;

    // Telemetry is best-effort and must not fail the call. This layer never sees
    // the caller identity the HTTP transport resolves, hence "mcp-external".
    let error_msg = call_result.as_ref().err().map(|e| e.to_string());
    let _ = db.with_conn(|conn| {
        if let Err(e) = hq_db::tool_usage::record_tool_call(
            conn,
            &args.tool,
            "mcp-external",
            &session_id,
            error_msg.is_none(),
            error_msg.as_deref(),
        ) {
            debug!(tool = args.tool, error = %e, "gateway: failed to record tool_usage row");
        }
        Ok(())
    });

    match call_result {
        Ok(result) => {
            let text = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
            Ok(CallToolResult::success(vec![ContentBlock::text(
                compact_output(text, &args.tool),
            )]))
        }
        Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
            "tool error: {e}"
        ))])),
    }
}

/// Tools a caller inside an HQ-spawned session may not use: they rewrite HQ's own settings.
const MARKED_DENIED_TOOLS: &[&str] = &["config_manage"];

/// The longest session id a marker carries; anything else is cut.
const MAX_SPAWNED_ID_CHARS: usize = 64;

/// `arguments` of an `hq_call` with the caller marked as running inside the HQ-spawned
/// session `session_id`, which tools read to refuse starting more sessions. A marker only
/// ever adds restrictions, so a caller that sets or omits it itself gains nothing. Other
/// tools' arguments pass through unchanged.
pub fn mark_spawned_session(
    arguments: Option<&serde_json::Map<String, Value>>,
    session_id: &str,
) -> Option<serde_json::Map<String, Value>> {
    let mut marked = arguments?.clone();
    if marked.get("tool").is_none() {
        return Some(marked);
    }
    let id: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(MAX_SPAWNED_ID_CHARS)
        .collect();
    let id = if id.is_empty() {
        "unknown".to_string()
    } else {
        id
    };
    let inner = marked.entry("args").or_insert_with(|| json!({}));
    if inner.is_null() {
        *inner = json!({});
    }
    if let Some(obj) = inner.as_object_mut() {
        obj.insert(
            hq_tools::harness_session::SPAWNED_SESSION_ARG.into(),
            json!(id),
        );
    }
    Some(marked)
}

/// The caller's session is whatever the gateway attested: a value in the call
/// arguments is dropped, and the verified id is set only for a connection that
/// proved its session.
fn attest_caller(args: &mut Value, caller_session: Option<&str>) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    obj.remove(hq_tools::harness_session::CALLER_SESSION_ARG);
    if let Some(id) = caller_session {
        obj.insert(
            hq_tools::harness_session::CALLER_SESSION_ARG.into(),
            json!(id),
        );
    }
}

/// Tools learn which key a call came in on only from this argument: any value
/// the caller sent is dropped, and it is set here for the handoff scope.
fn mark_scope(args: &mut Value, allowed: Option<&[&str]>) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    obj.remove(hq_tools::harness_session::HANDOFF_SCOPE_ARG);
    if allowed == Some(HANDOFF_ALLOWLIST) {
        obj.insert(
            hq_tools::harness_session::HANDOFF_SCOPE_ARG.into(),
            json!(true),
        );
    }
}

/// Large results are compressed so one call cannot flood the client's context window.
fn compact_output(text: String, tool: &str) -> String {
    let tokens = count_tokens_fast(&text);
    debug!(tool, result_tokens = tokens, "tool call completed");
    if tokens <= MICROCOMPACT_THRESHOLD {
        return text;
    }
    let mc = microcompact(&text);
    debug!(
        tool,
        original_tokens = mc.original_tokens,
        compacted_tokens = mc.compacted_tokens,
        strategy = ?mc.strategy,
        "microcompacted tool result"
    );
    // A cut JSON list reads as complete unless the caller is told up front.
    if mc.strategy != MicrocompactStrategy::MiddleOut {
        return mc.text;
    }
    format!(
        "[Result cut from {} to about {} tokens: the middle was removed. \
         Narrow the query or fetch items one at a time for the full data.]\n{}",
        mc.original_tokens, mc.compacted_tokens, mc.text
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_tools::registry::HqTool;

    struct DummyTool {
        name: &'static str,
    }

    #[async_trait::async_trait]
    impl HqTool for DummyTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "test tool"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!({"ok": true}))
        }
    }

    #[test]
    fn a_spawned_session_marker_lands_in_the_tool_args_and_is_cleaned() {
        let call = json!({"tool": "hq_ask", "args": {"question": "q"}});
        let map = call.as_object().unwrap();
        let marked = mark_spawned_session(Some(map), "hs-claude-code-1a2b/../x").unwrap();
        let marker = hq_tools::harness_session::SPAWNED_SESSION_ARG;
        assert_eq!(marked["args"][marker], "hs-claude-code-1a2bx");
        assert_eq!(marked["args"]["question"], "q");
        let blank = mark_spawned_session(Some(map), "///").unwrap();
        assert_eq!(blank["args"][marker], "unknown");
        let no_args = json!({"tool": "hq_discover"});
        let bare = mark_spawned_session(no_args.as_object(), "hs-1").unwrap();
        assert_eq!(bare["args"][marker], "hs-1");
        let discover = json!({"category": "x"});
        let untouched = mark_spawned_session(discover.as_object(), "hs-1").unwrap();
        assert!(
            untouched.get("args").is_none(),
            "hq_discover has no args to mark"
        );
    }

    #[test]
    fn handoff_scope_extends_spark_and_stays_clear_of_destructive_tools() {
        for name in SPARK_READONLY_ALLOWLIST {
            if *name == "harness_session_logs" {
                continue;
            }
            assert!(
                HANDOFF_ALLOWLIST.contains(name),
                "handoff lost spark's {name}"
            );
        }
        for name in [
            "task_create",
            "task_update",
            "task_comment_add",
            "harness_session_spawn",
            "harness_session_handoff",
        ] {
            assert!(
                HANDOFF_ALLOWLIST.contains(&name),
                "handoff should reach {name}"
            );
        }
        for name in [
            "task_delete",
            "harness_session_send",
            "harness_session_logs",
            "harness_session_stop",
            "harness_session_resume",
            "harness_session_link",
            "harness_session_goal",
            "harness_session_mode",
            "harness_session_attach",
            "herdr_send",
            "config_manage",
            "vault_write_note",
            "bash",
        ] {
            assert!(
                !HANDOFF_ALLOWLIST.contains(&name),
                "handoff must not reach {name}"
            );
        }
    }

    #[tokio::test]
    async fn asking_hq_is_open_to_the_handoff_key_and_closed_to_the_spark_key() {
        assert!(
            HANDOFF_ALLOWLIST.contains(&"hq_ask") && HANDOFF_ALLOWLIST.contains(&"hq_ask_result")
        );
        assert!(!SPARK_READONLY_ALLOWLIST.contains(&"hq_ask"));
        assert!(!SPARK_READONLY_ALLOWLIST.contains(&"hq_ask_result"));

        let mut registry = ToolRegistry::new();
        for name in ["hq_ask", "hq_ask_result"] {
            registry.register(Box::new(DummyTool { name }));
        }
        let db = hq_db::Database::open_memory().unwrap();
        for name in ["hq_ask", "hq_ask_result"] {
            let call = serde_json::json!({"tool": name, "args": {}});
            let map = call.as_object().unwrap().clone();
            let spark =
                handle_call(&registry, Some(&map), &db, Some(SPARK_READONLY_ALLOWLIST)).await;
            assert!(spark.is_err(), "spark must not reach {name}");
            let handoff = handle_call(&registry, Some(&map), &db, Some(HANDOFF_ALLOWLIST)).await;
            assert!(handoff.is_ok(), "handoff reaches {name}");
        }
    }

    #[test]
    fn only_the_handoff_key_gets_the_scope_marker_and_a_caller_cannot_forge_it() {
        let key = hq_tools::harness_session::HANDOFF_SCOPE_ARG;
        let mut args = serde_json::json!({ key: true, "cwd": "/x" });
        mark_scope(&mut args, None);
        assert!(args.get(key).is_none(), "owner calls carry no marker");
        let mut args = serde_json::json!({ key: true });
        mark_scope(&mut args, Some(SPARK_READONLY_ALLOWLIST));
        assert!(args.get(key).is_none(), "a forged marker is stripped");
        let mut args = serde_json::json!({});
        mark_scope(&mut args, Some(HANDOFF_ALLOWLIST));
        assert_eq!(args[key], true);
    }

    #[tokio::test]
    async fn a_marked_caller_cannot_use_the_config_tool_and_null_args_are_tolerated() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool { name: "config_manage" }));
        let db = hq_db::Database::open_memory().unwrap();
        let marker = hq_tools::harness_session::SPAWNED_SESSION_ARG;
        let marked = serde_json::json!({"tool": "config_manage", "args": {marker: "hs-1"}});
        assert!(handle_call(&registry, Some(marked.as_object().unwrap()), &db, None).await.is_err());
        let plain = serde_json::json!({"tool": "config_manage", "args": {}});
        assert!(handle_call(&registry, Some(plain.as_object().unwrap()), &db, None).await.is_ok());

        let null_args = serde_json::json!({"tool": "config_manage", "args": null});
        let marked = mark_spawned_session(null_args.as_object(), "hs-1").unwrap();
        assert_eq!(marked["args"][marker], "hs-1", "null args become an object, not a lost marker");
        assert!(handle_call(&registry, Some(&marked), &db, None).await.is_err());
    }

    #[tokio::test]
    async fn handoff_scope_refuses_a_tool_outside_it_and_hides_it_from_discovery() {
        let mut registry = ToolRegistry::new();
        for name in ["task_create", "task_delete"] {
            registry.register(Box::new(DummyTool { name }));
        }
        let db = hq_db::Database::open_memory().unwrap();
        let denied = serde_json::json!({"tool": "task_delete", "args": {}});
        let map = denied.as_object().unwrap().clone();
        assert!(
            handle_call(&registry, Some(&map), &db, Some(HANDOFF_ALLOWLIST))
                .await
                .is_err()
        );
        let allowed = serde_json::json!({"tool": "task_create", "args": {}});
        let map = allowed.as_object().unwrap().clone();
        assert!(
            handle_call(&registry, Some(&map), &db, Some(HANDOFF_ALLOWLIST))
                .await
                .is_ok()
        );

        let text = handle_discover(&registry, None, Some(HANDOFF_ALLOWLIST))
            .unwrap()
            .content[0]
            .as_text()
            .unwrap()
            .text
            .clone();
        assert!(text.contains("task_create") && !text.contains("task_delete"));
    }

    #[test]
    fn handle_discover_filters_to_allowlist() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "vault_search",
        }));
        registry.register(Box::new(DummyTool {
            name: "vault_write_note",
        }));

        let allowed = ["vault_search"];
        let result = handle_discover(&registry, None, Some(&allowed)).unwrap();
        let text = result.content[0].as_text().unwrap().text.clone();

        assert!(text.contains("vault_search"));
        assert!(!text.contains("vault_write_note"));
    }

    #[test]
    fn handle_discover_unrestricted_when_allowed_is_none() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "vault_write_note",
        }));

        let result = handle_discover(&registry, None, None).unwrap();
        let text = result.content[0].as_text().unwrap().text.clone();

        assert!(text.contains("vault_write_note"));
    }

    #[tokio::test]
    async fn handle_call_rejects_tool_outside_allowlist() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "vault_write_note",
        }));
        let db = hq_db::Database::open_memory().unwrap();
        let allowed = ["vault_search"];
        let call_args = serde_json::json!({"tool": "vault_write_note", "args": {}});
        let map = call_args.as_object().unwrap().clone();

        let result = handle_call(&registry, Some(&map), &db, Some(&allowed)).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn handle_call_allows_tool_inside_allowlist() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "vault_search",
        }));
        let db = hq_db::Database::open_memory().unwrap();
        let allowed = ["vault_search"];
        let call_args = serde_json::json!({"tool": "vault_search", "args": {}});
        let map = call_args.as_object().unwrap().clone();

        let result = handle_call(&registry, Some(&map), &db, Some(&allowed)).await;

        assert!(result.is_ok());
        let by_agent = db
            .with_conn(|conn| hq_db::tool_usage::by_agent(conn, "vault_search"))
            .unwrap();
        assert_eq!(by_agent, vec![("mcp-external".to_string(), 1)]);
    }

    /// hq-web's /mcp serializes these types directly, so their JSON is the HTTP wire format.
    #[tokio::test]
    async fn dispatch_results_serialize_to_the_2024_11_05_shape() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "vault_search",
        }));
        let db = hq_db::Database::open_memory().unwrap();
        let call_args = serde_json::json!({"tool": "vault_search"});
        let map = call_args.as_object().unwrap().clone();

        let result = dispatch(&registry, "hq_call", Some(&map), &db, None)
            .await
            .unwrap();

        let expected_text = serde_json::to_string_pretty(&serde_json::json!({"ok": true})).unwrap();
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::json!({
                "content": [{"type": "text", "text": expected_text}],
                "isError": false
            })
        );
        let tools = serde_json::to_value(create_gateway_tools(&registry)).unwrap();
        assert_eq!(tools[1]["name"], "hq_call");
        assert_eq!(tools[1].as_object().unwrap().len(), 3);
    }

    struct EchoArgs;

    #[async_trait::async_trait]
    impl HqTool for EchoArgs {
        fn name(&self) -> &str {
            "echo_args"
        }
        fn description(&self) -> &str {
            "returns its arguments"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(args)
        }
    }

    async fn caller_seen(supplied: Option<&str>, attested: Option<&str>) -> Option<String> {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoArgs));
        let db = hq_db::Database::open_memory().unwrap();
        let mut inner = serde_json::json!({});
        if let Some(id) = supplied {
            inner[hq_tools::harness_session::CALLER_SESSION_ARG] = id.into();
        }
        let call = serde_json::json!({"tool": "echo_args", "args": inner});
        let result = handle_call_as(&registry, call.as_object(), &db, None, attested)
            .await
            .unwrap();
        let text = serde_json::to_value(&result).unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        let args: serde_json::Value = serde_json::from_str(&text).unwrap();
        hq_tools::harness_session::caller_session(&args).map(str::to_string)
    }

    #[tokio::test]
    async fn a_caller_cannot_name_its_own_session() {
        assert_eq!(caller_seen(Some("hs-victim"), None).await, None);
    }

    #[tokio::test]
    async fn the_attested_session_replaces_anything_the_caller_wrote() {
        assert_eq!(caller_seen(None, Some("hs-real")).await.as_deref(), Some("hs-real"));
        assert_eq!(
            caller_seen(Some("hs-victim"), Some("hs-real")).await.as_deref(),
            Some("hs-real")
        );
    }

    #[test]
    fn a_session_token_reaches_task_threads_and_nothing_wider() {
        for denied in ["config_manage", "vault_read", "vault_write_note", "harness_session_spawn", "hq_ask"] {
            assert!(!SESSION_ALLOWLIST.contains(&denied), "{denied}");
        }
        assert!(SESSION_ALLOWLIST.contains(&"task_comment_add"));
    }
}
