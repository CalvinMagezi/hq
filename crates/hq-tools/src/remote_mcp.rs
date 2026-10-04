//! Remote MCP bridge: exposes any Streamable HTTP MCP server configured under
//! `remote_mcp:` as two gateway tools, `<name>_discover` (tools/list with an
//! optional substring filter) and `<name>_call` (tools/call).
//!
//! Requests are stateless JSON-RPC POSTs with a bearer token; no `initialize`
//! handshake is sent, so the server must accept bare `tools/list` and
//! `tools/call` calls.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use hq_core::config::RemoteMcpServer;
use serde_json::{Value, json};

use crate::registry::HqTool;

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
        let body: Value = resp.json().await?;
        let name = &self.server.name;
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

const BEHAVIORAL_PROMPT: &str = "Authentication for this remote MCP server is already configured; call its discover/call tools directly instead of looking for credentials. If a call fails, the error text says why. Actions on a remote server can have real effects: confirm mutating calls with the user unless they already asked for that exact action.";

/// Call a tool on a named remote MCP server directly.
pub async fn call_named_server(server: &RemoteMcpServer, tool: &str, args: Value) -> Result<Value> {
    let client = RemoteMcpClient {
        http: reqwest::Client::new(),
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
            "family-confirm",
            "relay",
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
) -> Vec<Box<dyn HqTool>> {
    let mut tools: Vec<Box<dyn HqTool>> = Vec::new();
    for server in servers {
        let name = server.name.clone();
        let client = Arc::new(RemoteMcpClient {
            http: reqwest::Client::new(),
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
        let tools = create_remote_mcp_tools(&servers, None);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["diagrams_discover", "diagrams_call"]);
        assert!(tools.iter().all(|t| t.category() == REMOTE_MCP_CATEGORY));
        assert!(tools.iter().all(|t| t.requires_live_user_turn()));
        assert!(tools[0].is_read_only() && !tools[1].is_read_only());
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
        let tools = create_remote_mcp_tools(&servers, Some(guest));
        let call_tool = tools
            .into_iter()
            .find(|t| t.name() == "acme_call")
            .unwrap();
        let res = call_tool
            .execute(json!({ "tool": "invoice_create", "args": { "amount": 100 } }))
            .await
            .unwrap();
        assert_eq!(res["status"], "confirmation_requested");
        assert!(res["message"].as_str().unwrap().contains("Carol"));
    }
}
