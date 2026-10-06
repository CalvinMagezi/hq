//! Runs real processes through the host. They use only `sh` and `cat`.

use hq_host::{AgentState, Host, HostError, PaneStatus, ReadSource, SpawnSpec};
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
    // Raw mode and no echo: the process sees exactly the bytes that were sent.
    host.spawn(sh(
        "k",
        "stty raw -echo; dd bs=1 count=3 2>/dev/null | od -An -tx1; sleep 30",
    ))
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let err = host
        .send_keys("k", &["enter".to_string(), "--help".to_string()])
        .unwrap_err();
    assert_eq!(err.code(), "invalid_keys");
    host.send_text("k", "abc").unwrap();
    let screen = wait_until_read(&host, "k", ReadSource::Visible, "63");
    let bytes: Vec<&str> = screen.split_whitespace().collect();
    assert_eq!(
        bytes,
        ["61", "62", "63"],
        "unexpected bytes reached the process"
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
    host.wait_quiet("wrap", Duration::from_millis(300), WAIT)
        .unwrap();
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

fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .unwrap()
        .success()
}

fn wait_dead(pid: u32) {
    let deadline = Instant::now() + WAIT;
    while pid_alive(pid) {
        assert!(Instant::now() < deadline, "process {pid} is still running");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn zero_and_absurd_sizes_are_refused() {
    let host = Host::new();
    for (rows, cols) in [(0, 80), (24, 0), (5000, 80), (24, 5000)] {
        let mut spec = sh("sized", "sleep 30");
        spec.rows = rows;
        spec.cols = cols;
        assert_eq!(
            host.spawn(spec).unwrap_err().code(),
            "invalid_size",
            "{rows}x{cols}"
        );
    }
    let mut spec = sh("sized", "sleep 30");
    spec.scrollback_rows = usize::MAX;
    host.spawn(spec).unwrap();
    assert_eq!(
        host.resize("sized", 0, 0).unwrap_err().code(),
        "invalid_size"
    );
    host.kill("sized").unwrap();
}

#[test]
fn a_working_directory_that_is_not_a_directory_is_an_error() {
    let host = Host::new();
    for cwd in ["/no/such/dir-hq-host", "relative/dir"] {
        let spec = SpawnSpec::new("cwd", vec!["true".into()], cwd);
        assert_eq!(
            host.spawn(spec).unwrap_err().code(),
            "spawn_failed",
            "{cwd}"
        );
    }
}

#[test]
fn kill_takes_down_a_child_that_ignores_hangup_and_its_children() {
    let host = Host::new();
    let info = host
        .spawn(sh(
            "stubborn",
            "trap '' HUP; sleep 300 & echo child=$!; wait",
        ))
        .unwrap();
    let shell = info.pid.expect("pid");
    let screen = wait_until_read(&host, "stubborn", ReadSource::Visible, "child=");
    let child: u32 = screen
        .split("child=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    host.kill("stubborn").unwrap();
    wait_dead(shell);
    wait_dead(child);
}

#[test]
fn remove_kills_and_frees_the_name_even_while_someone_waits() {
    let host = std::sync::Arc::new(Host::new());
    let info = host.spawn(sh("busy", "sleep 300")).unwrap();
    let pid = info.pid.expect("pid");
    let waiter = {
        let host = host.clone();
        std::thread::spawn(move || host.wait_exit("busy", Duration::from_secs(60)))
    };
    std::thread::sleep(Duration::from_millis(200));
    host.remove("busy").unwrap();
    wait_dead(pid);
    assert!(
        waiter.join().unwrap().is_ok(),
        "the waiter should be released by the exit"
    );
    host.spawn(sh("busy", "true")).unwrap();
}

#[test]
fn a_second_kill_after_exit_signals_nothing() {
    let host = Host::new();
    host.spawn(sh("done", "exit 0")).unwrap();
    host.wait_exit("done", WAIT).unwrap();
    host.kill("done").unwrap();
    host.kill("done").unwrap();
}

#[test]
fn output_is_complete_the_moment_exit_is_reported() {
    let host = Host::new();
    for i in 0..15 {
        let name = format!("race{i}");
        host.spawn(sh(
            &name,
            "i=0; while [ $i -lt 300 ]; do echo line$i; i=$((i+1)); done; echo END-MARKER; exit 3",
        ))
        .unwrap();
        assert_eq!(host.wait_exit(&name, WAIT).unwrap(), 3);
        let text = host.read(&name, ReadSource::Recent, 0).unwrap();
        assert!(
            text.contains("END-MARKER"),
            "iteration {i} lost the tail:\n{}",
            &text[text.len().saturating_sub(200)..]
        );
    }
}

fn agent(name: &str, kind: &str, script: &str) -> SpawnSpec {
    let mut spec = sh(name, script);
    spec.agent = Some(kind.to_string());
    spec
}

#[test]
fn state_comes_from_the_screen_and_title_of_a_known_agent() {
    let host = Host::new();
    // Claude Code shows a spinner glyph in its terminal title while it works.
    host.spawn(agent(
        "busy",
        "claude",
        "printf '\\033]0;\\342\\227\\221 thinking\\007'; sleep 30",
    ))
    .unwrap();
    let info = host
        .wait_state(
            "busy",
            &[AgentState::Working],
            Duration::from_millis(200),
            WAIT,
        )
        .unwrap();
    assert_eq!(info.state, Some(AgentState::Working));
    assert_eq!(info.rule.as_deref(), Some("osc_title_working"));
    assert_eq!(info.agent.as_deref(), Some("claude"));
    assert!(info.title.contains("thinking"));
    host.kill("busy").unwrap();
}

#[test]
fn a_calm_screen_of_a_known_agent_is_idle_and_waiting_for_another_state_times_out() {
    let host = Host::new();
    host.spawn(agent("calm", "claude", "echo hello; sleep 30"))
        .unwrap();
    let info = host
        .wait_state(
            "calm",
            &[AgentState::Idle],
            Duration::from_millis(200),
            WAIT,
        )
        .unwrap();
    assert_eq!(info.state, Some(AgentState::Idle));
    let err = host
        .wait_state(
            "calm",
            &[AgentState::Blocked],
            Duration::ZERO,
            Duration::from_millis(400),
        )
        .unwrap_err();
    assert_eq!(err.code(), "timeout");
    host.kill("calm").unwrap();
}

#[test]
fn a_blocked_dialog_is_seen_as_blocked() {
    let host = Host::new();
    let dialog = "printf '\\n  Hooks need review\\n\\n  1. Review hooks\\n\\n  Press enter to confirm or esc to go back\\n'; sleep 30";
    host.spawn(agent("dialog", "codex", dialog)).unwrap();
    let info = host
        .wait_state(
            "dialog",
            &[AgentState::Blocked],
            Duration::from_millis(200),
            WAIT,
        )
        .unwrap();
    assert_eq!(info.rule.as_deref(), Some("hooks_review_dialog"));
    host.kill("dialog").unwrap();
}

#[test]
fn an_agent_kind_without_rules_has_no_state() {
    let host = Host::new();
    host.spawn(agent("plain", "no-such-agent", "sleep 30"))
        .unwrap();
    assert!(host.info("plain").unwrap().state.is_none());
    host.spawn(sh("noagent", "sleep 30")).unwrap();
    assert!(host.info("noagent").unwrap().state.is_none());
    assert_eq!(
        host.wait_state(
            "plain",
            &[AgentState::Idle],
            Duration::ZERO,
            Duration::from_millis(300)
        )
        .unwrap_err()
        .code(),
        "timeout"
    );
    host.kill("plain").unwrap();
    host.kill("noagent").unwrap();
}

#[test]
fn a_flicker_shorter_than_the_stability_window_is_not_a_change() {
    let host = Host::new();
    // Working for about 300 ms, then calm for good.
    let script =
        "printf '\\033]0;\\342\\227\\221 x\\007'; sleep 0.3; printf '\\033]0;done\\007'; sleep 30";
    host.spawn(agent("flick", "claude", script)).unwrap();
    let err = host
        .wait_state(
            "flick",
            &[AgentState::Working],
            Duration::from_secs(2),
            Duration::from_secs(3),
        )
        .unwrap_err();
    assert_eq!(
        err.code(),
        "timeout",
        "a 300 ms spinner must not count as a 2 s state"
    );
    host.kill("flick").unwrap();
}
