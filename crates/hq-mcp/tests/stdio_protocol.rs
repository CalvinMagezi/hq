//! Drives the stdio MCP server with raw JSON-RPC lines, the way Claude Code does.

use std::sync::Arc;
use std::time::Duration;

use hq_mcp::server::HqMcpServer;
use hq_tools::registry::{HqTool, ToolRegistry};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};

const PIPE_BUFFER_BYTES: usize = 1 << 20;
const PINNED_PROTOCOL_VERSION: &str = "2024-11-05";
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

struct EchoTool;

#[async_trait::async_trait]
impl HqTool for EchoTool {
    fn name(&self) -> &str {
        "echo_tool"
    }
    fn description(&self) -> &str {
        "echoes its arguments"
    }
    fn category(&self) -> &str {
        "testing"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, args: Value) -> anyhow::Result<Value> {
        Ok(json!({"echo": args}))
    }
}

struct FailTool;

#[async_trait::async_trait]
impl HqTool for FailTool {
    fn name(&self) -> &str {
        "fail_tool"
    }
    fn description(&self) -> &str {
        "always fails"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<Value> {
        anyhow::bail!("boom")
    }
}

struct Client {
    writer: tokio::io::WriteHalf<DuplexStream>,
    lines: Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
}

impl Client {
    async fn send(&mut self, msg: Value) {
        let mut line = msg.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        loop {
            let line = tokio::time::timeout(REPLY_TIMEOUT, self.lines.next_line())
                .await
                .expect("server reply timed out")
                .unwrap()
                .expect("server closed the pipe");
            let reply: Value = serde_json::from_str(&line).unwrap();
            if reply["id"] == json!(id) {
                return reply;
            }
        }
    }
}

fn start_server() -> Client {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    registry.register(Box::new(FailTool));
    let db = hq_db::Database::open_memory().unwrap();
    let server = HqMcpServer::new(Arc::new(registry), db);

    let (client_io, server_io) = tokio::io::duplex(PIPE_BUFFER_BYTES);
    tokio::spawn(async move {
        let running = rmcp::serve_server(server, server_io).await.unwrap();
        let _ = running.waiting().await;
    });
    let (reader, writer) = tokio::io::split(client_io);
    Client { writer, lines: BufReader::new(reader).lines() }
}

async fn initialized_client() -> (Client, Value) {
    initialized_client_at(PINNED_PROTOCOL_VERSION).await
}

async fn initialized_client_at(protocol_version: &str) -> (Client, Value) {
    let mut client = start_server();
    let init = client
        .request(
            1,
            "initialize",
            json!({
                "protocolVersion": protocol_version,
                "capabilities": {},
                "clientInfo": {"name": "stdio-test", "version": "0"}
            }),
        )
        .await;
    client
        .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
    (client, init)
}

#[tokio::test]
async fn initialize_reports_server_identity_and_tools_capability() {
    let (_client, init) = initialized_client().await;
    let result = &init["result"];
    let keys: Vec<&String> = result.as_object().unwrap().keys().collect();
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(keys, ["capabilities", "instructions", "protocolVersion", "serverInfo"]);
    assert_eq!(result["protocolVersion"], PINNED_PROTOCOL_VERSION);
    assert_eq!(
        result["serverInfo"],
        json!({"name": "agent-hq", "version": env!("CARGO_PKG_VERSION")})
    );
    assert_eq!(result["capabilities"], json!({"tools": {}}));
    assert!(result["instructions"].as_str().unwrap().contains("**echo_tool**"));
}

#[tokio::test]
async fn newer_client_protocol_is_answered_with_the_pinned_version() {
    let (_client, init) = initialized_client_at("2025-06-18").await;
    assert_eq!(init["result"]["protocolVersion"], PINNED_PROTOCOL_VERSION);
}

#[tokio::test]
async fn tools_list_is_exactly_the_two_gateway_tools() {
    let (mut client, _) = initialized_client().await;
    let reply = client.request(2, "tools/list", json!({})).await;
    let result_keys: Vec<&String> = reply["result"].as_object().unwrap().keys().collect();
    assert_eq!(result_keys, ["tools"]);
    let tools = reply["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["hq_discover", "hq_call"]);
    assert_eq!(tools[0]["inputSchema"]["type"], "object");
    assert!(tools[0]["inputSchema"]["properties"]["category"]["description"]
        .as_str()
        .unwrap()
        .contains("testing"));
    assert_eq!(tools[1]["inputSchema"]["required"], json!(["tool"]));
    for tool in tools {
        let keys: Vec<&String> = tool.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["description", "inputSchema", "name"], "unexpected fields on {tool}");
    }
}

#[tokio::test]
async fn tools_call_runs_the_named_tool_through_hq_call() {
    let (mut client, _) = initialized_client().await;
    let reply = client
        .request(
            3,
            "tools/call",
            json!({"name": "hq_call", "arguments": {"tool": "echo_tool", "args": {"x": 1}}}),
        )
        .await;
    let result = &reply["result"];
    let keys: Vec<&String> = result.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["content", "isError"]);
    assert_eq!(result["isError"], false);
    assert_eq!(result["content"][0]["type"], "text");
    let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, json!({"echo": {"x": 1}}));
}

#[tokio::test]
async fn tools_call_discover_lists_registered_tools() {
    let (mut client, _) = initialized_client().await;
    let reply = client
        .request(4, "tools/call", json!({"name": "hq_discover", "arguments": {}}))
        .await;
    assert_eq!(reply["result"]["isError"], false);
    assert!(reply["result"]["content"][0]["text"].as_str().unwrap().contains("echo_tool"));
}

#[tokio::test]
async fn unknown_gateway_tool_is_an_invalid_params_error() {
    let (mut client, _) = initialized_client().await;
    let reply = client
        .request(5, "tools/call", json!({"name": "vault_read", "arguments": {}}))
        .await;
    assert_eq!(reply["error"]["code"], -32602);
    assert!(reply["error"]["message"].as_str().unwrap().contains("unknown tool: vault_read"));
}

#[tokio::test]
async fn unknown_inner_tool_is_an_invalid_params_error() {
    let (mut client, _) = initialized_client().await;
    let reply = client
        .request(
            6,
            "tools/call",
            json!({"name": "hq_call", "arguments": {"tool": "no_such_tool"}}),
        )
        .await;
    assert_eq!(reply["error"]["code"], -32602);
    assert!(reply["error"]["message"].as_str().unwrap().contains("unknown tool: no_such_tool"));
}

#[tokio::test]
async fn failing_inner_tool_is_a_tool_error_not_a_protocol_error() {
    let (mut client, _) = initialized_client().await;
    let reply = client
        .request(7, "tools/call", json!({"name": "hq_call", "arguments": {"tool": "fail_tool"}}))
        .await;
    let result = &reply["result"];
    let keys: Vec<&String> = result.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["content", "isError"]);
    assert_eq!(result["isError"], true);
    assert_eq!(result["content"][0]["text"], "tool error: boom");
}

#[tokio::test]
async fn ping_answers_with_an_empty_result() {
    let (mut client, _) = initialized_client().await;
    let reply = client.request(8, "ping", json!({})).await;
    assert!(reply.get("error").is_none(), "ping failed: {reply}");
    assert_eq!(reply["result"], json!({}));
}

struct NamedTool(&'static str);

#[async_trait::async_trait]
impl HqTool for NamedTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "named tool"
    }
    fn category(&self) -> &str {
        "testing"
    }
    fn parameters(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<Value> {
        Ok(json!({"ran": self.0}))
    }
}

/// A server limited to the tasks scope, the way `hq mcp-serve --scope tasks` builds it.
async fn tasks_scoped_client() -> Client {
    let mut registry = ToolRegistry::new();
    for name in ["task_create", "task_delete", "harness_session_spawn", "bash"] {
        registry.register(Box::new(NamedTool(name)));
    }
    let db = hq_db::Database::open_memory().unwrap();
    let server = HqMcpServer::new(Arc::new(registry), db)
        .with_allowlist(hq_mcp::gateway::TASKS_ALLOWLIST);
    let (client_io, server_io) = tokio::io::duplex(PIPE_BUFFER_BYTES);
    tokio::spawn(async move {
        let running = rmcp::serve_server(server, server_io).await.unwrap();
        let _ = running.waiting().await;
    });
    let (reader, writer) = tokio::io::split(client_io);
    let mut client = Client { writer, lines: BufReader::new(reader).lines() };
    client
        .request(
            1,
            "initialize",
            json!({
                "protocolVersion": PINNED_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "scoped-test", "version": "0"}
            }),
        )
        .await;
    client
        .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
    client
}

#[tokio::test]
async fn a_tasks_scoped_server_runs_task_tools_and_refuses_the_rest() {
    let mut client = tasks_scoped_client().await;

    let ok = client
        .request(2, "tools/call", json!({"name": "hq_call", "arguments": {"tool": "task_create", "args": {}}}))
        .await;
    assert_eq!(ok["result"]["isError"], false, "{ok}");

    for (id, tool) in [(3, "task_delete"), (4, "harness_session_spawn"), (5, "bash")] {
        let denied = client
            .request(id, "tools/call", json!({"name": "hq_call", "arguments": {"tool": tool, "args": {}}}))
            .await;
        assert!(
            denied.get("error").is_some() || denied["result"]["isError"] == true,
            "the tasks scope must refuse {tool}: {denied}"
        );
    }

    let found = client
        .request(6, "tools/call", json!({"name": "hq_discover", "arguments": {}}))
        .await;
    let text = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("task_create"), "{text}");
    for hidden in ["task_delete", "harness_session_spawn", "bash"] {
        assert!(!text.contains(hidden), "discovery must not list {hidden}: {text}");
    }
}

#[tokio::test]
async fn a_scoped_server_sends_instructions_without_the_catalog() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NamedTool("bash")));
    let db = hq_db::Database::open_memory().unwrap();
    let server = HqMcpServer::new(Arc::new(registry), db)
        .with_allowlist(hq_mcp::gateway::TASKS_ALLOWLIST);
    let (client_io, server_io) = tokio::io::duplex(PIPE_BUFFER_BYTES);
    tokio::spawn(async move {
        let running = rmcp::serve_server(server, server_io).await.unwrap();
        let _ = running.waiting().await;
    });
    let (reader, writer) = tokio::io::split(client_io);
    let mut client = Client { writer, lines: BufReader::new(reader).lines() };
    let init = client
        .request(
            1,
            "initialize",
            json!({
                "protocolVersion": PINNED_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "scoped-test", "version": "0"}
            }),
        )
        .await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert_eq!(instructions, hq_mcp::gateway::tasks_scope_instructions());
    assert!(!instructions.contains("bash"));
}
