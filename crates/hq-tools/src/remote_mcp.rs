//! Remote MCP bridge: exposes any Streamable HTTP MCP server configured under
//! `remote_mcp:` as two gateway tools, `<name>_discover` (tools/list with an
//! optional substring filter) and `<name>_call` (tools/call).
//!
//! Requests are stateless JSON-RPC POSTs with a bearer token; no `initialize`
//! handshake is sent, so the server must accept bare `tools/list` and
//! `tools/call` calls.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

use anyhow::Result;
use async_trait::async_trait;
use hq_core::config::{RemoteMcpAsk, RemoteMcpServer};
use serde_json::{Value, json};

use crate::registry::HqTool;

/// The most a server reply may be. A tool reply is cut far below this before a model
/// sees it; the cap only keeps a hostile or broken server from filling memory.
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// A client that never follows a redirect: a 307 or 308 would send the same POST, with
/// the tool arguments and the bearer token for this server, to whatever host it names.
fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

struct RemoteMcpClient {
    http: reqwest::Client,
    server: RemoteMcpServer,
    counter: AtomicU64,
}

impl RemoteMcpClient {
    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });

        let mut builder = self
            .http
            .post(&self.server.url)
            .header("Accept", "application/json, text/event-stream")
            .json(&req);
        if let Some(key) = &self.server.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder.send().await?;

        let status = resp.status();
        let name = &self.server.name;
        if status.is_redirection() {
            anyhow::bail!(
                "{name} MCP server answered {method} with a redirect ({status}), which is not followed: set the final URL in remote_mcp"
            );
        }
        let bytes = crate::web::read_limited(resp, MAX_RESPONSE_BYTES).await?;
        let body: Value = match serde_json::from_slice(&bytes) {
            Ok(body) => body,
            Err(_) if !status.is_success() => {
                let text: String = String::from_utf8_lossy(&bytes).chars().take(500).collect();
                anyhow::bail!("{name} MCP HTTP error {status} for {method}: {text}");
            }
            Err(e) => anyhow::bail!("{name} MCP reply to {method} is not JSON: {e}"),
        };
        if !status.is_success() {
            anyhow::bail!("{name} MCP HTTP error {status} for {method}: {body}");
        }
        if let Some(err) = body.get("error") {
            anyhow::bail!("{name} MCP error for {method}: {err}");
        }
        Ok(body["result"].clone())
    }

    /// Text blocks are joined and parsed as JSON when possible; an image
    /// block is surfaced separately instead of inlined as base64 text.
    async fn call_tool(&self, tool: &str, args: Value) -> Result<Value> {
        let result = self
            .rpc("tools/call", json!({ "name": tool, "arguments": args }))
            .await?;

        let content = result["content"].as_array().cloned().unwrap_or_default();
        let text: String = content
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let image = content.iter().find(|b| b["type"] == "image").cloned();

        let mut out =
            serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({ "text": text }));
        if let (Some(img), Some(obj)) = (image, out.as_object_mut()) {
            obj.insert("image".to_string(), img);
        }
        if result["isError"].as_bool().unwrap_or(false) {
            anyhow::bail!("{} tool {tool} returned an error: {out}", self.server.name);
        }
        Ok(out)
    }
}

/// Shared by every remote server's tools so one tool-policy entry covers them all.
pub const REMOTE_MCP_CATEGORY: &str = "remote_mcp";

/// The suffix of the tool that calls a server's tools; the rest of its name is the server.
const CALL_TOOL_SUFFIX: &str = "_call";
/// The suffix of the tool that only lists a server's tools.
const DISCOVER_TOOL_SUFFIX: &str = "_discover";

/// Tools each server marked `annotations.readOnlyHint: true` in its last `tools/list`, by server
/// name. Only a literal `true` is stored, so a tool with no mark, a `false` or a non-boolean is
/// never in the set and is treated as a write.
static READ_ONLY_TOOLS: LazyLock<RwLock<HashMap<String, HashSet<String>>>> =
    LazyLock::new(Default::default);

/// Whether every remote tool asks (`remote_mcp_ask: all`). Set where the tools are built, so the
/// agent and the connector gateway read the same answer.
static ASK_FOR_ALL: AtomicBool = AtomicBool::new(false);

/// Keep which of `server`'s `tools` are marked read-only, replacing what an earlier listing said.
pub fn record_tool_list(server: &str, tools: &[Value]) {
    let read_only: HashSet<String> = tools
        .iter()
        .filter(|t| t["annotations"]["readOnlyHint"] == Value::Bool(true))
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    match READ_ONLY_TOOLS.write() {
        Ok(mut all) => {
            all.insert(server.to_string(), read_only);
        }
        Err(e) => tracing::warn!(error = %e, "remote_mcp: read-only marks not stored"),
    }
}

/// Whether a call to the gateway tool `gateway_tool` (`<server>_call`) with `args` waits for the
/// owner under `mode`. A tool is spared only when its server said so in a listing we have seen.
pub fn call_asks_owner(mode: RemoteMcpAsk, gateway_tool: &str, args: &Value) -> bool {
    if mode == RemoteMcpAsk::All {
        return true;
    }
    // Listing changes nothing, and it is what refreshes the read-only marks.
    if gateway_tool.ends_with(DISCOVER_TOOL_SUFFIX) {
        return false;
    }
    let Some(server) = gateway_tool.strip_suffix(CALL_TOOL_SUFFIX) else {
        return true;
    };
    let Some(tool) = args.get("tool").and_then(Value::as_str) else {
        return true;
    };
    let Ok(all) = READ_ONLY_TOOLS.read() else {
        return true;
    };
    !all.get(server).is_some_and(|set| set.contains(tool))
}

/// [`call_asks_owner`] under the mode the tools were built with.
pub fn asks_owner(gateway_tool: &str, args: &Value) -> bool {
    let mode = if ASK_FOR_ALL.load(Ordering::Relaxed) {
        RemoteMcpAsk::All
    } else {
        RemoteMcpAsk::Writes
    };
    call_asks_owner(mode, gateway_tool, args)
}

const BEHAVIORAL_PROMPT: &str = "Authentication for this remote MCP server is already configured; call its discover/call tools directly instead of looking for credentials. If a call fails, the error text says why. Actions on a remote server can have real effects: confirm mutating calls with the user unless they already asked for that exact action.";

/// Call a tool on a named remote MCP server directly.
pub async fn call_named_server(server: &RemoteMcpServer, tool: &str, args: Value) -> Result<Value> {
    let client = RemoteMcpClient {
        http: http_client()?,
        server: server.clone(),
        counter: AtomicU64::new(1),
    };
    client.call_tool(tool, args).await
}

struct RemoteMcpTool {
    name: String,
    description: String,
    discover: bool,
    client: Arc<RemoteMcpClient>,
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

impl RemoteMcpTool {
    async fn request_owner_confirmation(
        &self,
        guest: &crate::family_guest::FamilyGuestContext,
        args: &Value,
    ) -> Result<Value> {
        let tool = args
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let tool_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
        let token = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();

        let config = hq_core::config::HqConfig::load().unwrap_or_default();
        let vault_path = &config.vault_path;

        let summary = format!(
            "{} wants to call {}.{} with {}",
            guest.name, self.client.server.name, tool, tool_args
        );
        let pending = crate::family_confirm::PendingFamilyAction {
            token: token.clone(),
            requester_name: guest.name.clone(),
            origin_channel_id: guest.origin_channel_id,
            server_name: self.client.server.name.clone(),
            tool: tool.to_string(),
            args: tool_args,
            summary: summary.clone(),
            created_at: chrono::Utc::now(),
        };
        crate::family_confirm::add_pending(vault_path, pending)?;

        let content = format!(
            "{summary}. Reply \"approve {token}\" or \"deny {token}\", or use the buttons — family-confirm-{token}"
        );
        let mut msg = hq_core::mailbox::new_message(
            hq_core::mailbox::FAMILY_CONFIRM_SENDER,
            hq_core::mailbox::RELAY_MAILBOX,
            hq_core::types::MailboxMessageType::Nudge,
            Some("Family Action Approval"),
            &content,
            None,
        );
        msg.meta.insert(
            "origin_channel_id".to_string(),
            guest.origin_channel_id.to_string(),
        );
        msg.meta.insert(
            hq_core::mailbox::META_INTERRUPT.to_string(),
            "true".to_string(),
        );
        let _ = hq_core::mailbox::send_message(vault_path, &msg);

        Ok(json!({
            "status": "confirmation_requested",
            "message": format!(
                "This action is paused pending approval from {owner} (token: {token}). Do not retry this tool call; tell {} that you are checking with {owner} first.",
                guest.name,
                owner = guest.owner_name
            )
        }))
    }
}

#[async_trait]
impl HqTool for RemoteMcpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        if self.discover {
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Substring to filter tool names and descriptions" }
                },
                "required": []
            })
        } else {
            json!({
                "type": "object",
                "properties": {
                    "tool": { "type": "string", "description": "Tool name from the discover results" },
                    "args": { "type": "object", "description": "Arguments matching the tool's parameter schema" }
                },
                "required": ["tool"]
            })
        }
    }
    fn category(&self) -> &str {
        REMOTE_MCP_CATEGORY
    }
    fn is_read_only(&self) -> bool {
        self.discover
    }
    fn requires_live_user_turn(&self) -> bool {
        self.client.server.live_user_turn_only
    }
    fn behavioral_prompt(&self) -> Option<&str> {
        Some(BEHAVIORAL_PROMPT)
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        if !self.discover {
            if let Some(guest) = &self.family_guest {
                return self.request_owner_confirmation(guest, &args).await;
            }
            let tool = args
                .get("tool")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("'tool' is required"))?;
            let tool_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
            return self.client.call_tool(tool, tool_args).await;
        }

        let result = self.client.rpc("tools/list", json!({})).await?;
        let tools = result["tools"].as_array().cloned().unwrap_or_default();
        record_tool_list(&self.client.server.name, &tools);
        let Some(q) = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::to_lowercase)
        else {
            return Ok(json!({ "tools": tools }));
        };
        let filtered: Vec<Value> = tools
            .into_iter()
            .filter(|t| {
                let name = t["name"].as_str().unwrap_or_default().to_lowercase();
                let desc = t["description"].as_str().unwrap_or_default().to_lowercase();
                name.contains(&q) || desc.contains(&q)
            })
            .collect();
        Ok(json!({ "tools": filtered }))
    }
}

/// Build the discover/call tool pair for every configured remote MCP server.
pub fn create_remote_mcp_tools(
    servers: &[RemoteMcpServer],
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
    ask: RemoteMcpAsk,
) -> Vec<Box<dyn HqTool>> {
    ASK_FOR_ALL.store(ask == RemoteMcpAsk::All, Ordering::Relaxed);
    let mut tools: Vec<Box<dyn HqTool>> = Vec::new();
    if servers.is_empty() {
        return tools;
    }
    let http = match http_client() {
        Ok(http) => http,
        Err(e) => {
            tracing::warn!(error = %e, "remote_mcp: no HTTP client, remote MCP tools left out");
            return tools;
        }
    };
    for server in servers {
        let name = server.name.clone();
        let client = Arc::new(RemoteMcpClient {
            http: http.clone(),
            server: server.clone(),
            counter: AtomicU64::new(1),
        });
        tools.push(Box::new(RemoteMcpTool {
            name: format!("{name}_discover"),
            description: format!(
                "List the tools the remote `{name}` MCP server exposes, with parameter schemas. Filter with `query`."
            ),
            discover: true,
            client: client.clone(),
            family_guest: family_guest.clone(),
        }));
        tools.push(Box::new(RemoteMcpTool {
            name: format!("{name}_call"),
            description: format!(
                "Call a tool on the remote `{name}` MCP server by name. Use {name}_discover first to find tools and their schemas."
            ),
            discover: false,
            client,
            family_guest: family_guest.clone(),
        }));
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_server_yields_a_named_tool_pair() {
        let servers = vec![RemoteMcpServer {
            name: "diagrams".into(),
            url: "https://example.invalid/mcp".into(),
            api_key: None,
            live_user_turn_only: true,
        }];
        let tools = create_remote_mcp_tools(&servers, None, RemoteMcpAsk::Writes);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["diagrams_discover", "diagrams_call"]);
        assert!(tools.iter().all(|t| t.category() == REMOTE_MCP_CATEGORY));
        assert!(tools.iter().all(|t| t.requires_live_user_turn()));
        assert!(tools[0].is_read_only() && !tools[1].is_read_only());
    }

    fn server_at(url: String) -> RemoteMcpServer {
        RemoteMcpServer {
            name: "acme".into(),
            url,
            api_key: Some("secret-token".into()),
            live_user_turn_only: false,
        }
    }

    /// A 307 would re-POST the tool arguments and the bearer token to the host it names.
    #[tokio::test]
    async fn a_redirect_is_refused_and_not_followed() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let elsewhere = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result": {}})))
            .expect(0)
            .mount(&elsewhere)
            .await;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(307).insert_header("Location", elsewhere.uri().as_str()),
            )
            .mount(&server)
            .await;
        let err = call_named_server(&server_at(server.uri()), "t", json!({"a": 1}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("redirect"), "{err}");
        elsewhere.verify().await;
    }

    #[tokio::test]
    async fn an_oversized_reply_is_refused() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![b' '; MAX_RESPONSE_BYTES + 1]),
            )
            .mount(&server)
            .await;
        let err = call_named_server(&server_at(server.uri()), "t", json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[tokio::test]
    async fn a_normal_reply_still_works() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let reply = json!({"jsonrpc": "2.0", "id": 1, "result": {"content": [{"type": "text", "text": "{\"ok\": true}"}]}});
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply))
            .mount(&server)
            .await;
        let out = call_named_server(&server_at(server.uri()), "t", json!({}))
            .await
            .unwrap();
        assert_eq!(out["ok"], true);
    }

    #[tokio::test]
    async fn family_guest_pauses_remote_mcp_call() {
        let servers = vec![RemoteMcpServer {
            name: "acme".into(),
            url: "https://example.invalid/mcp".into(),
            api_key: None,
            live_user_turn_only: false,
        }];
        let guest = crate::family_guest::FamilyGuestContext {
            name: "Carol".into(),
            origin_channel_id: 999888,
            owner_name: "Owner".into(),
            allowed_harnesses: Vec::new(),
        };
        let tools = create_remote_mcp_tools(&servers, Some(guest), RemoteMcpAsk::Writes);
        let call_tool = tools.into_iter().find(|t| t.name() == "acme_call").unwrap();
        let res = call_tool
            .execute(json!({ "tool": "invoice_create", "args": { "amount": 100 } }))
            .await
            .unwrap();
        assert_eq!(res["status"], "confirmation_requested");
        assert!(res["message"].as_str().unwrap().contains("Carol"));
    }

    fn listing(server: &str, tools: Value) {
        record_tool_list(server, tools.as_array().unwrap());
    }

    #[test]
    fn only_a_literal_true_mark_spares_a_tool() {
        listing(
            "marks",
            json!([
                {"name": "get", "annotations": {"readOnlyHint": true}},
                {"name": "put", "annotations": {"readOnlyHint": false}},
                {"name": "bare"},
                {"name": "noann", "annotations": {}},
                {"name": "text", "annotations": {"readOnlyHint": "true"}},
                {"name": "num", "annotations": {"readOnlyHint": 1}},
            ]),
        );
        let asks = |tool: &str| {
            call_asks_owner(RemoteMcpAsk::Writes, "marks_call", &json!({"tool": tool}))
        };
        assert!(!asks("get"));
        for tool in ["put", "bare", "noann", "text", "num", "never_listed"] {
            assert!(asks(tool), "{tool} must ask");
        }
    }

    #[test]
    fn all_asks_even_for_a_read_only_tool() {
        listing(
            "strict",
            json!([{"name": "get", "annotations": {"readOnlyHint": true}}]),
        );
        let args = json!({"tool": "get"});
        assert!(!call_asks_owner(RemoteMcpAsk::Writes, "strict_call", &args));
        assert!(call_asks_owner(RemoteMcpAsk::All, "strict_call", &args));
    }

    #[test]
    fn a_server_never_listed_or_a_malformed_call_asks() {
        let args = json!({"tool": "get"});
        assert!(call_asks_owner(
            RemoteMcpAsk::Writes,
            "unlisted_call",
            &args
        ));
        listing(
            "known",
            json!([{"name": "get", "annotations": {"readOnlyHint": true}}]),
        );
        assert!(call_asks_owner(
            RemoteMcpAsk::Writes,
            "known_call",
            &json!({})
        ));
        assert!(call_asks_owner(
            RemoteMcpAsk::Writes,
            "known_call",
            &json!({"tool": 5})
        ));
        assert!(call_asks_owner(RemoteMcpAsk::Writes, "known", &args));
    }

    #[test]
    fn a_new_listing_replaces_the_old_marks() {
        listing(
            "fresh",
            json!([{"name": "get", "annotations": {"readOnlyHint": true}}]),
        );
        listing(
            "fresh",
            json!([{"name": "get", "annotations": {"readOnlyHint": false}}]),
        );
        let args = json!({"tool": "get"});
        assert!(call_asks_owner(RemoteMcpAsk::Writes, "fresh_call", &args));
    }

    #[tokio::test]
    async fn discover_records_the_marks() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let reply = json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": [
            {"name": "read_it", "annotations": {"readOnlyHint": true}},
            {"name": "change_it"}
        ]}});
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(reply))
            .mount(&server)
            .await;
        let servers = vec![RemoteMcpServer {
            name: "listed".into(),
            url: server.uri(),
            api_key: None,
            live_user_turn_only: false,
        }];
        let tools = create_remote_mcp_tools(&servers, None, RemoteMcpAsk::Writes);
        let discover = tools
            .iter()
            .find(|t| t.name() == "listed_discover")
            .unwrap();
        // Filtered to nothing, the marks are still kept from the full listing.
        discover.execute(json!({"query": "zzz"})).await.unwrap();
        let read = json!({"tool": "read_it"});
        let change = json!({"tool": "change_it"});
        assert!(!asks_owner("listed_call", &read));
        assert!(asks_owner("listed_call", &change));
    }
}
