//! A connection owns the agent's whole process group.
#![cfg(unix)]

use hq_acp::Connection;
use std::process::Command;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(25);

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn dropping_a_connection_stops_what_the_agent_started() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fake-acp-agent"));
    cmd.arg("fork").env("FAKE_CHILD_PID_FILE", &pid_file);
    let mut c = Connection::spawn(cmd).expect("fake agent starts");
    c.initialize("hq-test", WAIT).expect("initialize");
    c.new_session("/work", serde_json::json!([]), WAIT)
        .expect("session");
    let child = std::fs::read_to_string(&pid_file).expect("the agent wrote its child's pid");
    assert!(alive(&child));
    drop(c);
    let deadline = Instant::now() + WAIT;
    while alive(&child) {
        assert!(
            Instant::now() < deadline,
            "the child outlived the connection"
        );
        std::thread::sleep(TICK);
    }
}

#[test]
fn stop_group_reports_a_signal_exit_as_128_plus_the_signal() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fake-acp-agent"));
    cmd.arg("turn");
    let mut c = Connection::spawn(cmd).expect("fake agent starts");
    c.initialize("hq-test", WAIT).expect("initialize");
    let code = c.stop_group(Duration::from_secs(2));
    assert_eq!(code, 128 + 15);
}
