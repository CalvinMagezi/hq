//! `NativeBackend` launching an agent over ACP, against a real in-process host. The agent is a
//! shell script that speaks just enough of the protocol (it needs only `sh` and `sed`).
#![cfg(unix)]

use hq_host::{Host, Server, StopHandle};
use hq_tools::agent_host::{AgentStatus, HostBackend, LaunchRequest, NativeBackend, PromptOutcome};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

/// Answers initialize and session/new, and ends every prompt with a message and `end_turn`.
const FAKE_AGENT: &str = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":1,"agentCapabilities":{}}}\n' "$id" ;;
    *'"session/new"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"sessionId":"s-1"}}\n' "$id" ;;
    *'"session/prompt"'*)
      printf '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"ACP-REPLY"}}}}\n'
      printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$id" ;;
  esac
done
"#;

struct Running {
    dir: tempfile::TempDir,
    stop: StopHandle,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::bind(&dir.path().join("run"), Arc::new(Host::new())).unwrap();
        let stop = server.stop_handle();
        let thread = std::thread::spawn(move || server.serve());
        Self {
            dir,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn acp_request(name: &str) -> LaunchRequest {
    LaunchRequest {
        name: name.into(),
        kind: "opencode".into(),
        cwd: std::env::temp_dir().to_string_lossy().into(),
        label: "t".into(),
        env: vec![],
        args: vec![],
        command: None,
        resume_args: None,
        mcp: None,
        start_timeout: WAIT,
        policy: None,
        acp_argv: Some(vec!["sh".into(), "-c".into(), FAKE_AGENT.into()]),
        acp_model: None,
    }
}

#[test]
fn an_acp_agent_launches_ready_takes_a_prompt_and_reads_back() {
    let host = Running::start();
    let b = NativeBackend::new(host.dir.path().join("run"));

    let launched = b.launch(&acp_request("hs-acp")).unwrap();
    assert!(launched.ready);
    let agent = launched.agent.expect("the host lists the agent");
    assert_eq!(agent.status, AgentStatus::Idle);

    let outcome = b.submit("hs-acp", "do it").unwrap();
    assert!(matches!(outcome, PromptOutcome::Submitted), "{outcome:?}");

    let deadline = Instant::now() + WAIT;
    let text = loop {
        let text = b.read("hs-acp", 50).unwrap();
        if text.contains("end: turn done") || Instant::now() > deadline {
            break text;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        text.contains("> do it") && text.contains("ACP-REPLY"),
        "{text}"
    );
    assert!(text.contains("end: turn done"), "{text}");

    b.close_workspace("hs-acp").unwrap();
    assert!(b.agent("hs-acp").unwrap().is_none());
}

#[test]
fn an_acp_agent_that_cannot_start_fails_the_launch_with_its_words() {
    let host = Running::start();
    let b = NativeBackend::new(host.dir.path().join("run"));
    let mut req = acp_request("hs-bad");
    req.acp_argv = Some(vec!["sh".into(), "-c".into(), "exit 3".into()]);
    let err = b.launch(&req).unwrap_err().to_string();
    assert!(
        err.contains("could not start") || err.contains("ended"),
        "{err}"
    );
    assert!(b.agent("hs-bad").unwrap().is_none());
}
