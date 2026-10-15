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
/// with `agent_host.handoff_cwd_allow`. It cannot send to or read the output of a
/// session (`harness_session_send`, `harness_session_logs`): the registry does
/// not record who created a session, so those would reach sessions this key
/// never started. No deletes, no vault writes, no session
/// stop/resume/link/goal changes, no config, no tools that reach other
/// services. It can ask HQ's chat agent a question (`hq_ask`) but only in
/// read-only mode, read back only the asks it made, and continue only threads those asks
/// started. The reply it gets is built without the session-log, the host and file-reading tools (see
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

/// Tool names reachable by a tasks-scoped connection (`AGENTHQ_TASKS_API_KEY`, or
/// `hq mcp-serve --scope tasks`): reading, filing, updating and commenting on tasks, working
/// them under a lease (claim, next, heartbeat, release), filing or moving many at once, reading
/// their links and time, and creating the folders and initiatives to place them in. It has no vault tools, so a
/// client on a machine HQ's owner does not control never sees their notes, and none that
/// start, read or message a session, ask HQ's chat agent, delete anything or reach another
/// service, so unlike the handoff scope it is not code execution on any host. The task tools
/// also know the call came in on this scope (see `mark_scope`) and then set no routing tags,
/// write to no agent's mailbox, and attribute writes to `mcp:tasks`. An exact list, so a tool
/// added to the registry later is denied until someone adds it here.
pub const TASKS_ALLOWLIST: &[&str] = &[
    "task_list",
    "task_get",
    "task_related",
    "task_comment_list",
    "task_create",
    "task_update",
    "task_comment_add",
    "task_claim",
    "task_next",
    "task_heartbeat",
    "task_release",
    "task_create_many",
    "task_update_many",
    "task_link_list",
    "task_time_report",
    "folder_list",
    "folder_create",
    "initiative_list",
    "initiative_create",
    "space_list",
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
pub fn create_gateway_tools_scoped(registry: &ToolRegistry, allowed: Option<&[&str]>) -> Vec<Tool> {
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

/// Instructions a scoped connection gets instead of the catalog: the catalog
/// would list tools the key cannot call.
pub const SCOPED_INSTRUCTIONS: &str = "Agent-HQ (restricted access). \
`hq_discover(category?, query?)` lists the tools this connection may use and \
`hq_call(tool, args)` runs one. Tools outside that list are refused.";

/// Server instructions sent on `initialize`: how to use the two gateway tools
/// plus the tool catalog, so a client knows the tools without calling hq_discover.
/// How any agent works a task, in the text every MCP client receives on connect. Short on
/// purpose; the installable `hq-tasks` skill has the long version.
const TASK_PROTOCOL_STEPS: &str = "## Working on tasks\n\
     HQ tracks your work as tasks. Your own queue: `task_next` picks the most urgent task assigned to you and starts it (or `task_list` with `assignee`, then `task_claim`).\n\
     1. Claim before you work. `task_claim` and `task_next` return a `lease` token: keep it for the session.\n\
     2. Pass `lease` on `task_update`, `task_comment_add` and `task_create` so the work is recorded as yours.\n\
     3. Call `task_heartbeat` every few minutes, with a `checkpoint` (summary, next_step) at good stopping points, so another session can pick up where you stopped.\n\
     4. Stop with `task_release` and a status: `ready_for_review` when done and it needs checking, `blocked` (the summary is the reason) when stuck, `to_do` to hand it back. Never `complete` unless the work is verified.\n";
const TASK_PROTOCOL_LINKS: &str = "5. Link what a task came from with `task_link_add`. File a big piece of work as its own initiative with one task per workstream.\n\
     Knowledge and notes belong in the vault, not in tasks.\n\n";
/// The tasks scope has no link-writing tool and no vault, so its step 5 names what it can do.
const TASK_PROTOCOL_LINKS_SCOPED: &str = "5. Give `task_create` the `links` a task came from (a url, pull request, commit or task). `task_create_many` files a big piece of work in one call.\n\n";

/// What a tasks-scoped connection is told on connect: the restricted-access note and how to
/// work a task with the tools it has.
pub fn tasks_scope_instructions() -> String {
    format!("{SCOPED_INSTRUCTIONS}\n\n{TASK_PROTOCOL_STEPS}{TASK_PROTOCOL_LINKS_SCOPED}")
}

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
    if registry.get("task_claim").is_some() {
        instructions.push_str(TASK_PROTOCOL_STEPS);
        instructions.push_str(TASK_PROTOCOL_LINKS);
    }
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
    dispatch_as(registry, name, arguments, db, allowed, None, None).await
}

/// Like `dispatch`, for a caller that proved it is the launched session
/// `caller_session`, or a claude.ai connector grant labelled `connector`. Tools see those and
/// nothing a caller wrote in their place.
pub async fn dispatch_as(
    registry: &ToolRegistry,
    name: &str,
    arguments: Option<&serde_json::Map<String, Value>>,
    db: &hq_db::Database,
    allowed: Option<&[&str]>,
    caller_session: Option<&str>,
    connector: Option<&str>,
) -> Result<CallToolResult, ErrorData> {
    let result = match name {
        "hq_discover" => handle_discover(registry, arguments, allowed),
        "hq_call" => {
            let caller = Caller {
                session: caller_session,
                connector,
            };
            handle_call_as(registry, arguments, db, allowed, caller).await
        }
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
    handle_call_as(registry, arguments, db, allowed, Caller::default()).await
}

/// Who the transport proved is calling, beyond the key's allowlist.
#[derive(Default, Clone, Copy)]
struct Caller<'a> {
    session: Option<&'a str>,
    connector: Option<&'a str>,
}

async fn handle_call_as(
    registry: &ToolRegistry,
    arguments: Option<&serde_json::Map<String, Value>>,
    db: &hq_db::Database,
    allowed: Option<&[&str]>,
    caller: Caller<'_>,
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
    if MARKED_DENIED_TOOLS.contains(&args.tool.as_str())
        && hq_tools::harness_session::spawned_session(&call_args).is_some()
    {
        return Err(ErrorData::invalid_params(
            format!(
                "{}: {}",
                args.tool,
                hq_tools::harness_session::SPAWNED_REFUSAL
            ),
            None,
        ));
    }
    mark_scope(&mut call_args, allowed, caller.connector);
    attest_caller(&mut call_args, caller.session);
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

/// Run one tool the way `hq_call` does for `allowed` (allowlist check, scope marking), but hand
/// back the tool's own JSON: no compaction, so a long list is not cut. For local callers such as
/// `hq task`, which print the result for another program to parse.
pub async fn call_tool_whole(
    registry: &ToolRegistry,
    tool: &str,
    args: Value,
    allowed: Option<&[&str]>,
) -> Result<Value, String> {
    if allowed.is_some_and(|a| !a.contains(&tool)) {
        return Err(format!("tool not permitted for this connection: {tool}"));
    }
    let t = registry
        .get(tool)
        .ok_or_else(|| format!("unknown tool: {tool}"))?;
    let mut args = if args.is_null() { json!({}) } else { args };
    mark_scope(&mut args, allowed, None);
    hq_tools::registry::validate_and_execute(t, args)
        .await
        .map_err(|e| format!("tool error: {e}"))
}

/// Tools a caller inside an HQ-spawned session may not use: they rewrite HQ's own settings.
const MARKED_DENIED_TOOLS: &[&str] = &["config_manage"];

/// Tools a claude.ai connector may run only once the owner approves that exact call: they act
/// on the owner's outside accounts, reach the owner's chat or an agent that acts on what it is
/// sent, persist instructions or settings, pair a machine, delete, or write and publish on the
/// HQ host. Only tools the gateway serves belong here; a registry test checks each name.
/// Coding-agent session tools are deliberately absent (`docs/plans/claude-connector.md`).
pub const CONNECTOR_CONFIRM_TOOLS: &[&str] = &[
    "google_workspace",
    "agent_send_message",
    "config_manage",
    "host_add",
    "self_update_install",
    "self_update_rollback",
    "skill_manage",
    "slash_command_manage",
    "task_delete",
    "note_review",
    "vault_delete",
    "file_edit_batch",
    "git_commit",
    "git_pr",
    "github_clone",
    "host_send",
    "host_read",
    "convert_to_markdown",
    "convert_from_markdown",
    "ocr_extract_text",
];

/// Vault folders whose notes become instructions: skills, slash commands and agent definitions.
const INSTRUCTION_FOLDERS: &[&str] = &["skills/", "_skills/", "_commands/", "agents/"];
/// Vault tools that write a note at `path` (or move one to `to`).
const VAULT_WRITE_TOOLS: &[&str] = &[
    "vault_write_note",
    "vault_append_note",
    "vault_patch_section",
    "vault_frontmatter_update",
    "vault_move",
];
/// Task tools whose tags and assignees put text in a mailbox that the owner's chat or an agent
/// drains, the same reach `agent_send_message` has.
pub const TASK_ROUTING_TOOLS: &[&str] = &[
    "task_create",
    "task_update",
    "task_create_many",
    "task_update_many",
];

/// Whether a connector's call to `tool` with `args` waits for the owner's approval. Remote MCP
/// servers proxy the owner's accounts, and `hq_ask` in full mode hands the question to HQ's own
/// agent with every tool, so both count.
pub fn needs_owner_confirmation(registry: &ToolRegistry, tool: &str, args: &Value) -> bool {
    if tool == "hq_ask" {
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or_default();
        return matches!(
            hq_tools::ask::AskMode::parse(mode),
            Ok(hq_tools::ask::AskMode::Full)
        );
    }
    // An export with `output` writes a file anywhere the HQ user can.
    if matches!(tool, "vault_export" | "vault_export_pdf") {
        return args
            .get("output")
            .and_then(Value::as_str)
            .is_some_and(|o| !o.is_empty());
    }
    if VAULT_WRITE_TOOLS.contains(&tool) {
        return ["path", "to", "destination", "dest"]
            .iter()
            .filter_map(|k| args.get(*k).and_then(Value::as_str))
            .any(in_instruction_folder);
    }
    if TASK_ROUTING_TOOLS.contains(&tool) {
        return routes_to_mailbox(args);
    }
    CONNECTOR_CONFIRM_TOOLS.contains(&tool)
        || registry
            .get(tool)
            .is_some_and(|t| t.category() == hq_tools::remote_mcp::REMOTE_MCP_CATEGORY)
}

fn in_instruction_folder(path: &str) -> bool {
    let path = path
        .trim()
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_ascii_lowercase();
    INSTRUCTION_FOLDERS.iter().any(|f| path.starts_with(f))
}

/// Whether a task call sets tags or assignees, on the call, its `defaults` or any item.
fn routes_to_mailbox(args: &Value) -> bool {
    let sets = |v: &Value| {
        ["tags", "assignees"].iter().any(|k| {
            v.get(*k)
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
        })
    };
    let items = ["tasks", "updates"]
        .iter()
        .filter_map(|k| args.get(*k).and_then(Value::as_array))
        .flatten();
    sets(args) || args.get("defaults").is_some_and(sets) || items.into_iter().any(sets)
}

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

/// Tools learn which key a call came in on only from these arguments: any value
/// the caller sent is dropped, and they are set here for the handoff and tasks scopes and for
/// a connector grant.
fn mark_scope(args: &mut Value, allowed: Option<&[&str]>, connector: Option<&str>) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    obj.remove(hq_tools::harness_session::HANDOFF_SCOPE_ARG);
    obj.remove(hq_tools::harness_session::TASKS_SCOPE_ARG);
    obj.remove(hq_tools::harness_session::CONNECTOR_ARG);
    // Native agent sessions set these themselves; a caller here never may.
    obj.retain(|key, _| !key.starts_with(hq_tools::provenance::NATIVE_ARG_PREFIX));
    if let Some(label) = connector {
        obj.insert(
            hq_tools::harness_session::CONNECTOR_ARG.into(),
            json!(label),
        );
    }
    if allowed == Some(HANDOFF_ALLOWLIST) {
        obj.insert(
            hq_tools::harness_session::HANDOFF_SCOPE_ARG.into(),
            json!(true),
        );
    }
    if allowed == Some(TASKS_ALLOWLIST) {
        obj.insert(
            hq_tools::harness_session::TASKS_SCOPE_ARG.into(),
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
            "host_send",
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

    /// The tasks scope exists so a client can file and update tasks without being
    /// able to run code or read the owner's notes: no vault tools, nothing that spawns,
    /// reads or messages a session, asks HQ's chat agent, deletes, writes the vault or
    /// reaches another service.
    #[test]
    fn tasks_scope_is_task_work_only() {
        for name in [
            "task_list",
            "task_get",
            "task_create",
            "task_update",
            "task_comment_add",
            "task_comment_list",
        ] {
            assert!(TASKS_ALLOWLIST.contains(&name), "tasks must reach {name}");
        }
        for name in [
            "harness_session_spawn",
            "harness_session_handoff",
            "harness_session_send",
            "harness_session_logs",
            "harness_session_list",
            "harness_session_status",
            "session_search",
            "hq_ask",
            "hq_ask_result",
            "task_delete",
            "task_create_from_note",
            "space_create",
            "space_update",
            "vault_search",
            "vault_read",
            "vault_context",
            "vault_write_note",
            "memory_entity_graph",
            "config_manage",
            "host_send",
            "bash",
            "agent_message_send",
            "agent_delegate",
        ] {
            assert!(
                !TASKS_ALLOWLIST.contains(&name),
                "tasks must not reach {name}"
            );
        }
        for name in TASKS_ALLOWLIST {
            assert!(
                ["task_", "folder_", "initiative_", "space_"]
                    .iter()
                    .any(|p| name.starts_with(p)),
                "{name} is not a task, folder, initiative or space tool"
            );
        }
        // Archiving, restoring, purging, writing links and reading who is routed where stay
        // with the owner's key.
        for name in [
            "task_delete",
            "task_restore",
            "task_link_add",
            "task_link_remove",
            "task_routing_audit",
        ] {
            assert!(
                !TASKS_ALLOWLIST.contains(&name),
                "tasks must not reach {name}"
            );
        }
    }

    /// The protocol a tasks-scoped client is told on connect may only name tools it can call.
    #[test]
    fn the_tasks_scope_protocol_names_only_tools_the_scope_has() {
        let text = tasks_scope_instructions();
        let named = text
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|w| w.starts_with("task_"));
        let mut count = 0;
        for name in named {
            count += 1;
            assert!(
                TASKS_ALLOWLIST.contains(&name),
                "the scope protocol names {name}"
            );
        }
        assert!(count >= 5, "{text}");
    }

    #[tokio::test]
    async fn tasks_scope_calls_task_tools_and_refuses_spawn_and_delete() {
        let mut registry = ToolRegistry::new();
        for name in [
            "task_create",
            "task_delete",
            "harness_session_spawn",
            "hq_ask",
        ] {
            registry.register(Box::new(DummyTool { name }));
        }
        let db = hq_db::Database::open_memory().unwrap();
        let call = |name: &str| serde_json::json!({"tool": name, "args": {}});
        let ok = call("task_create");
        assert!(
            handle_call(&registry, ok.as_object(), &db, Some(TASKS_ALLOWLIST))
                .await
                .is_ok()
        );
        for name in ["task_delete", "harness_session_spawn", "hq_ask"] {
            let denied = call(name);
            assert!(
                handle_call(&registry, denied.as_object(), &db, Some(TASKS_ALLOWLIST))
                    .await
                    .is_err(),
                "tasks must not call {name}"
            );
        }
        let found = handle_discover(&registry, None, Some(TASKS_ALLOWLIST)).unwrap();
        let text = format!("{found:?}");
        assert!(text.contains("task_create"));
        assert!(!text.contains("harness_session_spawn") && !text.contains("task_delete"));
    }

    /// `mark_scope` tells the scopes apart by the content of their lists, so two scopes that
    /// ever had the same list would both be marked.
    #[test]
    fn the_scope_allowlists_are_all_different() {
        let lists: [(&str, &[&str]); 4] = [
            ("spark", SPARK_READONLY_ALLOWLIST),
            ("handoff", HANDOFF_ALLOWLIST),
            ("tasks", TASKS_ALLOWLIST),
            ("session", SESSION_ALLOWLIST),
        ];
        for (i, (a, la)) in lists.iter().enumerate() {
            for (b, lb) in lists.iter().skip(i + 1) {
                assert_ne!(la, lb, "{a} and {b} have the same allowlist");
            }
        }
    }

    #[test]
    fn only_an_attested_grant_carries_the_connector_marker() {
        let key = hq_tools::harness_session::CONNECTOR_ARG;
        let mut forged = json!({"q": 1, key: "owner"});
        mark_scope(&mut forged, None, None);
        assert!(
            forged.get(key).is_none(),
            "a caller cannot claim to be a connector"
        );
        let mut granted = json!({key: "forged-label"});
        mark_scope(&mut granted, None, Some("Claude"));
        assert_eq!(granted[key], "Claude");
    }

    #[test]
    fn review_decisions_are_closed_to_every_scoped_key_and_need_the_owner_through_a_connector() {
        for (name, list) in [
            ("spark", SPARK_READONLY_ALLOWLIST),
            ("handoff", HANDOFF_ALLOWLIST),
            ("tasks", TASKS_ALLOWLIST),
        ] {
            assert!(
                !list.contains(&"note_review"),
                "the {name} key can set review state"
            );
        }
        assert!(needs_owner_confirmation(
            &ToolRegistry::new(),
            "note_review",
            &json!({"path": "Notebooks/a.md", "state": "accepted"})
        ));
    }

    #[test]
    fn the_activity_view_is_not_served_to_a_scoped_key() {
        for (name, list) in [
            ("spark", SPARK_READONLY_ALLOWLIST),
            ("handoff", HANDOFF_ALLOWLIST),
            ("tasks", TASKS_ALLOWLIST),
        ] {
            assert!(
                !list.contains(&"task_activity"),
                "the {name} key can read the activity view, which lists sessions and notes across spaces"
            );
        }
    }

    #[test]
    fn a_caller_cannot_pose_as_a_native_agent_session() {
        use hq_tools::provenance::{NATIVE_AGENT_ARG, NATIVE_LEASES_ARG, NATIVE_SESSION_ARG};
        let mut args = json!({
            "path": "Notes/a.md",
            NATIVE_SESSION_ARG: "someone-elses-session",
            NATIVE_AGENT_ARG: "relay",
            NATIVE_LEASES_ARG: ["lease-1"],
            "_hq_native_future": true,
        });
        mark_scope(&mut args, None, None);
        let obj = args.as_object().unwrap();
        assert!(
            obj.keys().all(|k| !k.starts_with("_hq_native_")),
            "native markers survived: {obj:?}"
        );
        assert_eq!(
            args["path"], "Notes/a.md",
            "ordinary arguments are untouched"
        );
    }

    #[test]
    fn connector_confirmation_covers_the_listed_tools_remote_mcp_and_full_asks_only() {
        let registry = ToolRegistry::new();
        let none = json!({});
        assert!(needs_owner_confirmation(&registry, "vault_delete", &none));
        assert!(needs_owner_confirmation(
            &registry,
            "google_workspace",
            &none
        ));
        assert!(!needs_owner_confirmation(&registry, "vault_search", &none));
        assert!(
            !needs_owner_confirmation(&registry, "harness_session_spawn", &none),
            "coding sessions are deliberately not gated"
        );
        assert!(needs_owner_confirmation(
            &registry,
            "hq_ask",
            &json!({"mode": "full"})
        ));
        assert!(needs_owner_confirmation(
            &registry,
            "vault_export",
            &json!({"output": "/home/u/.bashrc"})
        ));
        assert!(!needs_owner_confirmation(
            &registry,
            "vault_export",
            &json!({"path": "a.md"})
        ));
        for path in [
            "skills/x/SKILL.md",
            "/_commands/deploy.md",
            "./Agents/coder.md",
        ] {
            assert!(
                needs_owner_confirmation(&registry, "vault_write_note", &json!({ "path": path })),
                "{path}"
            );
        }
        assert!(!needs_owner_confirmation(
            &registry,
            "vault_write_note",
            &json!({"path": "Notebooks/a.md"})
        ));
        assert!(needs_owner_confirmation(
            &registry,
            "vault_move",
            &json!({"path": "a.md", "to": "skills/a.md"})
        ));
        assert!(needs_owner_confirmation(
            &registry,
            "task_create",
            &json!({"title": "t", "tags": ["relay"]})
        ));
        assert!(!needs_owner_confirmation(
            &registry,
            "task_create",
            &json!({"title": "t", "tags": []})
        ));
        assert!(needs_owner_confirmation(
            &registry,
            "task_create_many",
            &json!({"tasks": [{"title": "a"}, {"title": "b", "assignees": ["hq"]}]})
        ));
        assert!(needs_owner_confirmation(
            &registry,
            "task_update_many",
            &json!({"updates": [{"id": "x"}], "defaults": {"tags": ["relay"]}})
        ));
        assert!(!needs_owner_confirmation(
            &registry,
            "task_update",
            &json!({"id": "x", "status": "complete"})
        ));
        assert!(!needs_owner_confirmation(
            &registry,
            "hq_ask",
            &json!({"mode": "read_only"})
        ));
        assert!(!needs_owner_confirmation(&registry, "hq_ask", &none));
        for padded in [" full", "full\n", "\tfull "] {
            assert!(
                needs_owner_confirmation(&registry, "hq_ask", &json!({ "mode": padded })),
                "the gate reads {padded:?} as the tool does"
            );
        }
    }

    #[test]
    fn each_scope_gets_only_its_own_marker_and_a_caller_cannot_forge_either() {
        let handoff = hq_tools::harness_session::HANDOFF_SCOPE_ARG;
        let tasks = hq_tools::harness_session::TASKS_SCOPE_ARG;
        let forged = || serde_json::json!({ handoff: true, tasks: true, "x": 1 });

        let mut args = forged();
        mark_scope(&mut args, None, None);
        assert!(
            args.get(handoff).is_none() && args.get(tasks).is_none(),
            "owner calls carry no marker"
        );

        let mut args = forged();
        mark_scope(&mut args, Some(SPARK_READONLY_ALLOWLIST), None);
        assert!(args.get(handoff).is_none() && args.get(tasks).is_none());

        let mut args = forged();
        mark_scope(&mut args, Some(TASKS_ALLOWLIST), None);
        assert_eq!(args[tasks], true);
        assert!(
            args.get(handoff).is_none(),
            "a tasks call is never marked as handoff"
        );

        let mut args = forged();
        mark_scope(&mut args, Some(HANDOFF_ALLOWLIST), None);
        assert_eq!(args[handoff], true);
        assert!(
            args.get(tasks).is_none(),
            "a handoff call is never marked as tasks"
        );
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
        mark_scope(&mut args, None, None);
        assert!(args.get(key).is_none(), "owner calls carry no marker");
        let mut args = serde_json::json!({ key: true });
        mark_scope(&mut args, Some(SPARK_READONLY_ALLOWLIST), None);
        assert!(args.get(key).is_none(), "a forged marker is stripped");
        let mut args = serde_json::json!({});
        mark_scope(&mut args, Some(HANDOFF_ALLOWLIST), None);
        assert_eq!(args[key], true);
    }

    #[tokio::test]
    async fn a_marked_caller_cannot_use_the_config_tool_and_null_args_are_tolerated() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool {
            name: "config_manage",
        }));
        let db = hq_db::Database::open_memory().unwrap();
        let marker = hq_tools::harness_session::SPAWNED_SESSION_ARG;
        let marked = serde_json::json!({"tool": "config_manage", "args": {marker: "hs-1"}});
        assert!(
            handle_call(&registry, Some(marked.as_object().unwrap()), &db, None)
                .await
                .is_err()
        );
        let plain = serde_json::json!({"tool": "config_manage", "args": {}});
        assert!(
            handle_call(&registry, Some(plain.as_object().unwrap()), &db, None)
                .await
                .is_ok()
        );

        let null_args = serde_json::json!({"tool": "config_manage", "args": null});
        let marked = mark_spawned_session(null_args.as_object(), "hs-1").unwrap();
        assert_eq!(
            marked["args"][marker], "hs-1",
            "null args become an object, not a lost marker"
        );
        assert!(
            handle_call(&registry, Some(&marked), &db, None)
                .await
                .is_err()
        );
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
        let result = handle_call_as(
            &registry,
            call.as_object(),
            &db,
            None,
            Caller {
                session: attested,
                connector: None,
            },
        )
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
        assert_eq!(
            caller_seen(None, Some("hs-real")).await.as_deref(),
            Some("hs-real")
        );
        assert_eq!(
            caller_seen(Some("hs-victim"), Some("hs-real"))
                .await
                .as_deref(),
            Some("hs-real")
        );
    }

    #[test]
    fn a_session_token_reaches_task_threads_and_nothing_wider() {
        for denied in [
            "config_manage",
            "vault_read",
            "vault_write_note",
            "harness_session_spawn",
            "hq_ask",
        ] {
            assert!(!SESSION_ALLOWLIST.contains(&denied), "{denied}");
        }
        assert!(SESSION_ALLOWLIST.contains(&"task_comment_add"));
    }
}
