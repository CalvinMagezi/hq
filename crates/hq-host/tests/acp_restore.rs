//! An ACP agent comes back after the host restarts: the stored session is loaded again, what the
//! agent replays is not counted as new work, and an agent that started with extra variables waits
//! for them like a terminal agent does.
#![cfg(unix)]

use hq_host::{Host, ReadSource, SpawnSpec, Transport};
use std::path::Path;
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

/// What a crash leaves behind: the state file, copied before anything is stopped.
fn crash_copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    std::fs::copy(from.join("session.json"), to.join("session.json")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let private = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(to.join("session.json"), private).unwrap();
}

#[test]
fn a_restarted_host_loads_the_stored_session_and_counts_none_of_its_history() {
    let tmp = tempfile::tempdir().unwrap();
    let (before, after) = (tmp.path().join("before"), tmp.path().join("after"));
    let first = Host::new().with_state_dir(&before);
    first.spawn(spec("back1", "turn")).unwrap();
    crash_copy(&before, &after);
    first.kill("back1").unwrap();
    first.wait_exit("back1", WAIT).unwrap();

    let second = Host::new().with_state_dir(&after);
    let report = second.restore();
    assert_eq!(report.restored, vec!["back1".to_string()], "{report:?}");
    let info = second.info("back1").unwrap();
    assert_eq!(info.agent_session_id.as_deref(), Some("fake-1"));
    assert!(
        info.acp.unwrap().touched.is_empty(),
        "the replayed edit is old work"
    );
    let text = second.read("back1", ReadSource::Recent, 0).unwrap();
    assert!(
        text.contains("restored session fake-1") && !text.contains("REPLAYED"),
        "{text}"
    );

    second.prompt("back1", "go on").unwrap();
    until("the next turn", || second.info("back1").unwrap().done);
    second.kill("back1").unwrap();
}

#[test]
fn a_stopped_agent_is_not_brought_back() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("run");
    let host = Host::new().with_state_dir(&dir);
    host.spawn(spec("back2", "turn")).unwrap();
    host.remove("back2").unwrap();
    let again = Host::new().with_state_dir(&dir);
    assert!(again.restore().restored.is_empty());
}

#[test]
fn an_agent_started_with_variables_waits_for_them_and_never_stores_their_values() {
    let tmp = tempfile::tempdir().unwrap();
    let (before, after) = (tmp.path().join("before"), tmp.path().join("after"));
    let first = Host::new().with_state_dir(&before);
    let mut s = spec("back3", "turn");
    s.env.push(("HQ_TEST_SECRET".into(), "hunter2".into()));
    first.spawn(s).unwrap();
    crash_copy(&before, &after);
    first.kill("back3").unwrap();
    let stored = std::fs::read_to_string(after.join("session.json")).unwrap();
    assert!(stored.contains("HQ_TEST_SECRET") && !stored.contains("hunter2"));

    let second = Host::new().with_state_dir(&after);
    assert!(second.restore().restored.is_empty());
    assert_eq!(second.awaiting().len(), 1);
    let env = vec![("HQ_TEST_SECRET".to_string(), "hunter2".to_string())];
    let info = second.resume("back3", env).unwrap();
    assert_eq!(info.agent_session_id.as_deref(), Some("fake-1"));
    second.kill("back3").unwrap();
}

#[test]
fn a_session_the_agent_no_longer_has_fails_the_restore_with_its_words() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("run");
    std::fs::create_dir_all(&dir).unwrap();
    let bin = env!("CARGO_BIN_EXE_hq-host-fake-acp-agent");
    let record = serde_json::json!({"version": 1, "panes": [{
        "name": "back4", "resume_argv": [bin, "turn"], "agent": "fake",
        "cwd": std::env::temp_dir(), "env_keys": [], "rows": 40, "cols": 120,
        "scrollback_rows": 0, "acp_session": "gone-session"}]});
    std::fs::write(dir.join("session.json"), record.to_string()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        dir.join("session.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let host = Host::new().with_state_dir(&dir);
    let report = host.restore();
    assert!(report.restored.is_empty());
    let why = &report.skipped.first().expect("skipped").1;
    assert!(why.contains("session not found"), "{why}");
}
