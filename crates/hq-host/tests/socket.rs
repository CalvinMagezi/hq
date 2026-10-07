//! The control socket, exercised through real sockets and real processes.

use hq_host::{Client, ClientError, Host, Limits, Server, socket_path};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

struct Running {
    dir: tempfile::TempDir,
    stop: hq_host::StopHandle,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::bind(&dir.path().join("run"), Arc::new(Host::new())).unwrap();
        let stop = server.stop_handle();
        let thread = std::thread::spawn(move || server.serve());
        Self {
            dir,
            stop,
            thread: Some(thread),
        }
    }

    fn run_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("run")
    }

    fn client(&self) -> Client {
        Client::connect(&self.run_dir()).unwrap()
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

fn raw(dir: &Path, line: &str) -> Value {
    let mut s = UnixStream::connect(socket_path(dir)).unwrap();
    s.set_read_timeout(Some(WAIT)).unwrap();
    s.write_all(line.as_bytes()).unwrap();
    s.write_all(b"\n").unwrap();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while s.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
        buf.push(byte[0]);
    }
    serde_json::from_slice(&buf).unwrap()
}

fn spawn_params(name: &str, script: &str) -> Value {
    json!({ "name": name, "argv": ["sh", "-c", script], "cwd": std::env::temp_dir() })
}

fn read_until(c: &mut Client, name: &str, want: &str) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        let text = c
            .call("agent.read", json!({ "name": name, "source": "visible" }))
            .unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        if text.contains(want) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "never saw {want:?}; screen:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_full_agent_round_trip_over_the_socket() {
    let host = Running::start();
    let mut c = host.client();
    let info = c
        .call(
            "agent.spawn",
            spawn_params("one", "read line; printf 'got:%s\\n' \"$line\"; exit 7"),
        )
        .unwrap();
    assert_eq!(info["status"], "running");
    c.call(
        "agent.prompt",
        json!({ "name": "one", "text": "hello socket" }),
    )
    .unwrap();
    read_until(&mut c, "one", "got:hello socket");
    let waited = c
        .call(
            "agent.wait",
            json!({ "name": "one", "until": "exit", "timeout_ms": 10_000 }),
        )
        .unwrap();
    assert_eq!(waited["exit_code"], 7);
    assert_eq!(
        c.call("agent.get", json!({ "name": "one" })).unwrap()["status"],
        "exited"
    );
    assert_eq!(
        c.call("agent.list", json!({})).unwrap()["agents"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    c.call("agent.remove", json!({ "name": "one" })).unwrap();
    assert_eq!(c.call("host.status", json!({})).unwrap()["agents"], 0);
}

#[test]
fn keys_resize_and_quiet_work_remotely() {
    let host = Running::start();
    let mut c = host.client();
    c.call("agent.spawn", json!({ "name": "cat", "argv": ["cat"], "cwd": std::env::temp_dir(), "rows": 20, "cols": 60 })).unwrap();
    c.call("agent.send_text", json!({ "name": "cat", "text": "abc\r" }))
        .unwrap();
    read_until(&mut c, "cat", "abc");
    c.call(
        "agent.resize",
        json!({ "name": "cat", "rows": 10, "cols": 50 }),
    )
    .unwrap();
    let quiet = c
        .call(
            "agent.wait",
            json!({ "name": "cat", "until": "quiet", "quiet_ms": 300, "timeout_ms": 10_000 }),
        )
        .unwrap();
    assert_eq!(
        (quiet["rows"].as_u64(), quiet["cols"].as_u64()),
        (Some(10), Some(50))
    );
    let err = c
        .call(
            "agent.send_keys",
            json!({ "name": "cat", "keys": ["--nope"] }),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some("invalid_keys"));
    c.call(
        "agent.send_keys",
        json!({ "name": "cat", "keys": ["ctrl+d"] }),
    )
    .unwrap();
    assert_eq!(
        c.call("agent.wait", json!({ "name": "cat", "until": "exit" }))
            .unwrap()["exit_code"],
        0
    );
}

#[test]
fn nothing_works_before_hello_and_the_token_is_checked() {
    let host = Running::start();
    let dir = host.run_dir();
    let early = raw(&dir, r#"{"id":1,"method":"host.status","params":{}}"#);
    assert_eq!(early["error"]["code"], "unauthenticated");
    let wrong = raw(
        &dir,
        r#"{"id":2,"method":"hello","params":{"protocol_version":1,"token":"nope"}}"#,
    );
    assert_eq!(wrong["error"]["code"], "unauthorized");
    match Client::connect_with_token(&dir, "nope") {
        Err(e @ ClientError::Remote { .. }) => assert_eq!(e.code(), Some("unauthorized")),
        other => panic!("expected unauthorized, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn a_protocol_version_mismatch_is_refused() {
    let host = Running::start();
    let token = std::fs::read_to_string(hq_host::token_path(&host.run_dir())).unwrap();
    let line = format!(
        r#"{{"id":1,"method":"hello","params":{{"protocol_version":999,"token":"{}"}}}}"#,
        token.trim()
    );
    assert_eq!(
        raw(&host.run_dir(), &line)["error"]["code"],
        "protocol_mismatch"
    );
}

#[test]
fn bad_requests_get_errors_not_silence() {
    let host = Running::start();
    let mut c = host.client();
    assert_eq!(
        c.call("agent.nope", json!({})).unwrap_err().code(),
        Some("unknown_method")
    );
    assert_eq!(
        c.call("agent.get", json!({})).unwrap_err().code(),
        Some("invalid_params")
    );
    assert_eq!(
        c.call("agent.get", json!({ "name": "ghost" }))
            .unwrap_err()
            .code(),
        Some("agent_not_found")
    );
    assert_eq!(
        c.call("agent.wait", json!({ "name": "x", "until": "never" }))
            .unwrap_err()
            .code(),
        Some("invalid_params")
    );
    assert_eq!(
        raw(&host.run_dir(), "this is not json")["error"]["code"],
        "bad_request"
    );
}

#[test]
fn an_oversized_line_closes_the_connection() {
    let host = Running::start();
    let mut s = UnixStream::connect(socket_path(&host.run_dir())).unwrap();
    s.write_all(&vec![b'x'; hq_host::MAX_LINE_BYTES + 10]).ok();
    s.write_all(b"\n").ok();
    let mut out = String::new();
    s.read_to_string(&mut out).ok();
    assert!(out.contains("line_too_long"), "{out:?}");
}

#[test]
fn socket_and_directory_are_private() {
    let host = Running::start();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&host.run_dir()), 0o700);
    assert_eq!(mode(&socket_path(&host.run_dir())), 0o600);
    assert_eq!(mode(&hq_host::token_path(&host.run_dir())), 0o600);
}

#[test]
fn a_live_host_blocks_a_second_one_but_a_stale_socket_is_replaced() {
    let host = Running::start();
    let err = Server::bind(&host.run_dir(), Arc::new(Host::new()))
        .err()
        .expect("second bind must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);

    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join("run");
    {
        let first = Server::bind(&run, Arc::new(Host::new())).unwrap();
        drop(first); // listener closed, socket file left behind
    }
    assert!(socket_path(&run).exists());
    Server::bind(&run, Arc::new(Host::new())).expect("stale socket replaced");
}

#[test]
fn host_stop_ends_serving_and_removes_the_socket() {
    let mut host = Running::start();
    let mut c = host.client();
    c.call("host.stop", json!({})).unwrap();
    host.thread.take().unwrap().join().unwrap();
    assert!(!socket_path(&host.run_dir()).exists());
}

fn start_with(limits: Limits) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let server =
        Server::bind_with_limits(&dir.path().join("run"), Arc::new(Host::new()), limits).unwrap();
    let stop = server.stop_handle();
    let thread = std::thread::spawn(move || server.serve());
    Running {
        dir,
        stop,
        thread: Some(thread),
    }
}

#[test]
fn a_huge_read_is_cut_to_a_tail_that_fits_and_the_connection_stays_in_sync() {
    let host = Running::start();
    let mut c = host.client();
    let script = "i=0; while [ $i -lt 3000 ]; do echo line$i-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; i=$((i+1)); done; echo THE-END; sleep 30";
    let mut params = spawn_params("big", script);
    params["cols"] = json!(1000);
    c.call("agent.spawn", params).unwrap();
    read_until(&mut c, "big", "THE-END");
    let read = c
        .call("agent.read", json!({ "name": "big", "source": "recent" }))
        .unwrap();
    assert_eq!(read["truncated"], true);
    let text = read["text"].as_str().unwrap();
    assert!(text.len() < hq_host::MAX_LINE_BYTES && text.contains("THE-END"));
    // The next call must still get its own reply.
    assert_eq!(c.call("host.status", json!({})).unwrap()["agents"], 1);
    c.call("agent.kill", json!({ "name": "big" })).unwrap();
}

#[test]
fn stop_wakes_the_server_even_if_the_socket_file_is_gone() {
    let mut host = Running::start();
    std::fs::remove_file(socket_path(&host.run_dir())).unwrap();
    host.stop.stop();
    let thread = host.thread.take().unwrap();
    let deadline = Instant::now() + WAIT;
    while !thread.is_finished() {
        assert!(
            Instant::now() < deadline,
            "serve() did not return after stop"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    thread.join().unwrap();
}

#[test]
fn host_stop_answers_before_the_server_goes_away() {
    for _ in 0..20 {
        let mut host = Running::start();
        let mut c = host.client();
        assert!(
            c.call("host.stop", json!({})).is_ok(),
            "the reply to host.stop was lost"
        );
        host.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn connections_are_capped_and_idle_ones_time_out_before_hello() {
    let limits = Limits {
        max_connections: 3,
        auth_timeout: Duration::from_millis(400),
        ..Limits::default()
    };
    let host = start_with(limits);
    let idle: Vec<UnixStream> = (0..3)
        .map(|_| {
            let s = UnixStream::connect(socket_path(&host.run_dir())).unwrap();
            s.set_read_timeout(Some(WAIT)).unwrap();
            s
        })
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    let mut over = UnixStream::connect(socket_path(&host.run_dir())).unwrap();
    over.set_read_timeout(Some(WAIT)).unwrap();
    let mut out = String::new();
    over.read_to_string(&mut out).ok();
    assert!(out.contains("too_many_connections"), "{out:?}");
    // The silent connections are dropped after the auth timeout, freeing slots.
    std::thread::sleep(Duration::from_millis(900));
    for mut s in idle {
        let mut buf = [0u8; 1];
        assert_eq!(
            s.read(&mut buf).unwrap_or(0),
            0,
            "idle connection should have been closed"
        );
    }
    host.client().call("host.status", json!({})).unwrap();
}

#[test]
fn only_one_of_two_simultaneous_hosts_wins_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join("run");
    let results: Vec<_> = (0..8)
        .map(|_| {
            let run = run.clone();
            std::thread::spawn(move || Server::bind(&run, Arc::new(Host::new())).map(|s| (s, ())))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
}

#[test]
fn state_is_reported_and_waited_for_over_the_socket() {
    let host = Running::start();
    let mut c = host.client();
    let mut params = spawn_params(
        "claude1",
        "printf '\\033]0;\\342\\227\\221 busy\\007'; sleep 30",
    );
    params["agent"] = json!("claude");
    let info = c.call("agent.spawn", params).unwrap();
    assert_eq!(info["agent"], "claude");
    let waited = c
        .call("agent.wait", json!({ "name": "claude1", "until": "state", "states": ["working"], "stable_ms": 200, "timeout_ms": 10_000 }))
        .unwrap();
    assert_eq!(waited["state"], "working");
    assert_eq!(waited["rule"], "osc_title_working");
    let got = c.call("agent.get", json!({ "name": "claude1" })).unwrap();
    assert!(got["title"].as_str().unwrap().contains("busy"));
    let err = c
        .call("agent.wait", json!({ "name": "claude1", "until": "state" }))
        .unwrap_err();
    assert_eq!(err.code(), Some("invalid_params"));
    let err = c
        .call(
            "agent.wait",
            json!({ "name": "claude1", "until": "state", "states": ["bogus"] }),
        )
        .unwrap_err();
    assert_eq!(err.code(), Some("invalid_params"));
    c.call("agent.kill", json!({ "name": "claude1" })).unwrap();
}

fn screen_of(c: &mut Client, name: &str) -> String {
    let r = c.call("agent.read", json!({ "name": name })).unwrap();
    r["text"].as_str().unwrap_or_default().to_string()
}

/// Spawns a claude-kind agent that prints its hook token and run directory.
fn spawn_reporter(c: &mut Client, name: &str) -> (String, String) {
    let mut params = spawn_params(name, "echo T=$HQ_HOST_TOKEN D=$HQ_HOST_DIR; sleep 60");
    params["agent"] = json!("claude");
    c.call("agent.spawn", params).unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let text = screen_of(c, name);
        let field = |key: &str| {
            text.split_whitespace()
                .find_map(|w| w.strip_prefix(key))
                .map(str::to_string)
        };
        if let (Some(t), Some(d)) = (field("T="), field("D="))
            && !t.is_empty()
            && !d.is_empty()
        {
            return (t, d);
        }
        assert!(Instant::now() < deadline, "no token on screen: {text:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_pane_reports_on_itself_with_the_token_it_was_given() {
    let host = Running::start();
    let mut op = host.client();
    let (token, dir) = spawn_reporter(&mut op, "rep");
    assert_eq!(std::path::Path::new(&dir), host.run_dir().as_path());

    let mut pane = Client::connect_with_token(std::path::Path::new(&dir), &token).unwrap();
    pane.call(
        "agent.report",
        json!({ "event": "UserPromptSubmit", "session_id": "conv-123" }),
    )
    .unwrap();

    let info = op.call("agent.get", json!({ "name": "rep" })).unwrap();
    assert_eq!(info["state"], "working");
    assert_eq!(info["rule"], "hook:UserPromptSubmit");
    assert_eq!(info["agent_session_id"], "conv-123");

    pane.call("agent.report", json!({ "event": "Stop" }))
        .unwrap();
    let info = op.call("agent.get", json!({ "name": "rep" })).unwrap();
    assert_eq!(info["state"], "idle");
    assert_eq!(info["agent_session_id"], "conv-123", "the id is kept");
}

#[test]
fn a_pane_token_reaches_nothing_but_its_own_report() {
    let host = Running::start();
    let mut op = host.client();
    let (token, dir) = spawn_reporter(&mut op, "mine");
    let (_other_token, _) = spawn_reporter(&mut op, "theirs");
    let mut pane = Client::connect_with_token(std::path::Path::new(&dir), &token).unwrap();

    for (method, params) in [
        ("agent.list", json!({})),
        ("agent.kill", json!({ "name": "mine" })),
        ("agent.read", json!({ "name": "mine" })),
        ("host.stop", json!({})),
    ] {
        let err = pane.call(method, params).unwrap_err();
        assert_eq!(err.code(), Some("forbidden"), "{method}");
    }
    let err = pane
        .call("agent.report", json!({ "name": "theirs", "event": "Stop" }))
        .unwrap_err();
    assert_eq!(err.code(), Some("forbidden"));
    let theirs = op.call("agent.get", json!({ "name": "theirs" })).unwrap();
    assert_ne!(theirs["rule"], "hook:Stop");
    assert!(
        op.call("agent.get", json!({ "name": "mine" })).is_ok(),
        "still running"
    );
}

#[test]
fn a_removed_agents_token_stops_working() {
    let host = Running::start();
    let mut op = host.client();
    let (token, dir) = spawn_reporter(&mut op, "gone");
    op.call("agent.remove", json!({ "name": "gone" })).unwrap();
    let Err(err) = Client::connect_with_token(std::path::Path::new(&dir), &token) else {
        panic!("a removed agent's token was accepted");
    };
    assert!(
        matches!(err, ClientError::Remote { ref code, .. } if code == "unauthorized"),
        "{err}"
    );
}

#[test]
fn an_unknown_token_is_unauthorized_and_the_operator_can_report_for_any_agent() {
    let host = Running::start();
    let mut op = host.client();
    spawn_reporter(&mut op, "any");
    assert!(Client::connect_with_token(&host.run_dir(), "not-a-token").is_err());
    op.call(
        "agent.report",
        json!({ "name": "any", "event": "UserPromptSubmit" }),
    )
    .unwrap();
    assert_eq!(
        op.call("agent.get", json!({ "name": "any" })).unwrap()["state"],
        "working"
    );
    let err = op
        .call("agent.report", json!({ "event": "Stop" }))
        .unwrap_err();
    assert_eq!(err.code(), Some("invalid_params"));
}

/// Polls until an event of `kind` for `name` shows up; returns it and the new cursor.
fn next_event(c: &mut Client, after: u64, name: &str, kind: &str) -> (Value, u64) {
    let deadline = Instant::now() + WAIT;
    let mut cursor = after;
    loop {
        let r = c
            .call("events.poll", json!({ "after": cursor, "timeout_ms": 500 }))
            .unwrap();
        cursor = r["last_seq"].as_u64().unwrap();
        let wanted = |e: &&Value| {
            let kind_ok = e["name"] == name && (e["kind"] == kind || kind == "state:working");
            kind_ok && (kind != "state:working" || e["state"] == "working")
        };
        if let Some(e) = r["events"].as_array().unwrap().iter().find(wanted) {
            return (e.clone(), cursor);
        }
        assert!(Instant::now() < deadline, "no {kind} event for {name}");
    }
}

#[test]
fn a_client_hears_about_spawns_state_changes_exits_and_removals() {
    let host = Running::start();
    let mut c = host.client();
    let start = c.call("events.poll", json!({})).unwrap();
    assert_eq!(start["events"].as_array().unwrap().len(), 0);
    let cursor = start["last_seq"].as_u64().unwrap();

    let mut params = spawn_params("ev", "sleep 60");
    params["agent"] = json!("claude");
    c.call("agent.spawn", params).unwrap();
    let (spawned, cursor) = next_event(&mut c, cursor, "ev", "spawned");
    assert!(spawned["seq"].as_u64().unwrap() > 0);

    c.call(
        "agent.report",
        json!({ "name": "ev", "event": "UserPromptSubmit" }),
    )
    .unwrap();
    let (state, cursor) = next_event(&mut c, cursor, "ev", "state:working");
    assert_eq!(state["state"], "working");
    assert_eq!(state["rule"], "hook:UserPromptSubmit");

    c.call("agent.kill", json!({ "name": "ev" })).unwrap();
    let (_, cursor) = next_event(&mut c, cursor, "ev", "exited");
    c.call("agent.remove", json!({ "name": "ev" })).unwrap();
    next_event(&mut c, cursor, "ev", "removed");
}

#[test]
fn an_idle_poll_waits_then_returns_empty_and_a_pane_token_cannot_poll() {
    let host = Running::start();
    let mut op = host.client();
    let last = op.call("events.poll", json!({})).unwrap()["last_seq"]
        .as_u64()
        .unwrap();
    let began = Instant::now();
    let r = op
        .call("events.poll", json!({ "after": last, "timeout_ms": 300 }))
        .unwrap();
    assert!(r["events"].as_array().unwrap().is_empty());
    assert!(began.elapsed() >= Duration::from_millis(250));

    let (token, dir) = spawn_reporter(&mut op, "poller");
    let mut pane = Client::connect_with_token(std::path::Path::new(&dir), &token).unwrap();
    let err = pane.call("events.poll", json!({})).unwrap_err();
    assert_eq!(err.code(), Some("forbidden"));
}

#[test]
fn each_state_change_bumps_the_agents_change_counter() {
    let host = Running::start();
    let mut c = host.client();
    let mut params = spawn_params("seq", "sleep 60");
    params["agent"] = json!("claude");
    c.call("agent.spawn", params).unwrap();

    let seq_of = |c: &mut Client| {
        c.call("agent.get", json!({ "name": "seq" })).unwrap()["state_seq"]
            .as_u64()
            .unwrap()
    };
    let wait_past = |c: &mut Client, old: u64| {
        let deadline = Instant::now() + WAIT;
        loop {
            let now = seq_of(c);
            if now > old {
                return now;
            }
            assert!(Instant::now() < deadline, "counter stuck at {old}");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let first = wait_past(&mut c, 0);
    c.call("agent.report", json!({ "name": "seq", "event": "UserPromptSubmit" })).unwrap();
    let second = wait_past(&mut c, first);
    c.call("agent.report", json!({ "name": "seq", "event": "Stop" })).unwrap();
    let third = wait_past(&mut c, second);
    assert!(first < second && second < third);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(seq_of(&mut c), third, "no change, no bump");
}

#[test]
fn a_finished_turn_is_done_until_the_agent_works_again() {
    let host = Running::start();
    let mut c = host.client();
    let mut params = spawn_params("fin", "sleep 60");
    params["agent"] = json!("claude");
    c.call("agent.spawn", params).unwrap();

    let info = |c: &mut Client| c.call("agent.get", json!({ "name": "fin" })).unwrap();
    let wait_for = |c: &mut Client, want: bool| {
        let deadline = Instant::now() + WAIT;
        loop {
            let i = c.call("agent.get", json!({ "name": "fin" })).unwrap();
            if i["done"] == want && i["state"] == if want { "idle" } else { "working" } {
                return i;
            }
            assert!(Instant::now() < deadline, "done never became {want}: {i}");
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    assert_eq!(info(&mut c)["done"], false, "a fresh agent has not finished anything");

    c.call("agent.report", json!({ "name": "fin", "event": "UserPromptSubmit" })).unwrap();
    wait_for(&mut c, false);
    c.call("agent.report", json!({ "name": "fin", "event": "Stop" })).unwrap();
    let finished = wait_for(&mut c, true);

    c.call("agent.report", json!({ "name": "fin", "event": "UserPromptSubmit" })).unwrap();
    let working = wait_for(&mut c, false);
    assert!(working["state_seq"].as_u64() > finished["state_seq"].as_u64());
}
