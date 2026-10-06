//! Restart behavior: what the host brings back, and what it must not.

use hq_host::{Host, ReadSource, SpawnSpec};
use std::path::Path;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

fn resumable(name: &str, first: &str, again: &str) -> SpawnSpec {
    let sh = |script: &str| vec!["sh".to_string(), "-c".to_string(), script.to_string()];
    let mut spec = SpawnSpec::new(name, sh(first), std::env::temp_dir());
    spec.resume_argv = Some(sh(again));
    spec
}

fn wait_for(host: &Host, name: &str, want: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        let text = host.read(name, ReadSource::Visible, 0).unwrap_or_default();
        if text.contains(want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{name} never showed {want:?}; screen:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn state_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("run");
    (tmp, dir)
}

fn copy_state(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    std::fs::copy(from.join("session.json"), to.join("session.json")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        to.join("session.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
}

#[test]
fn a_crashed_host_is_brought_back_with_the_resume_command_not_the_original() {
    let (_t1, dir1) = state_dir();
    let first = Host::new().with_state_dir(&dir1);
    first
        .spawn(resumable(
            "alive",
            "echo first-run; sleep 60",
            "echo resumed-run; sleep 60",
        ))
        .unwrap();
    wait_for(&first, "alive", "first-run");

    // What a crash leaves behind: the state file as it is while the agent runs.
    let (_t2, dir2) = state_dir();
    copy_state(&dir1, &dir2);

    let second = Host::new().with_state_dir(&dir2);
    let report = second.restore();
    assert_eq!(report.restored, ["alive"], "{:?}", report.skipped);
    wait_for(&second, "alive", "resumed-run");
    assert!(
        !second
            .read("alive", ReadSource::Visible, 0)
            .unwrap()
            .contains("first-run")
    );
    assert!(second.info("alive").unwrap().resumable);
}

#[test]
fn a_clean_shutdown_stops_the_agents_but_keeps_them_for_the_next_start() {
    let (_t, dir) = state_dir();
    let host = Host::new().with_state_dir(&dir);
    host.spawn(resumable("keep", "sleep 60", "echo back-again; sleep 60"))
        .unwrap();
    host.shutdown();
    host.wait_exit("keep", WAIT).unwrap();
    std::thread::sleep(Duration::from_millis(300)); // let the exit hook run and be ignored

    let next = Host::new().with_state_dir(&dir);
    assert_eq!(next.restore().restored, ["keep"]);
    wait_for(&next, "keep", "back-again");
}

#[test]
fn an_agent_that_exited_or_was_removed_is_not_brought_back() {
    let (_t, dir) = state_dir();
    let host = Host::new().with_state_dir(&dir);
    host.spawn(resumable("done", "exit 0", "echo nope"))
        .unwrap();
    host.wait_exit("done", WAIT).unwrap();
    host.spawn(resumable("gone", "sleep 60", "echo nope"))
        .unwrap();
    host.remove("gone").unwrap();
    host.spawn(resumable("stays", "sleep 60", "echo yes; sleep 60"))
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let (_t2, dir2) = state_dir();
    copy_state(&dir, &dir2);
    let report = Host::new().with_state_dir(&dir2).restore();
    assert_eq!(report.restored, ["stays"], "{:?}", report.skipped);
}

#[test]
fn an_agent_without_a_resume_command_is_never_saved() {
    let (_t, dir) = state_dir();
    let host = Host::new().with_state_dir(&dir);
    let spec = SpawnSpec::new(
        "plain",
        vec!["sleep".into(), "60".into()],
        std::env::temp_dir(),
    );
    host.spawn(spec).unwrap();
    assert!(!host.info("plain").unwrap().resumable);
    let saved = std::fs::read_to_string(dir.join("session.json")).unwrap();
    assert!(!saved.contains("plain"), "{saved}");
}

#[test]
fn one_bad_record_does_not_stop_the_others() {
    let (_t, dir) = state_dir();
    let host = Host::new().with_state_dir(&dir);
    host.spawn(resumable("good", "sleep 60", "echo good-back; sleep 60"))
        .unwrap();
    let mut gone = resumable("lost", "sleep 60", "echo x");
    let missing = std::env::temp_dir().join("hq-host-test-dir-that-will-vanish");
    std::fs::create_dir_all(&missing).unwrap();
    gone.cwd = missing.clone();
    host.spawn(gone).unwrap();
    let (_t2, dir2) = state_dir();
    copy_state(&dir, &dir2);
    std::fs::remove_dir_all(&missing).unwrap();

    let report = Host::new().with_state_dir(&dir2).restore();
    assert_eq!(report.restored, ["good"]);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].0, "lost");
}

#[test]
fn a_state_file_others_can_write_is_not_obeyed() {
    let (_t, dir) = state_dir();
    let host = Host::new().with_state_dir(&dir);
    host.spawn(resumable("a", "sleep 60", "echo should-not-run"))
        .unwrap();
    let (_t2, dir2) = state_dir();
    copy_state(&dir, &dir2);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        dir2.join("session.json"),
        std::fs::Permissions::from_mode(0o666),
    )
    .unwrap();

    let next = Host::new().with_state_dir(&dir2);
    let report = next.restore();
    assert!(report.restored.is_empty());
    assert_eq!(report.skipped[0].0, "session.json");
    assert!(
        next.list().is_empty(),
        "nothing may start from an untrusted file"
    );
}

#[test]
fn an_empty_resume_command_is_refused() {
    let host = Host::new();
    let mut spec = resumable("x", "true", "true");
    spec.resume_argv = Some(Vec::new());
    assert_eq!(host.spawn(spec).unwrap_err().code(), "invalid_resume");
}

#[test]
fn a_host_without_a_state_dir_remembers_nothing() {
    let host = Host::new();
    host.spawn(resumable("x", "sleep 60", "sleep 60")).unwrap();
    assert!(host.restore().restored.is_empty());
}

#[test]
fn an_agent_that_was_told_to_stop_is_not_brought_back_even_before_it_has_exited() {
    let (_t, dir) = state_dir();
    let host = Host::new().with_state_dir(&dir);
    // Ignores the hangup, so it is still running when the state is read.
    host.spawn(resumable("stubborn", "trap '' HUP; sleep 60", "echo nope"))
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    host.kill("stubborn").unwrap();

    let (_t2, dir2) = state_dir();
    copy_state(&dir, &dir2);
    let report = Host::new().with_state_dir(&dir2).restore();
    assert!(
        report.restored.is_empty(),
        "a killed agent was resurrected: {report:?}"
    );
}
