//! `NativeBackend` against a real in-process host and real processes.

use hq_host::{Host, Server, StopHandle};
use hq_tools::agent_host::{
    AgentStatus, AgentHostError, HostBackend, LaunchRequest, NativeBackend, PromptOutcome,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

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

    fn backend(&self) -> NativeBackend {
        NativeBackend::new(self.dir.path().join("run"))
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

/// `cat` run as an agent of `kind`: it echoes what it is sent.
fn cat(name: &str, kind: &str) -> LaunchRequest {
    LaunchRequest {
        name: name.into(),
        kind: kind.into(),
        cwd: std::env::temp_dir().to_string_lossy().into(),
        label: "t".into(),
        env: vec![("HQ_TEST".into(), "1".into())],
        args: vec![],
        // A function ignores the extra flags (such as --settings) a launch adds.
        command: Some("f() { echo started; cat; }; f".into()),
        resume_args: None,
        mcp: None,
        start_timeout: WAIT,
    }
}

fn read_until(b: &NativeBackend, name: &str, needle: &str) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        let text = b.read(name, 50).unwrap();
        if text.contains(needle) || Instant::now() > deadline {
            return text;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_unreachable_host_is_reported_as_unreachable() {
    let b = NativeBackend::new("/nonexistent/hq-host-run");
    assert!(matches!(b.version(), Err(AgentHostError::Unreachable { .. })));
}

#[test]
fn launch_send_read_and_close_round_trip() {
    let host = Running::start();
    let b = host.backend();
    assert!(!b.version().unwrap().is_empty());

    let launched = b.launch(&cat("hs-one", "pi")).unwrap();
    assert!(
        launched.ready,
        "a kind without rules has no state to wait for"
    );
    assert_eq!(launched.pane_id, "hs-one");
    assert_eq!(b.agents().unwrap().len(), 1);
    assert!(b.shell_pid("hs-one").is_some());

    b.send_text("hs-one", "hello-native").unwrap();
    let text = read_until(&b, "hs-one", "hello-native");
    assert!(text.contains("hello-native"), "screen was: {text}");
    assert_eq!(b.read_sourced("hs-one", 50).unwrap().1, "recent-unwrapped");

    b.close_workspace("hs-one").unwrap();
    assert!(b.agent("hs-one").unwrap().is_none());
    b.close_workspace("hs-one").unwrap();
}

#[test]
fn a_taken_name_is_an_api_error() {
    let host = Running::start();
    let b = host.backend();
    b.launch(&cat("hs-dup", "pi")).unwrap();
    match b.launch(&cat("hs-dup", "pi")) {
        Err(AgentHostError::Api { code, .. }) => assert_eq!(code, "name_taken"),
        other => panic!("expected name_taken, got {other:?}"),
    }
}

#[test]
fn invalid_keys_are_refused_before_they_reach_the_host() {
    let host = Running::start();
    let b = host.backend();
    b.launch(&cat("hs-keys", "pi")).unwrap();
    let err = b.send_keys("hs-keys", &["--help".to_string()]).unwrap_err();
    assert!(matches!(err, AgentHostError::Api { ref code, .. } if code == "invalid_keys"));
    b.send_keys("hs-keys", &["ctrl+c".to_string()]).unwrap();
}

#[test]
fn a_known_agent_settles_and_a_silent_submit_retries_enter() {
    let host = Running::start();
    let b = host.backend();
    let launched = b.launch(&cat("hs-claude", "claude")).unwrap();
    assert!(
        launched.ready,
        "a known agent with no matching rule is idle"
    );

    match b.prompt("hs-claude", "ping", Some(WAIT)).unwrap() {
        PromptOutcome::Settled(info) => assert_eq!(info.status, AgentStatus::Idle),
        other => panic!("expected Settled, got {other:?}"),
    }
    // `cat` never shows working or blocked, so submit sees no reaction.
    assert!(matches!(
        b.submit("hs-claude", "again").unwrap(),
        PromptOutcome::Resubmitted
    ));
}

#[test]
fn waiting_for_a_state_that_never_comes_times_out() {
    let host = Running::start();
    let b = host.backend();
    b.launch(&cat("hs-wait", "claude")).unwrap();
    let err = b
        .wait(
            "hs-wait",
            &[AgentStatus::Blocked],
            Duration::from_millis(900),
        )
        .unwrap_err();
    assert!(matches!(err, AgentHostError::Api { ref code, .. } if code == "timeout"));
}

#[test]
fn a_claude_launch_gets_private_hook_settings_and_other_kinds_do_not() {
    use std::os::unix::fs::PermissionsExt;
    let host = Running::start();
    let b = host.backend();
    b.launch(&cat("hs-hooked", "claude")).unwrap();
    b.launch(&cat("hs-plain", "pi")).unwrap();

    let hooks = host.dir.path().join("run/hooks");
    let file = hooks.join("hs-hooked.json");
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("host report") && text.contains("UserPromptSubmit"),
        "{text}"
    );
    assert!(!hooks.join("hs-plain.json").exists());
}

#[test]
fn launch_is_not_ready_before_the_agent_has_drawn_its_screen() {
    let host = Running::start();
    let b = host.backend();
    let mut req = cat("hs-slow", "claude");
    // Draws nothing for a while, as a CLI does while it starts up.
    req.command = Some("f() { sleep 2; echo drawn-screen; cat; }; f".into());
    let began = Instant::now();
    let launched = b.launch(&req).unwrap();
    let text = b.read("hs-slow", 20).unwrap();
    assert!(launched.ready);
    assert!(
        began.elapsed() >= Duration::from_millis(1900) && text.contains("drawn-screen"),
        "returned after {:?} with screen {text:?}",
        began.elapsed()
    );
}

#[test]
fn a_finished_turn_reads_as_done_with_a_rising_counter_and_an_event() {
    use hq_host::Client;
    let host = Running::start();
    let b = host.backend();
    b.launch(&cat("hs-turn", "claude")).unwrap();
    let mut op = Client::connect(&host.dir.path().join("run")).unwrap();
    let report = |op: &mut Client, event: &str| {
        op.call(
            "agent.report",
            serde_json::json!({ "name": "hs-turn", "event": event }),
        )
        .unwrap();
    };

    let cursor = b.poll_events(None, Duration::ZERO).unwrap().last_seq;
    report(&mut op, "UserPromptSubmit");
    let working = b.agent("hs-turn").unwrap().unwrap();
    assert_eq!(working.status, AgentStatus::Working);

    report(&mut op, "Stop");
    let done = b.agent("hs-turn").unwrap().unwrap();
    assert_eq!(done.status, AgentStatus::Done);
    assert!(
        done.state_change_seq > working.state_change_seq,
        "the supervisor alerts once per counter value"
    );

    let heard = b.poll_events(Some(cursor), Duration::from_secs(2)).unwrap();
    assert!(
        heard.events.iter().any(|e| e.name == "hs-turn"
            && e.kind == "state"
            && e.state.as_deref() == Some("idle")),
        "{heard:?}"
    );
    assert!(!heard.lost);
}

#[test]
fn a_wrapper_command_receives_the_hook_flag_not_just_the_shell() {
    let host = Running::start();
    let b = host.backend();
    let mut req = cat("hs-wrap", "claude");
    req.command = Some("f() { echo \"flags: $*\"; cat; }; f".into());
    b.launch(&req).unwrap();
    let text = read_until(&b, "hs-wrap", "flags:");
    assert!(
        text.contains("--settings") && text.contains("hs-wrap.json"),
        "the wrapper's own arguments were: {text}"
    );
}

#[test]
fn a_launch_with_hq_access_gets_a_private_mcp_config_and_flag() {
    use hq_tools::agent_host::McpAccess;
    use std::os::unix::fs::PermissionsExt;
    let host = Running::start();
    let b = host.backend();
    let mut req = cat("hs-mcp", "claude");
    req.command = Some("f() { echo \"flags: $*\"; cat; }; f".into());
    req.mcp = Some(McpAccess {
        url: "https://hq.example/mcp".into(),
        token: "hqs_secret123".into(),
    });
    assert!(
        !format!("{req:?}").contains("hqs_secret123"),
        "the token must not reach logs"
    );
    b.launch(&req).unwrap();

    let text = read_until(&b, "hs-mcp", "flags:");
    assert!(
        text.contains("--mcp-config") && text.contains("--settings"),
        "{text}"
    );
    assert!(
        !text.contains("hqs_secret123"),
        "the token is in the file, not the command line"
    );
    let file = host.dir.path().join("run/mcp/hs-mcp.json");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let cfg = std::fs::read_to_string(&file).unwrap();
    assert!(
        cfg.contains("Bearer hqs_secret123") && cfg.contains("https://hq.example/mcp"),
        "{cfg}"
    );

    // Another kind never gets one.
    let mut other = cat("hs-nomcp", "pi");
    other.mcp = req.mcp.clone();
    b.launch(&other).unwrap();
    assert!(!host.dir.path().join("run/mcp/hs-nomcp.json").exists());
}
