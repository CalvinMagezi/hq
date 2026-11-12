//! The client against a scripted agent process: handshake, a turn, a dialog, cancel, failures.

use hq_acp::{AgentState, Choice, ClientError, Connection, FailureKind, StopReason, TurnEnd};
use std::process::Command;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(50);

fn start(scenario: &str) -> Connection {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fake-acp-agent"));
    cmd.arg(scenario);
    let mut c = Connection::spawn(cmd).expect("fake agent starts");
    let caps = c.initialize("hq-test", WAIT).expect("initialize");
    assert!(caps.load_session && caps.resume && !caps.fork);
    c
}

fn pump_until(c: &mut Connection, done: impl Fn(&Connection) -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !done(c) {
        assert!(std::time::Instant::now() < deadline, "timed out");
        c.pump(TICK).expect("agent alive");
    }
}

#[test]
fn a_plain_turn_ends_done_with_text_and_usage() {
    let mut c = start("turn");
    c.new_session("/work", serde_json::json!([]), WAIT)
        .expect("session");
    c.prompt("go").expect("prompt");
    assert_eq!(c.tracker.state(), AgentState::Working);
    pump_until(&mut c, |c| c.tracker.done());
    assert_eq!(c.tracker.text(), "DONE");
    assert_eq!(c.tracker.context_use(), (Some(100), Some(1000)));
    assert!(
        matches!(c.tracker.end(), Some(TurnEnd::Stopped { reason: StopReason::EndTurn, usage }) if usage.total == Some(7))
    );
}

#[test]
fn a_dialog_blocks_until_answered_then_the_edit_counts() {
    let mut c = start("permission");
    c.new_session("/work", serde_json::json!([]), WAIT)
        .expect("session");
    c.prompt("write").expect("prompt");
    pump_until(&mut c, |c| c.tracker.state() == AgentState::Blocked);
    let seq = c.tracker.pending().expect("dialog").seq;
    c.answer(seq, &Choice::Option("yes".into()))
        .expect("answer");
    pump_until(&mut c, |c| c.tracker.done());
    assert_eq!(
        c.tracker.touched().into_iter().collect::<Vec<_>>(),
        vec!["/work/a.txt"]
    );
}

#[test]
fn denying_a_dialog_leaves_the_edit_uncounted() {
    let mut c = start("permission");
    c.new_session("/work", serde_json::json!([]), WAIT)
        .expect("session");
    c.prompt("write").expect("prompt");
    pump_until(&mut c, |c| c.tracker.state() == AgentState::Blocked);
    let seq = c.tracker.pending().expect("dialog").seq;
    c.answer(seq, &Choice::Option("no".into())).expect("answer");
    pump_until(&mut c, |c| c.tracker.done());
    assert!(c.tracker.touched().is_empty());
}

#[test]
fn cancel_ends_the_turn_as_cancelled() {
    let mut c = start("cancel");
    c.new_session("/work", serde_json::json!([]), WAIT)
        .expect("session");
    c.prompt("loop").expect("prompt");
    pump_until(&mut c, |c| c.tracker.text() == "working");
    c.cancel().expect("cancel");
    pump_until(&mut c, |c| c.tracker.done());
    assert!(matches!(
        c.tracker.end(),
        Some(TurnEnd::Stopped {
            reason: StopReason::Cancelled,
            ..
        })
    ));
}

#[test]
fn a_provider_error_is_a_classified_failure() {
    let mut c = start("error");
    c.new_session("/work", serde_json::json!([]), WAIT)
        .expect("session");
    c.prompt("x").expect("prompt");
    pump_until(&mut c, |c| c.tracker.done());
    assert!(matches!(
        c.tracker.end(),
        Some(TurnEnd::Failed {
            kind: FailureKind::Billing,
            ..
        })
    ));
}

#[test]
fn a_login_error_comes_back_from_new_session() {
    let mut c = start("auth");
    match c.new_session("/work", serde_json::json!([]), WAIT) {
        Err(ClientError::Rpc(e)) => assert_eq!(e.code, -32000),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn banner_lines_before_json_are_ignored() {
    let mut c = start("noise");
    c.new_session("/work", serde_json::json!([]), WAIT)
        .expect("session");
}

#[test]
fn a_dead_process_reports_exited() {
    let mut cmd = Command::new("true");
    cmd.arg("ignored");
    let mut c = Connection::spawn(cmd).expect("spawns");
    assert!(matches!(
        c.initialize("hq-test", WAIT),
        Err(ClientError::Exited)
    ));
}
