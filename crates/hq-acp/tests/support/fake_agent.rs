//! A scripted ACP agent for tests. The first argument picks the scenario:
//! `turn`, `permission`, `cancel`, `error`, `auth`, `noise`.

use serde_json::{Value, json};
use std::io::{BufRead, Write, stdin, stdout};

const PERMISSION_REQUEST_ID: u64 = 900;

fn send(v: Value) {
    let mut out = stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

fn update(update: Value) {
    send(
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "fake-1", "update": update}}),
    );
}

fn chunk(text: &str) {
    update(
        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}),
    );
}

fn reply(id: &Value, result: Value) {
    send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
}

fn fail(id: &Value, code: i64, message: &str) {
    send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
}

fn next_line(lines: &mut impl Iterator<Item = String>) -> Option<Value> {
    lines.next().and_then(|l| serde_json::from_str(&l).ok())
}

fn prompt_turn(
    scenario: &str,
    mcp_seen: &str,
    id: &Value,
    lines: &mut impl Iterator<Item = String>,
) {
    let end_turn = json!({"stopReason": "end_turn", "usage": {"inputTokens": 5, "outputTokens": 2, "totalTokens": 7}});
    match scenario {
        "mcp" | "mcp-no-http" => {
            chunk(&format!("MCP:{mcp_seen}"));
            reply(id, end_turn);
        }
        "error" => fail(
            id,
            -32603,
            "Your current subscription does not have access to this model",
        ),
        "cancel" => {
            chunk("working");
            while let Some(m) = next_line(lines) {
                if m["method"] == "session/cancel" {
                    return reply(id, json!({"stopReason": "cancelled"}));
                }
            }
        }
        "permission" => {
            update(
                json!({"sessionUpdate": "tool_call", "toolCallId": "t1", "kind": "edit", "title": "Write a.txt",
                "locations": [{"path": "/work/a.txt"}]}),
            );
            send(
                json!({"jsonrpc": "2.0", "id": PERMISSION_REQUEST_ID, "method": "session/request_permission",
                "params": {"sessionId": "fake-1", "toolCall": {"toolCallId": "t1", "title": "Write a.txt", "kind": "edit"},
                    "options": [{"optionId": "yes", "name": "Allow", "kind": "allow_once"},
                                {"optionId": "no", "name": "Deny", "kind": "reject_once"}]}}),
            );
            let answer = next_line(lines).unwrap_or(Value::Null);
            let picked = answer
                .pointer("/result/outcome/optionId")
                .and_then(Value::as_str);
            if picked == Some("yes") {
                update(
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"}),
                );
                chunk("DONE");
                reply(id, end_turn);
            } else {
                update(
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "failed"}),
                );
                reply(id, json!({"stopReason": "cancelled"}));
            }
        }
        _ => {
            chunk("DO");
            chunk("NE");
            update(json!({"sessionUpdate": "usage_update", "size": 1000, "used": 100}));
            reply(id, end_turn);
        }
    }
}

/// Starts a long sleep that ignores nothing but outlives the agent unless the whole process group
/// is stopped, and writes its pid to the file named by FAKE_CHILD_PID_FILE.
fn leave_a_child_behind() {
    let child = std::process::Command::new("sleep").arg("300").spawn();
    if let (Ok(child), Ok(path)) = (child, std::env::var("FAKE_CHILD_PID_FILE")) {
        let _ = std::fs::write(path, child.id().to_string());
    }
}

fn main() {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "turn".into());
    if scenario == "noise" {
        println!("welcome to a chatty agent");
    }
    let mut lines = stdin().lock().lines().map_while(Result::ok);
    let mut mcp_seen = String::new();
    while let Some(m) = next_line(&mut lines) {
        let id = m.get("id").cloned().unwrap_or(Value::Null);
        match m["method"].as_str() {
            Some("initialize") => reply(
                &id,
                json!({"protocolVersion": 1,
                "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"resume": {}},
                    "mcpCapabilities": {"http": scenario == "mcp"}},
                "agentInfo": {"name": "fake", "version": "0"}}),
            ),
            Some("session/new") if scenario == "auth" => {
                fail(&id, -32000, "Authentication required")
            }
            Some("session/new") => {
                if scenario == "fork" {
                    leave_a_child_behind();
                }
                mcp_seen = m["params"]["mcpServers"].to_string();
                reply(&id, json!({"sessionId": "fake-1"}))
            }
            Some("session/prompt") => prompt_turn(&scenario, &mcp_seen, &id, &mut lines),
            _ => {}
        }
    }
}
