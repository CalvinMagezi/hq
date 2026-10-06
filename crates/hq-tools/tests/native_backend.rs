//! `NativeBackend` against a real in-process host and real processes.

use hq_host::{Host, Server, StopHandle};
use hq_tools::herdr::{
    AgentStatus, HerdrError, HostBackend, LaunchRequest, NativeBackend, PromptOutcome,
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
        command: Some("cat".into()),
        resume_args: None,
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
    assert!(matches!(b.version(), Err(HerdrError::Unreachable { .. })));
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
        Err(HerdrError::Api { code, .. }) => assert_eq!(code, "name_taken"),
        other => panic!("expected name_taken, got {other:?}"),
    }
}

#[test]
fn invalid_keys_are_refused_before_they_reach_the_host() {
    let host = Running::start();
    let b = host.backend();
    b.launch(&cat("hs-keys", "pi")).unwrap();
    let err = b.send_keys("hs-keys", &["--help".to_string()]).unwrap_err();
    assert!(matches!(err, HerdrError::Api { ref code, .. } if code == "invalid_keys"));
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
    assert!(matches!(err, HerdrError::Api { ref code, .. } if code == "timeout"));
}
