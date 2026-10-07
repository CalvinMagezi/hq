//! The hook path end to end: a pane runs the hook command Claude Code would
//! run, with a hook payload on stdin, and the host learns the agent's state.

use hq_host::{Client, Host, Server, SpawnSpec, claude_settings};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(15);

fn hq() -> &'static str {
    env!("CARGO_BIN_EXE_hq")
}

fn hook_command() -> String {
    let settings = claude_settings(&format!("'{}' host report", hq()));
    settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_hook_event_in_a_pane_reaches_the_host_through_hq_host_report() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("run");
    let server = Server::bind(&dir, Arc::new(Host::new())).unwrap();
    let stop = server.stop_handle();
    let thread = std::thread::spawn(move || server.serve());

    let payload = r#"{"hook_event_name":"UserPromptSubmit","session_id":"conv-e2e"}"#;
    let script = format!("printf '%s' '{payload}' | {}; sleep 60", hook_command());
    let mut client = Client::connect(&dir).unwrap();
    client
        .call(
            "agent.spawn",
            json!({ "name": "hooked", "argv": ["sh", "-c", script],
                    "cwd": std::env::temp_dir(), "agent": "claude" }),
        )
        .unwrap();

    let deadline = Instant::now() + WAIT;
    let info = loop {
        let info = client
            .call("agent.get", json!({ "name": "hooked" }))
            .unwrap();
        if info["agent_session_id"] == "conv-e2e" || Instant::now() > deadline {
            break info;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    stop.stop();
    thread.join().unwrap();

    assert_eq!(info["agent_session_id"], "conv-e2e", "{info}");
    assert_eq!(info["state"], "working");
    assert_eq!(info["rule"], "hook:UserPromptSubmit");
}

#[test]
fn the_report_command_is_silent_and_succeeds_without_a_host() {
    let out = std::process::Command::new(hq())
        .args(["host", "report"])
        .env_remove("HQ_HOST_TOKEN")
        .env_remove("HQ_HOST_DIR")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "a SessionStart hook's output reaches the model"
    );
}
