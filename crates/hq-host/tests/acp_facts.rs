//! What an ACP agent reports about a turn reaches the host's info, for the supervisor to record.
#![cfg(unix)]

use hq_host::{Host, SpawnSpec, Transport};
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
    spec
}

fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

#[test]
fn a_finished_turn_reports_its_edits_tokens_and_context() {
    let host = Host::new();
    host.spawn(spec("fact1", "permission")).unwrap();
    assert_eq!(
        host.info("fact1").unwrap().acp.unwrap().touched,
        Vec::<String>::new()
    );
    host.prompt("fact1", "write").unwrap();
    until("the dialog", || {
        host.info("fact1").unwrap().pending_seq.is_some()
    });
    let seq = host.info("fact1").unwrap().pending_seq.unwrap();
    host.answer("fact1", seq, Some(1)).unwrap();
    until("the turn", || host.info("fact1").unwrap().done);
    let facts = host.info("fact1").unwrap().acp.unwrap();
    assert_eq!(facts.touched, vec!["/work/a.txt"]);
    assert_eq!(facts.stop_reason.as_deref(), Some("turn done"));
    assert_eq!(facts.turn_tokens, Some(7));
    host.kill("fact1").unwrap();
}

#[test]
fn a_declined_edit_is_not_reported_and_a_terminal_agent_has_no_facts() {
    let host = Host::new();
    host.spawn(spec("fact2", "permission")).unwrap();
    host.prompt("fact2", "write").unwrap();
    until("the dialog", || {
        host.info("fact2").unwrap().pending_seq.is_some()
    });
    let seq = host.info("fact2").unwrap().pending_seq.unwrap();
    host.answer("fact2", seq, None).unwrap();
    until("the turn", || host.info("fact2").unwrap().done);
    let facts = host.info("fact2").unwrap().acp.unwrap();
    assert!(facts.touched.is_empty());
    assert_eq!(facts.stop_reason.as_deref(), Some("cancelled"));
    host.kill("fact2").unwrap();

    let pty = SpawnSpec::new(
        "fact3",
        vec!["sh".into(), "-c".into(), "sleep 5".into()],
        std::env::temp_dir(),
    );
    host.spawn(pty).unwrap();
    assert!(host.info("fact3").unwrap().acp.is_none());
    host.kill("fact3").unwrap();
}
