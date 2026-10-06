//! Runs real processes through the host. They use only `sh` and `cat`.

use hq_host::{Host, HostError, PaneStatus, ReadSource, SpawnSpec};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

fn sh(name: &str, script: &str) -> SpawnSpec {
    SpawnSpec::new(
        name,
        vec!["sh".into(), "-c".into(), script.into()],
        std::env::temp_dir(),
    )
}

fn wait_until_read(host: &Host, name: &str, source: ReadSource, want: &str) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        let text = host.read(name, source, 0).unwrap();
        if text.contains(want) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "never saw {want:?}; screen was:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn output_shows_up_on_screen() {
    let host = Host::new();
    host.spawn(sh("hello", "printf 'hello world\\n'; sleep 30"))
        .unwrap();
    wait_until_read(&host, "hello", ReadSource::Visible, "hello world");
    host.kill("hello").unwrap();
}

#[test]
fn exit_code_is_reported() {
    let host = Host::new();
    host.spawn(sh("quits", "exit 3")).unwrap();
    assert_eq!(host.wait_exit("quits", WAIT).unwrap(), 3);
    assert_eq!(
        host.info("quits").unwrap().status,
        PaneStatus::Exited { code: 3 }
    );
    assert!(matches!(
        host.send_text("quits", "x"),
        Err(HostError::Exited(_))
    ));
}

#[test]
fn typed_input_reaches_the_process() {
    let host = Host::new();
    host.spawn(SpawnSpec::new(
        "echo",
        vec!["cat".into()],
        std::env::temp_dir(),
    ))
    .unwrap();
    host.send_text("echo", "ping\r").unwrap();
    wait_until_read(&host, "echo", ReadSource::Visible, "ping");
    host.send_keys("echo", &["ctrl+d".to_string()]).unwrap();
    assert_eq!(host.wait_exit("echo", WAIT).unwrap(), 0);
}

#[test]
fn prompt_pastes_then_presses_enter() {
    let host = Host::new();
    host.spawn(sh(
        "reader",
        "read line; printf 'got:%s\\n' \"$line\"; sleep 30",
    ))
    .unwrap();
    host.prompt("reader", "do the thing").unwrap();
    wait_until_read(&host, "reader", ReadSource::Visible, "got:do the thing");
    host.kill("reader").unwrap();
}

#[test]
fn invalid_keys_send_nothing() {
    let host = Host::new();
    host.spawn(SpawnSpec::new(
        "k",
        vec!["cat".into()],
        std::env::temp_dir(),
    ))
    .unwrap();
    let err = host
        .send_keys("k", &["enter".to_string(), "--help".to_string()])
        .unwrap_err();
    assert_eq!(err.code(), "invalid_keys");
    host.send_text("k", "marker\r").unwrap();
    let screen = wait_until_read(&host, "k", ReadSource::Visible, "marker");
    assert_eq!(
        screen.lines().filter(|l| l.contains("marker")).count(),
        2,
        "{screen}"
    );
    host.kill("k").unwrap();
}

#[test]
fn resize_reaches_the_process_and_the_screen() {
    let host = Host::new();
    host.spawn(sh("size", "while true; do stty size; sleep 0.2; done"))
        .unwrap();
    wait_until_read(&host, "size", ReadSource::Visible, "40 120");
    host.resize("size", 30, 90).unwrap();
    wait_until_read(&host, "size", ReadSource::Visible, "30 90");
    let info = host.info("size").unwrap();
    assert_eq!((info.rows, info.cols), (30, 90));
    host.kill("size").unwrap();
}

#[test]
fn long_lines_wrap_on_screen_and_rejoin_when_unwrapped() {
    let host = Host::new();
    let mut spec = sh("wrap", "printf '%0200d\\n' 0; sleep 30");
    spec.cols = 80;
    host.spawn(spec).unwrap();
    wait_until_read(&host, "wrap", ReadSource::Recent, "0000");
    std::thread::sleep(Duration::from_millis(200));
    let displayed = host.read("wrap", ReadSource::Recent, 0).unwrap();
    assert_eq!(
        displayed.lines().filter(|l| l.starts_with('0')).count(),
        3,
        "{displayed}"
    );
    let unwrapped = host.read("wrap", ReadSource::RecentUnwrapped, 0).unwrap();
    let line = unwrapped.lines().find(|l| l.starts_with('0')).unwrap();
    assert_eq!(line.len(), 200);
    host.kill("wrap").unwrap();
}

#[test]
fn scrollback_keeps_what_scrolled_off() {
    let host = Host::new();
    let mut spec = sh(
        "many",
        "i=1; while [ $i -le 120 ]; do echo row$i; i=$((i+1)); done; sleep 30",
    );
    spec.rows = 24;
    host.spawn(spec).unwrap();
    wait_until_read(&host, "many", ReadSource::Recent, "row120");
    let all = host.read("many", ReadSource::Recent, 0).unwrap();
    assert!(all.lines().any(|l| l == "row1") && all.lines().any(|l| l == "row120"));
    assert_eq!(
        host.read("many", ReadSource::Recent, 5)
            .unwrap()
            .lines()
            .count(),
        5
    );
    assert!(
        host.read("many", ReadSource::Visible, 0)
            .unwrap()
            .lines()
            .count()
            <= 24
    );
    host.kill("many").unwrap();
}

#[test]
fn the_pane_gets_a_clean_environment() {
    // Cargo sets this for the test process, so the parent really has it.
    assert!(std::env::var("CARGO_MANIFEST_DIR").is_ok());
    let host = Host::new();
    let mut spec = sh(
        "env",
        "printf 'sid=%s term=%s leaked=%s\\n' \"$HQ_SESSION_ID\" \"$TERM\" \"${CARGO_MANIFEST_DIR:-none}\"",
    );
    spec.env.push(("HQ_SESSION_ID".into(), "hs-test-1".into()));
    host.spawn(spec).unwrap();
    let screen = wait_until_read(&host, "env", ReadSource::Visible, "sid=");
    assert!(
        screen.contains("sid=hs-test-1 term=xterm-256color leaked=none"),
        "{screen}"
    );
}

#[test]
fn names_are_checked_and_unique() {
    let host = Host::new();
    assert_eq!(
        host.spawn(sh("Bad Name", "true")).unwrap_err().code(),
        "invalid_name"
    );
    host.spawn(sh("dup", "sleep 30")).unwrap();
    assert_eq!(
        host.spawn(sh("dup", "true")).unwrap_err().code(),
        "name_taken"
    );
    assert_eq!(host.info("nobody").unwrap_err().code(), "agent_not_found");
    host.remove("dup").unwrap();
    assert!(host.list().is_empty());
}

#[test]
fn a_missing_program_is_a_spawn_error() {
    let host = Host::new();
    let spec = SpawnSpec::new(
        "gone",
        vec!["/no/such/program-hq".into()],
        std::env::temp_dir(),
    );
    assert_eq!(host.spawn(spec).unwrap_err().code(), "spawn_failed");
}

#[test]
fn wait_quiet_returns_once_output_stops() {
    let host = Host::new();
    host.spawn(sh("burst", "echo a; echo b; sleep 30")).unwrap();
    host.wait_quiet("burst", Duration::from_millis(400), WAIT)
        .unwrap();
    assert!(host.info("burst").unwrap().quiet_for >= Duration::from_millis(400));
    host.kill("burst").unwrap();
}

#[test]
fn dropping_the_host_kills_its_processes() {
    let pid = {
        let host = Host::new();
        host.spawn(sh("orphan", "sleep 300"))
            .unwrap()
            .pid
            .expect("pid")
    };
    let deadline = Instant::now() + WAIT;
    loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .unwrap()
            .success();
        if !alive {
            break;
        }
        assert!(Instant::now() < deadline, "process {pid} survived the host");
        std::thread::sleep(Duration::from_millis(100));
    }
}
