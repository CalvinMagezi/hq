//! MCP over HTTP (JSON-RPC 2.0 Streamable HTTP transport).

use crate::WsState;
use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use std::sync::Arc;

/// Env var holding the handoff-scoped key.
const HANDOFF_KEY_ENV: &str = "AGENTHQ_HANDOFF_API_KEY";
/// Env var holding the tasks-scoped key.
const TASKS_KEY_ENV: &str = "AGENTHQ_TASKS_API_KEY";

fn rpc_ok(id: Value, result: Value) -> Json<Value> {
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn rpc_err(id: Value, code: i64, message: impl Into<String>) -> Json<Value> {
    Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}}))
}

/// MCP over HTTP handler.
/// Handles initialize, tools/list, and tools/call.
/// Auth: set AGENTHQ_API_KEY (full access) and/or AGENTHQ_SPARK_API_KEY
/// (read-only allowlist) and/or AGENTHQ_TASKS_API_KEY (reads plus task writes,
/// no session tools) and/or AGENTHQ_HANDOFF_API_KEY (reads plus task writes
/// and session spawn/send) env vars; clients send x-api-key or
/// Authorization: Bearer <key>. With no key set every call is refused, except
/// under the loopback-only dev switch. See `crate::auth::resolve_identity`.
pub(crate) async fn mcp_handler(
    headers: axum::http::HeaderMap,
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let keys = McpKeys::from_env();
    let switch_on = std::env::var(crate::auth::MCP_DEV_NO_AUTH_ENV).is_ok_and(|v| v == "1");
    let dev_open = crate::auth::mcp_dev_open(&headers, switch_on, state.web_bind_is_loopback);
    handle_mcp(&state, &headers, body, &keys, dev_open).await
}

/// The configured keys, one per scope.
#[derive(Default)]
struct McpKeys {
    full: Option<String>,
    spark: Option<String>,
    handoff: Option<String>,
    tasks: Option<String>,
}

impl McpKeys {
    fn from_env() -> Self {
        Self {
            full: std::env::var("AGENTHQ_API_KEY").ok(),
            spark: std::env::var("AGENTHQ_SPARK_API_KEY").ok(),
            handoff: std::env::var(HANDOFF_KEY_ENV).ok().filter(|k| !k.trim().is_empty()),
            tasks: std::env::var(TASKS_KEY_ENV).ok().filter(|k| !k.trim().is_empty()),
        }
    }

    fn none_configured(&self) -> bool {
        self.full.is_none()
            && self.spark.is_none()
            && self.handoff.is_none()
            && self.tasks.is_none()
    }
}

async fn handle_mcp(
    state: &WsState,
    headers: &axum::http::HeaderMap,
    body: Value,
    keys: &McpKeys,
    dev_open: bool,
) -> Json<Value> {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let identity = crate::auth::resolve_identity(
        headers,
        keys.full.as_deref(),
        keys.spark.as_deref(),
        keys.handoff.as_deref(),
        keys.tasks.as_deref(),
        dev_open,
    );
    // A launched agent's own token, when none of the configured keys matched.
    let session = match identity {
        Some(_) => None,
        None => session_for_secret(state, crate::auth::presented_secret(headers)),
    };
    if identity.is_none() && session.is_none() {
        if keys.none_configured() {
            tracing::warn!("mcp_http: refused, no AGENTHQ_API_KEY configured");
            return rpc_err(id, -32001, "Unauthorized: this server has no AGENTHQ_API_KEY configured");
        }
        return rpc_err(id, -32001, "Unauthorized");
    }
    let allowed: Option<&[&str]> = match (identity, &session) {
        (Some(crate::auth::ApiIdentity::Full), _) => None,
        (Some(crate::auth::ApiIdentity::Spark), _) => Some(hq_mcp::gateway::SPARK_READONLY_ALLOWLIST),
        (Some(crate::auth::ApiIdentity::Tasks), _) => Some(hq_mcp::gateway::TASKS_ALLOWLIST),
        (Some(crate::auth::ApiIdentity::Handoff), _) => Some(hq_mcp::gateway::HANDOFF_ALLOWLIST),
        (None, _) => Some(hq_mcp::gateway::SESSION_ALLOWLIST),
    };

    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    tracing::debug!(method = %method, "mcp_http: handling json-rpc method");

    match method {
        "initialize" => rpc_ok(id, initialize_result(state.registry.as_deref(), allowed)),
        "notifications/initialized" | "ping" => rpc_ok(id, json!({})),
        "tools/list" => {
            let tools = state
                .registry
                .as_ref()
                .and_then(|r| serde_json::to_value(hq_mcp::gateway::create_gateway_tools_scoped(r, allowed)).ok())
                .unwrap_or(json!([]));
            rpc_ok(id, json!({"tools": tools}))
        }
        "tools/call" => {
            // A proven session is also marked as spawned, so it can never start more.
            let spawned = session.as_deref().or_else(|| spawned_by(headers));
            call_tool(state, id, &body, allowed, spawned, session.as_deref()).await
        }
        _ => rpc_err(id, -32601, "Method not found"),
    }
}

/// The running session a presented secret belongs to, if it is a session token.
fn session_for_secret(state: &WsState, secret: &str) -> Option<String> {
    if secret.is_empty() {
        return None;
    }
    state
        .db
        .with_conn(|c| hq_db::session_tokens::session_for_token(c, secret))
        .ok()
        .flatten()
}

/// Header an MCP client config fills from the `HQ_SESSION_ID` a pane launched by HQ carries.
const SPAWNED_SESSION_HEADER: &str = "x-hq-session-id";

fn spawned_by(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(SPAWNED_SESSION_HEADER)?
        .to_str()
        .ok()
        .filter(|v| !v.trim().is_empty())
}

async fn call_tool(
    state: &WsState,
    id: Value,
    body: &Value,
    allowed: Option<&[&str]>,
    spawned_by: Option<&str>,
    caller_session: Option<&str>,
) -> Json<Value> {
    let params = body.get("params").and_then(|v| v.as_object());
    let tool_name = params.and_then(|p| p.get("name")).and_then(|v| v.as_str()).unwrap_or("");
    let arguments = params.and_then(|p| p.get("arguments")).and_then(|v| v.as_object()).cloned();
    let arguments = match spawned_by {
        Some(session) => hq_mcp::gateway::mark_spawned_session(arguments.as_ref(), session),
        None => arguments,
    };
    let Some(registry) = &state.registry else {
        return rpc_err(id, -32603, "MCP registry not initialized");
    };
    let result =
        hq_mcp::gateway::dispatch_as(
            registry,
            tool_name,
            arguments.as_ref(),
            &state.db,
            allowed,
            caller_session,
        )
        .await;
    match result {
        Ok(call_result) => rpc_ok(id, serde_json::to_value(&call_result).unwrap_or(json!({"content": []}))),
        Err(e) => rpc_err(id, e.code.0.into(), e.message),
    }
}

/// The `initialize` result. Read-only (Spark) callers see only their allowlist
/// through hq_discover, so they get no catalog; the key is omitted rather than
/// null because strict clients (the TS SDK's optional string) reject null.
fn initialize_result(
    registry: Option<&hq_tools::registry::ToolRegistry>,
    allowed: Option<&[&str]>,
) -> Value {
    let mut result = json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "agent-hq", "version": env!("CARGO_PKG_VERSION")}
    });
    if let (Some(registry), None) = (registry, allowed) {
        result["instructions"] = hq_mcp::gateway::server_instructions(registry).into();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_header_marks_a_call_and_a_blank_one_does_not() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(spawned_by(&headers), None);
        headers.insert(SPAWNED_SESSION_HEADER, "".parse().unwrap());
        assert_eq!(
            spawned_by(&headers),
            None,
            "an unset session variable in the client config sends a blank header"
        );
        headers.insert(SPAWNED_SESSION_HEADER, "hs-claude-code-1".parse().unwrap());
        assert_eq!(spawned_by(&headers), Some("hs-claude-code-1"));
    }

    #[test]
    fn initialize_omits_instructions_instead_of_sending_null() {
        let spark: &[&str] = &["vault_read"];
        let registry = hq_tools::registry::ToolRegistry::new();
        assert!(initialize_result(None, None).get("instructions").is_none());
        assert!(initialize_result(Some(&registry), Some(spark)).get("instructions").is_none());
        assert!(initialize_result(Some(&registry), None)["instructions"].is_string());
    }

    use axum::body::Body;
    use axum::http::Request;
    use hq_tools::registry::{HqTool, ToolRegistry};
    use tower::ServiceExt;

    const FULL: &str = "full-key-1111";
    const SPARK: &str = "spark-key-2222";
    const HANDOFF: &str = "handoff-key-3333";
    const TASKS: &str = "tasks-key-4444";

    struct Fake(&'static str, &'static str);

    #[async_trait::async_trait]
    impl HqTool for Fake {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        fn category(&self) -> &str {
            self.1
        }
        async fn execute(&self, _args: Value) -> anyhow::Result<Value> {
            Ok(json!({"ran": self.0}))
        }
    }

    /// `vault_search` is in every scope, `harness_session_logs` only in Spark's,
    /// `task_create` only in handoff's, and `config_manage` in neither.
    fn app() -> axum::Router {
        let mut registry = ToolRegistry::new();
        for (name, category) in [
            ("vault_search", "vault"),
            ("harness_session_logs", "sessions"),
            ("task_create", "tasks"),
            ("config_manage", "config"),
            ("harness_session_spawn", "sessions"),
        ] {
            registry.register(Box::new(Fake(name, category)));
        }
        let vault = tempfile::TempDir::new().unwrap();
        let state = Arc::new(
            WsState::new(vault.path().to_path_buf(), None).with_registry(Arc::new(registry)),
        );
        let keys = Arc::new(McpKeys {
            full: Some(FULL.into()),
            spark: Some(SPARK.into()),
            handoff: Some(HANDOFF.into()),
            tasks: Some(TASKS.into()),
        });
        axum::Router::new().route(
            "/mcp",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                    let (state, keys) = (state.clone(), keys.clone());
                    async move { handle_mcp(&state, &headers, body, &keys, false).await }
                },
            ),
        )
    }

    async fn rpc(app: &axum::Router, key: Option<&str>, body: Value) -> Value {
        let mut req = Request::post("/mcp").header("content-type", "application/json");
        if let Some(key) = key {
            req = req.header("authorization", format!("Bearer {key}"));
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn list() -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
    }

    fn call(tool: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": "hq_call", "arguments": {"tool": tool, "args": {}}}})
    }

    fn discover() -> Value {
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "hq_discover", "arguments": {}}})
    }

    fn names(listing: &Value) -> Vec<String> {
        let text = listing["result"]["content"][0]["text"].as_str().unwrap_or("[]");
        let tools: Vec<Value> = serde_json::from_str(text).unwrap();
        tools.iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }

    #[tokio::test]
    async fn a_request_without_a_key_is_refused_for_every_method() {
        let app = app();
        for body in [list(), call("vault_search"), discover()] {
            let res = rpc(&app, None, body).await;
            assert_eq!(res["error"]["code"], -32001, "{res}");
            assert!(res.get("result").is_none(), "{res}");
        }
        let wrong = rpc(&app, Some("nope"), list()).await;
        assert_eq!(wrong["error"]["code"], -32001);
    }

    #[tokio::test]
    async fn tools_list_is_the_two_gateway_tools_and_names_only_reachable_categories() {
        let app = app();
        let description = |res: &Value| res["result"]["tools"].to_string();
        for key in [FULL, SPARK, HANDOFF, TASKS] {
            let res = rpc(&app, Some(key), list()).await;
            let tools = res["result"]["tools"].as_array().unwrap();
            let listed: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert_eq!(listed, ["hq_discover", "hq_call"], "{key}");
        }
        let full = description(&rpc(&app, Some(FULL), list()).await);
        assert!(full.contains("config") && full.contains("tasks") && full.contains("sessions"), "{full}");
        let spark = description(&rpc(&app, Some(SPARK), list()).await);
        assert!(spark.contains("vault") && spark.contains("sessions"), "{spark}");
        assert!(!spark.contains("config") && !spark.contains("tasks"), "{spark}");
        let handoff = description(&rpc(&app, Some(HANDOFF), list()).await);
        assert!(handoff.contains("tasks") && !handoff.contains("config"), "{handoff}");
        assert!(handoff.contains("sessions"), "handoff can spawn sessions: {handoff}");
        let tasks = description(&rpc(&app, Some(TASKS), list()).await);
        assert!(tasks.contains("tasks") && tasks.contains("vault"), "{tasks}");
        assert!(!tasks.contains("config") && !tasks.contains("sessions"), "{tasks}");
    }

    #[tokio::test]
    async fn discovery_is_filtered_to_the_key_scope() {
        let app = app();
        let full = names(&rpc(&app, Some(FULL), discover()).await);
        assert_eq!(full.len(), 5, "{full:?}");
        let spark = names(&rpc(&app, Some(SPARK), discover()).await);
        assert_eq!(spark, ["harness_session_logs", "vault_search"]);
        let handoff = names(&rpc(&app, Some(HANDOFF), discover()).await);
        assert_eq!(handoff, ["harness_session_spawn", "task_create", "vault_search"]);
        let tasks = names(&rpc(&app, Some(TASKS), discover()).await);
        assert_eq!(tasks, ["task_create", "vault_search"], "no session tools on the tasks key");
    }

    fn call_failed(res: &Value) -> bool {
        res.get("error").is_some() || res["result"]["isError"] == true
    }

    #[tokio::test]
    async fn a_tool_outside_the_key_scope_is_refused_and_one_inside_it_runs() {
        let app = app();
        let cases = [
            (FULL, "config_manage", true),
            (FULL, "task_create", true),
            (SPARK, "vault_search", true),
            (SPARK, "harness_session_logs", true),
            (SPARK, "task_create", false),
            (SPARK, "config_manage", false),
            (HANDOFF, "task_create", true),
            (HANDOFF, "vault_search", true),
            (HANDOFF, "harness_session_logs", false),
            (HANDOFF, "config_manage", false),
            (TASKS, "task_create", true),
            (TASKS, "vault_search", true),
            (TASKS, "harness_session_spawn", false),
            (TASKS, "harness_session_logs", false),
            (TASKS, "config_manage", false),
        ];
        for (key, tool, runs) in cases {
            let res = rpc(&app, Some(key), call(tool)).await;
            if runs {
                assert!(!call_failed(&res), "{key} should run {tool}: {res}");
                assert!(res.to_string().contains(tool), "{res}");
            } else {
                assert!(res["error"]["message"].as_str().unwrap().contains("not permitted"), "{key} {tool}: {res}");
            }
        }
    }

    /// Records the caller session each call arrives with.
    struct WhoAmI;

    #[async_trait::async_trait]
    impl HqTool for WhoAmI {
        fn name(&self) -> &str {
            "task_get"
        }
        fn description(&self) -> &str {
            "reports the attested caller"
        }
        fn parameters(&self) -> Value {
            json!({})
        }
        async fn execute(&self, args: Value) -> anyhow::Result<Value> {
            Ok(json!({"caller": hq_tools::harness_session::caller_session(&args)}))
        }
    }

    /// An app whose database holds one running session and its token.
    fn session_app() -> (axum::Router, String) {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(WhoAmI));
        registry.register(Box::new(Fake("vault_search", "vault")));
        registry.register(Box::new(Fake("config_manage", "config")));
        let vault = tempfile::TempDir::new().unwrap();
        let state = Arc::new(
            WsState::new(vault.path().to_path_buf(), None).with_registry(Arc::new(registry)),
        );
        let token = state
            .db
            .with_conn(|c| {
                hq_db::harness_sessions_registry::insert(
                    c,
                    &hq_db::harness_sessions_registry::NewSession {
                        id: "hs-agent",
                        harness: "claude-code",
                        label: "t",
                        cwd: "/t",
                        mission_id: None,
                        placement: hq_db::harness_sessions_registry::Placement {
                            host: "native",
                            agent_name: "hs-agent",
                            workspace_id: "hs-agent",
                            pane_id: "hs-agent",
                        },
                    },
                )?;
                hq_db::session_tokens::mint(c, "hs-agent")
            })
            .unwrap();
        let keys = Arc::new(McpKeys {
            full: Some(FULL.into()),
            ..McpKeys::default()
        });
        let app = axum::Router::new().route(
            "/mcp",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, Json(body): Json<Value>| {
                    let (state, keys) = (state.clone(), keys.clone());
                    async move { handle_mcp(&state, &headers, body, &keys, false).await }
                },
            ),
        );
        (app, token)
    }

    fn call_with(tool: &str, args: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
               "params": {"name": "hq_call", "arguments": {"tool": tool, "args": args}}})
    }

    fn result_of(res: &Value) -> Value {
        let text = res["result"]["content"][0]["text"].as_str().unwrap_or("null");
        serde_json::from_str(text).unwrap_or(Value::Null)
    }

    #[tokio::test]
    async fn a_session_token_is_attested_and_limited_to_the_session_allowlist() {
        let (app, token) = session_app();
        let ok = rpc(&app, Some(&token), call_with("task_get", json!({}))).await;
        assert_eq!(result_of(&ok)["caller"], "hs-agent", "{ok}");

        for tool in ["vault_search", "config_manage"] {
            let denied = rpc(&app, Some(&token), call_with(tool, json!({}))).await;
            assert!(denied.get("error").is_some(), "{tool}: {denied}");
        }
    }

    #[tokio::test]
    async fn a_session_cannot_pose_as_another_session() {
        let (app, token) = session_app();
        let spoof = call_with("task_get", json!({"_hq_caller_session": "hs-victim"}));
        let res = rpc(&app, Some(&token), spoof).await;
        assert_eq!(result_of(&res)["caller"], "hs-agent", "{res}");
    }

    #[tokio::test]
    async fn keys_are_never_given_an_attested_session_and_bad_tokens_are_refused() {
        let (app, _token) = session_app();
        let spoof = call_with("task_get", json!({"_hq_caller_session": "hs-victim"}));
        let res = rpc(&app, Some(FULL), spoof).await;
        assert_eq!(result_of(&res)["caller"], Value::Null, "{res}");

        for secret in ["hqs_forged", "not-a-token"] {
            let res = rpc(&app, Some(secret), call_with("task_get", json!({}))).await;
            assert_eq!(res["error"]["code"], -32001, "{secret}: {res}");
        }
    }
}
