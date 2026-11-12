//! An agent driven over ACP, through the same host calls a terminal agent answers. The agent is a
//! scripted process (crates/hq-acp/tests/support/fake_agent.rs).
#![cfg(unix)]

use hq_host::{
    AgentState, Host, HostError, PaneInfo, PaneStatus, ReadSource, SpawnSpec, Transport,
};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(25);

fn spec(name: &str, scenario: &str) -> SpawnSpec {
    let bin = env!("CARGO_BIN_EXE_hq-host-fake-acp-agent");
    let mut spec = SpawnSpec::new(
        name,
        vec![bin.into(), scenario.into()],
        std::env::temp_dir(),
    );
    spec.transport = Transport::Acp;
    spec.agent = Some("fake".into());
    spec
}

fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

fn blocked(host: &Host, name: &str) -> PaneInfo {
    let states = [AgentState::Blocked];
    host.wait_state(name, &states, Duration::ZERO, WAIT)
        .expect("blocks at a dialog")
}

fn done(host: &Host, name: &str) -> PaneInfo {
    until("the turn to finish", || {
        host.info(name).is_ok_and(|i| i.done)
    });
    host.info(name).unwrap()
}

#[test]
fn a_started_agent_reports_what_the_protocol_told_it() {
    let host = Host::new();
    let info = host.spawn(spec("acp1", "turn")).unwrap();
    assert_eq!(info.state, Some(AgentState::Idle));
    assert_eq!(info.rule.as_deref(), Some("acp"));
    assert_eq!(info.agent_session_id.as_deref(), Some("fake-1"));
    assert!(!info.done && !info.resumable);
    assert_eq!(host.list().len(), 1);
    host.kill("acp1").unwrap();
}

#[test]
fn a_turn_ends_done_and_its_text_can_be_read() {
    let host = Host::new();
    host.spawn(spec("acp2", "turn")).unwrap();
    host.prompt("acp2", "go").unwrap();
    let info = done(&host, "acp2");
    assert_eq!(info.state, Some(AgentState::Idle));
    assert!(info.state_seq > 0);
    let text = host.read("acp2", ReadSource::Recent, 0).unwrap();
    assert!(
        text.contains("> go") && text.contains("DONE") && text.contains("end: turn done"),
        "{text}"
    );
    host.kill("acp2").unwrap();
}

#[test]
fn a_dialog_is_read_as_data_and_answered_by_number() {
    let host = Host::new();
    host.spawn(spec("acp3", "permission")).unwrap();
    host.prompt("acp3", "write").unwrap();
    let info = blocked(&host, "acp3");
    let (dialog, seq) = (
        info.pending.expect("dialog"),
        info.pending_seq.expect("number"),
    );
    assert_eq!(dialog.options.len(), 2);
    assert!(
        dialog
            .question
            .as_deref()
            .is_some_and(|q| q.contains("Write a.txt"))
    );
    assert!(matches!(
        host.prompt("acp3", "x"),
        Err(HostError::Blocked(_))
    ));
    assert!(matches!(
        host.answer("acp3", seq + 1, Some(1)),
        Err(HostError::StaleDialog { .. })
    ));
    assert!(matches!(
        host.answer("acp3", seq, Some(9)),
        Err(HostError::NoSuchOption(9))
    ));
    let answered = host.answer("acp3", seq, Some(1)).unwrap();
    assert!(answered.registered);
    done(&host, "acp3");
    let text = host.read("acp3", ReadSource::Recent, 0).unwrap();
    assert!(
        text.contains("tool: Write a.txt") && text.contains("ask: Write a.txt"),
        "{text}"
    );
    host.kill("acp3").unwrap();
}

#[test]
fn declining_a_dialog_ends_the_turn_without_the_edit() {
    let host = Host::new();
    host.spawn(spec("acp4", "permission")).unwrap();
    host.prompt("acp4", "write").unwrap();
    let seq = blocked(&host, "acp4").pending_seq.unwrap();
    host.answer("acp4", seq, None).unwrap();
    done(&host, "acp4");
    let text = host.read("acp4", ReadSource::Recent, 0).unwrap();
    assert!(
        text.contains("end: cancelled") && !text.contains("DONE"),
        "{text}"
    );
    host.kill("acp4").unwrap();
}

#[test]
fn esc_stops_a_turn_and_other_raw_input_is_refused() {
    let host = Host::new();
    host.spawn(spec("acp5", "cancel")).unwrap();
    host.prompt("acp5", "loop").unwrap();
    until("the agent to start", || {
        host.read("acp5", ReadSource::Recent, 0)
            .unwrap()
            .contains("working")
    });
    assert!(matches!(
        host.send_text("acp5", "hi"),
        Err(HostError::Unsupported(_))
    ));
    assert!(matches!(
        host.paste("acp5", "hi"),
        Err(HostError::Unsupported(_))
    ));
    assert!(matches!(
        host.send_keys("acp5", &["enter".into()]),
        Err(HostError::Unsupported(_))
    ));
    host.send_keys("acp5", &["esc".into()]).unwrap();
    done(&host, "acp5");
    let text = host.read("acp5", ReadSource::Recent, 0).unwrap();
    assert!(text.contains("end: cancelled"), "{text}");
    host.kill("acp5").unwrap();
}

#[test]
fn a_second_prompt_waits_for_the_first_turn() {
    let host = Host::new();
    host.spawn(spec("acp6", "turn")).unwrap();
    host.prompt("acp6", "one").unwrap();
    host.prompt("acp6", "two").unwrap();
    until("both turns", || {
        host.read("acp6", ReadSource::Recent, 0)
            .unwrap()
            .matches("end: turn done")
            .count()
            == 2
    });
    let text = host.read("acp6", ReadSource::Recent, 0).unwrap();
    assert!(
        text.find("> one").unwrap() < text.find("> two").unwrap(),
        "{text}"
    );
    host.kill("acp6").unwrap();
}

#[test]
fn read_since_hands_back_only_new_lines() {
    let host = Host::new();
    host.spawn(spec("acp7", "turn")).unwrap();
    let start = host.read_since("acp7", None, 100).unwrap();
    assert!(start.lines.is_empty());
    host.prompt("acp7", "go").unwrap();
    done(&host, "acp7");
    let next = host.read_since("acp7", Some(&start.next), 100).unwrap();
    assert!(!next.reset);
    assert!(
        next.lines.iter().any(|l| l == "DONE") && next.lines.iter().any(|l| l.starts_with("end:"))
    );
    let again = host.read_since("acp7", Some(&next.next), 100).unwrap();
    assert!(again.lines.is_empty());
    host.kill("acp7").unwrap();
}

#[test]
fn a_missing_login_fails_the_start_with_the_agents_words() {
    let host = Host::new();
    let err = host.spawn(spec("acp8", "auth")).unwrap_err();
    assert!(
        matches!(&err, HostError::Spawn { detail, .. } if detail.contains("Authentication required")),
        "{err}"
    );
    assert!(host.list().is_empty());
    // The name is free again.
    host.spawn(spec("acp8", "turn")).unwrap();
    host.kill("acp8").unwrap();
}

#[test]
fn killing_ends_the_process_and_removing_forgets_it() {
    let host = Host::new();
    host.spawn(spec("acp9", "turn")).unwrap();
    let pid = host.info("acp9").unwrap().pid.unwrap();
    host.kill("acp9").unwrap();
    host.wait_exit("acp9", WAIT).unwrap();
    assert!(matches!(
        host.info("acp9").unwrap().status,
        PaneStatus::Exited { .. }
    ));
    assert!(matches!(
        host.prompt("acp9", "x"),
        Err(HostError::Exited(_))
    ));
    assert!(
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .is_ok_and(|o| !o.status.success()),
        "process {pid} is still alive"
    );
    host.remove("acp9").unwrap();
    assert!(matches!(host.info("acp9"), Err(HostError::NotFound(_))));
}

#[test]
fn names_are_shared_with_terminal_agents() {
    let host = Host::new();
    host.spawn(spec("same", "turn")).unwrap();
    let pty = SpawnSpec::new(
        "same",
        vec!["sh".into(), "-c".into(), "sleep 5".into()],
        std::env::temp_dir(),
    );
    assert!(matches!(host.spawn(pty), Err(HostError::NameTaken(_))));
    host.kill("same").unwrap();
}
