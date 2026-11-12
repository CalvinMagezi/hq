//! What happens to an ACP agent when it is stopped, locked down or flooded with prompts.
#![cfg(unix)]

use hq_host::{AgentState, Host, HostError, SpawnSpec, Transport};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(25);
const FLOOD: usize = 40;

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

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn stopping_an_agent_stops_what_it_started() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let host = Host::new();
    let mut s = spec("life1", "fork");
    s.env.push((
        "FAKE_CHILD_PID_FILE".into(),
        pid_file.to_string_lossy().into(),
    ));
    host.spawn(s).unwrap();
    until("the child's pid", || pid_file.exists());
    let child = std::fs::read_to_string(&pid_file).unwrap();
    assert!(alive(&child), "the child should run while the agent does");
    host.kill("life1").unwrap();
    host.wait_exit("life1", WAIT).unwrap();
    until("the child to stop", || !alive(&child));
}

#[test]
fn a_dead_agent_no_longer_claims_a_state_or_a_dialog() {
    let host = Host::new();
    host.spawn(spec("life2", "permission")).unwrap();
    host.prompt("life2", "write").unwrap();
    let blocked = [AgentState::Blocked];
    host.wait_state("life2", &blocked, Duration::ZERO, WAIT)
        .unwrap();
    host.kill("life2").unwrap();
    host.wait_exit("life2", WAIT).unwrap();
    let info = host.info("life2").unwrap();
    assert!(info.pending.is_none() && info.pending_seq.is_none());
    assert_eq!(info.state, Some(AgentState::Unknown));
    let idle = [AgentState::Idle];
    let waited = host.wait_state("life2", &idle, Duration::ZERO, WAIT);
    assert!(matches!(waited, Err(HostError::Exited(_))), "{waited:?}");
}

#[test]
fn lockdown_stops_acp_agents_too() {
    let host = Host::new();
    host.spawn(spec("life3", "turn")).unwrap();
    host.lockdown("test", "test");
    host.wait_exit("life3", WAIT).unwrap();
    let again = host.spawn(spec("life4", "turn"));
    assert!(matches!(again, Err(HostError::LockedDown(_))));
}

#[test]
fn too_many_waiting_prompts_are_refused() {
    let host = Host::new();
    host.spawn(spec("life5", "cancel")).unwrap();
    host.prompt("life5", "first").unwrap();
    let refused = (0..FLOOD)
        .map(|n| host.prompt("life5", &format!("more {n}")))
        .filter(Result::is_err)
        .count();
    assert!(refused > 0, "the queue should have a limit");
    host.kill("life5").unwrap();
}
