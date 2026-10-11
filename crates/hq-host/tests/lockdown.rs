//! The kill switch and the start-rate breaker, through real sockets and processes.
#![cfg(unix)]

use hq_host::{BurstLimit, Client, ClientError, Host, Server, clear_lockdown, read_lockdown};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const WAIT_MS: u64 = 10_000;
const BREAKER_MAX: usize = 2;
const BREAKER_WINDOW: Duration = Duration::from_secs(600);

struct Running {
    run: PathBuf,
    host: Arc<Host>,
    stop: hq_host::StopHandle,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    fn start(run: &Path, host: Host) -> Self {
        let host = Arc::new(host);
        let server = Server::bind(run, host.clone()).unwrap();
        let stop = server.stop_handle();
        let thread = std::thread::spawn(move || server.serve());
        Self {
            run: run.to_path_buf(),
            host,
            stop,
            thread: Some(thread),
        }
    }

    fn client(&self) -> Client {
        Client::connect(&self.run).unwrap()
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

fn sleeper(name: &str) -> Value {
    json!({ "name": name, "argv": ["sh", "-c", "sleep 60"], "cwd": std::env::temp_dir(),
            "resume_argv": ["sh", "-c", "sleep 60"] })
}

fn error_code(result: Result<Value, ClientError>) -> String {
    match result {
        Err(ClientError::Remote { code, .. }) => code,
        other => panic!("expected a remote error, got {other:?}"),
    }
}

#[test]
fn lockdown_stops_every_agent_and_refuses_new_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    let host = Running::start(&run, Host::new().with_state_dir(&run));
    let mut c = host.client();
    c.call("agent.spawn", sleeper("a")).unwrap();
    c.call("agent.spawn", sleeper("b")).unwrap();

    let locked = c
        .call("host.lockdown", json!({ "reason": "mass spawn" }))
        .unwrap();
    assert_eq!(locked["lockdown"]["reason"], "mass spawn");
    for name in ["a", "b"] {
        c.call(
            "agent.wait",
            json!({ "name": name, "until": "exit", "timeout_ms": WAIT_MS }),
        )
        .unwrap();
    }
    assert_eq!(
        error_code(c.call("agent.spawn", sleeper("c"))),
        "locked_down"
    );
    let status = c.call("host.status", json!({})).unwrap();
    assert_eq!(status["lockdown"]["source"], "operator");
    assert_eq!(read_lockdown(&run).unwrap().reason, "mass spawn");
}

#[test]
fn there_is_no_method_that_unlocks() {
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    let host = Running::start(&run, Host::new().with_state_dir(&run));
    let mut c = host.client();
    c.call("host.lockdown", json!({ "reason": "test" }))
        .unwrap();
    for method in ["host.unlock", "host.lockdown_clear"] {
        assert_eq!(error_code(c.call(method, json!({}))), "unknown_method");
    }
    assert!(read_lockdown(&run).is_some());
}

#[test]
fn a_locked_host_restores_nothing_after_a_restart_until_unlocked() {
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    {
        let host = Running::start(&run, Host::new().with_state_dir(&run));
        host.client().call("agent.spawn", sleeper("kept")).unwrap();
        host.host.shutdown();
    }
    hq_host::engage_lockdown(&run, &hq_host::Lockdown::new("from the cli", "cli")).unwrap();

    let host = Host::new().with_state_dir(&run);
    let report = host.restore();
    assert!(report.restored.is_empty(), "{report:?}");
    assert!(
        report
            .skipped
            .iter()
            .any(|(name, why)| name == "kept" && why.contains("locked down"))
    );
    assert!(
        host.awaiting().iter().any(|a| a.name == "kept"),
        "the record is kept for later"
    );
    assert!(host.list().is_empty());

    assert!(clear_lockdown(&run).unwrap());
    let info = host.resume("kept", Vec::new()).unwrap();
    assert_eq!(info.name, "kept");
    host.shutdown();
}

#[test]
fn too_many_starts_lock_the_host_without_stopping_what_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    let limit = BurstLimit {
        max: BREAKER_MAX,
        window: BREAKER_WINDOW,
    };
    let host = Running::start(
        &run,
        Host::new().with_state_dir(&run).with_burst_limit(limit),
    );
    let mut c = host.client();
    for n in 0..BREAKER_MAX {
        c.call("agent.spawn", sleeper(&format!("ok{n}"))).unwrap();
    }
    assert_eq!(
        error_code(c.call("agent.spawn", sleeper("over"))),
        "locked_down"
    );
    let held = read_lockdown(&run).unwrap();
    assert_eq!(held.source, "breaker");

    let agents = c.call("agent.list", json!({})).unwrap();
    let running = agents["agents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["status"] == "running")
        .count();
    assert_eq!(
        running, BREAKER_MAX,
        "a tripped breaker freezes starts, it does not kill: {agents}"
    );
    let polled = c
        .call("events.poll", json!({ "after": 0, "timeout_ms": 0 }))
        .unwrap();
    assert!(
        polled["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "lockdown"),
        "{polled}"
    );
    host.host.shutdown();
}
