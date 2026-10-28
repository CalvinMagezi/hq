//! The Gemini Apps profile's MCP endpoint, `/mcp/gemini/v1` (`docs/plans/gemini-connector.md`).
//! Google snapshots a server's tools when it is added, so this path lists a fixed set of
//! read-only tools with fixed schemas; a change to them belongs on a new version path. Only a
//! Gemini grant's access token is accepted here: no API key, session token or dev switch.

use super::guard;
use crate::WsState;
use crate::mcp_http::{answer, negotiated_version, rpc_err, rpc_ok};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use hq_db::mcp_connector::{self as store, GrantAccess, Profile};
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// The tools this version exposes. Each is read-only and listed in `tool_list`.
pub(crate) const GEMINI_TOOLS: &[&str] =
    &["vault_search", "vault_read", "vault_list", "vault_outline"];
/// Search results one call may return, so a result stays small enough to check whole.
const MAX_SEARCH_RESULTS: u64 = 10;
const SEARCH_MODES: &[&str] = &["hybrid", "keyword", "semantic", "recent"];
const JSONRPC_INVALID_PARAMS: i64 = -32602;
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
const JSONRPC_UNAUTHORIZED: i64 = -32001;
const INSTRUCTIONS: &str = "Agent HQ, read-only. Search the owner's notes with vault_search, \
    list a folder with vault_list, see a note's headings with vault_outline, and read it with \
    vault_read. Nothing here changes anything.";

pub(crate) async fn mcp_handler(
    headers: HeaderMap,
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let Some(settings) = state.connector.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let secret = crate::auth::presented_secret(&headers);
    let Some(grant) = gemini_grant(&state, secret) else {
        let challenge = super::gemini_www_authenticate(settings, !secret.is_empty());
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, challenge)],
            rpc_err(id, JSONRPC_UNAUTHORIZED, "Unauthorized"),
        )
            .into_response();
    };
    let is_notification = body.get("id").is_none();
    let method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let reply = match method {
        "initialize" => rpc_ok(
            id,
            json!({
                "protocolVersion": negotiated_version(&body),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "agent-hq-gemini", "version": "1"},
                "instructions": INSTRUCTIONS,
            }),
        ),
        "ping" => rpc_ok(id, json!({})),
        m if m.starts_with("notifications/") => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({ "tools": tool_list() })),
        "tools/call" => call(&state, id, &body, &grant).await,
        _ => rpc_err(id, JSONRPC_METHOD_NOT_FOUND, "Method not found"),
    };
    answer(is_notification, reply)
}

/// The live Gemini grant an access token belongs to; a Claude grant's token is refused here.
fn gemini_grant(state: &WsState, secret: &str) -> Option<GrantAccess> {
    if secret.is_empty() {
        return None;
    }
    let now = super::now();
    state
        .db
        .with_conn(|c| store::grant_for_access_token(c, secret, now))
        .ok()
        .flatten()
        .filter(|g| g.profile == Profile::Gemini)
}

/// The fixed v1 tool list. Annotations are what Google reads to decide on confirmation.
pub(crate) fn tool_list() -> Value {
    let read_only = json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    });
    let tool = |name: &str, title: &str, description: &str, schema: Value| {
        let mut annotations = read_only.clone();
        annotations["title"] = title.into();
        json!({
            "name": name,
            "title": title,
            "description": description,
            "inputSchema": schema,
            "annotations": annotations,
        })
    };
    json!([
        tool(
            "vault_search",
            "Search notes",
            "Search the owner's notes by keyword or meaning. Returns note paths, titles and short snippets. Use mode \"recent\" with no query for the most recently changed notes.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "What to look for"},
                    "mode": {"type": "string", "enum": SEARCH_MODES, "description": "hybrid (default), keyword, semantic or recent"},
                    "limit": {"type": "integer", "description": "Results to return, 1 to 10 (default 10)"}
                },
                "required": []
            }),
        ),
        tool(
            "vault_read",
            "Read a note",
            "Read one note by its path, optionally only a range of lines.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The note's path, as vault_search or vault_list returned it"},
                    "start_line": {"type": "integer", "description": "First line to return, from 1"},
                    "end_line": {"type": "integer", "description": "Last line to return"}
                },
                "required": ["path"]
            }),
        ),
        tool(
            "vault_list",
            "List notes in a folder",
            "List the notes directly inside one folder. An empty folder means the top level.",
            json!({
                "type": "object",
                "properties": {
                    "directory": {"type": "string", "description": "Folder path, for example Notebooks/Projects"}
                },
                "required": []
            }),
        ),
        tool(
            "vault_outline",
            "Outline a note",
            "List a note's headings, to find the part worth reading.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The note's path"}
                },
                "required": ["path"]
            }),
        ),
    ])
}

/// Runs one pinned tool through the connector guard (rate limit, call log, scope), as the
/// `hq_call` the Claude path would send, then drops anything from a system folder.
async fn call(state: &WsState, id: Value, body: &Value, grant: &GrantAccess) -> Json<Value> {
    let params = &body["params"];
    let name = params["name"].as_str().unwrap_or_default();
    if !GEMINI_TOOLS.contains(&name) {
        return rpc_err(id, JSONRPC_INVALID_PARAMS, format!("unknown tool: {name}"));
    }
    let empty = Map::new();
    let given = params["arguments"].as_object().unwrap_or(&empty);
    let args = match pinned_arguments(name, given) {
        Ok(a) => a,
        Err(why) => return tool_error(id, why),
    };
    let inner = json!({
        "jsonrpc": "2.0",
        "id": id.clone(),
        "method": "tools/call",
        "params": {"name": "hq_call", "arguments": {"tool": name, "args": args}},
    });
    let mut reply =
        guard::connector_call(state, id.clone(), &inner, Some(GEMINI_TOOLS), grant).await;
    if reply.0["result"]["isError"] == true || reply.0.get("error").is_some() {
        return reply;
    }
    let text = reply.0["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    match without_system_paths(text) {
        Some(clean) => {
            reply.0["result"]["content"] = json!([{ "type": "text", "text": clean }]);
            reply
        }
        // A result HQ cannot check whole (cut for size, or not JSON) is not passed on.
        None => tool_error(
            id,
            "The result was too large to return; narrow the query or read a smaller range.",
        ),
    }
}

/// The arguments a pinned tool takes, with the right types, nothing else, and no path into a
/// system folder. The error is shown to the model.
fn pinned_arguments(tool: &str, given: &Map<String, Value>) -> Result<Value, &'static str> {
    let text = |k: &str| given.get(k).and_then(Value::as_str);
    let line = |k: &str| given.get(k).and_then(Value::as_u64);
    let mut out = Map::new();
    match tool {
        "vault_search" => {
            if let Some(q) = text("query") {
                out.insert("query".into(), q.into());
            }
            if let Some(m) = text("mode") {
                if !SEARCH_MODES.contains(&m) {
                    return Err("mode must be hybrid, keyword, semantic or recent");
                }
                out.insert("mode".into(), m.into());
            }
            let limit = line("limit")
                .unwrap_or(MAX_SEARCH_RESULTS)
                .clamp(1, MAX_SEARCH_RESULTS);
            out.insert("limit".into(), limit.into());
        }
        "vault_list" => {
            let dir = text("directory").unwrap_or_default().trim_matches('/');
            if !dir.is_empty() && is_system_path(dir) {
                return Err("that folder is not available to this connection");
            }
            out.insert("directory".into(), dir.into());
        }
        _ => {
            let path = text("path").ok_or("path is required")?;
            if is_system_path(path) {
                return Err("that note is not available to this connection");
            }
            out.insert("path".into(), path.into());
            for k in ["start_line", "end_line"] {
                if let Some(n) = line(k).filter(|_| tool == "vault_read") {
                    out.insert(k.into(), n.into());
                }
            }
        }
    }
    Ok(Value::Object(out))
}

/// HQ's own folders (`_system`, `_threads`, `_mailboxes`, `_data`, `_trash` and the like),
/// hidden ones, and anything that is not a plain relative path.
pub(crate) fn is_system_path(path: &str) -> bool {
    path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|seg| seg.is_empty() || seg.starts_with('_') || seg.starts_with('.'))
}

/// The tool's JSON with every listed path in a system folder removed, or `None` when the text
/// is not whole JSON. Search results, their related notes and list entries are all filtered.
fn without_system_paths(text: &str) -> Option<String> {
    let mut v: Value = serde_json::from_str(text).ok()?;
    if let Some(results) = v.get_mut("results").and_then(Value::as_array_mut) {
        results.retain(|r| r["path"].as_str().is_some_and(|p| !is_system_path(p)));
        for r in results.iter_mut() {
            if let Some(related) = r.get_mut("related").and_then(Value::as_array_mut) {
                related.retain(|p| p.as_str().is_some_and(|p| !is_system_path(p)));
            }
        }
        let count = results.len();
        v["count"] = count.into();
    }
    if let Some(paths) = v.get_mut("paths").and_then(Value::as_array_mut) {
        paths.retain(|p| p.as_str().is_some_and(|p| !is_system_path(p)));
        let count = paths.len();
        v["count"] = count.into();
    }
    serde_json::to_string_pretty(&v).ok()
}

fn tool_error(id: Value, text: &str) -> Json<Value> {
    rpc_ok(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
    )
}

#[cfg(test)]
mod tests;
