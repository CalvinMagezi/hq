//! A restarted built-in host holds agents that had env until HQ supplies it.

use hq_db::Database;
use hq_db::harness_sessions_registry::{self as registry, NewSession, Placement};
use hq_host::{Host, ReadSource, Server, SpawnSpec};
use hq_tools::harness_session::resume_awaiting;
use hq_tools::agent_host::{Host as HostHandle, NativeBackend};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);
const SESSION: &str = "hs-restart";

fn sh(script: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), script.into()]
}

#[test]
fn a_restarted_host_gets_each_agents_env_back_from_hq() {
    let tmp = tempfile::tempdir().unwrap();
    let (first_dir, second_dir) = (tmp.path().join("one"), tmp.path().join("two"));

    // The host before the restart: an agent launched with HQ's session id.
    let first = Host::new().with_state_dir(&first_dir);
    let mut spec = SpawnSpec::new(SESSION, sh("sleep 60"), std::env::temp_dir());
    spec.env = vec![("HQ_SESSION_ID".into(), SESSION.into())];
    spec.resume_argv = Some(sh("echo session=$HQ_SESSION_ID; sleep 60"));
    first.spawn(spec).unwrap();

    // What a crash leaves behind, read by a fresh host.
    std::fs::create_dir_all(&second_dir).unwrap();
    std::fs::copy(
        first_dir.join("session.json"),
        second_dir.join("session.json"),
    )
    .unwrap();
    let second = Arc::new(Host::new().with_state_dir(&second_dir));
    assert!(second.restore().restored.is_empty());
    let server = Server::bind(&second_dir, second.clone()).unwrap();
    let stop = server.stop_handle();
    let thread = std::thread::spawn(move || server.serve());

    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        registry::insert(
            c,
            &NewSession {
                id: SESSION,
                harness: "claude-code",
                label: "t",
                cwd: "/tmp",
                mission_id: None,
                placement: Placement {
                    host: "native",
                    agent_name: SESSION,
                    workspace_id: SESSION,
                    pane_id: SESSION,
                },
            },
        )
    })
    .unwrap();
    // A session on another host may share the agent name; it must not win.
    // Inserted last so the newest-first listing offers it first.
    db.with_conn(|c| {
        registry::insert(
            c,
            &NewSession {
                id: "hs-elsewhere",
                harness: "codex",
                label: "t",
                cwd: "/tmp",
                mission_id: None,
                placement: Placement {
                    host: "local",
                    agent_name: SESSION,
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
            },
        )
    })
    .unwrap();
    let mut rows = db.with_conn(|c| registry::list(c, None, 10)).unwrap();
    rows.sort_by_key(|r| r.host == "native");

    let backend: HostHandle = Arc::new(NativeBackend::new(&second_dir));
    resume_awaiting(&rows, &backend);

    let deadline = Instant::now() + WAIT;
    let screen = loop {
        let text = second
            .read(SESSION, ReadSource::Visible, 0)
            .unwrap_or_default();
        if text.contains("session=hs-restart") || Instant::now() > deadline {
            break text;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        screen.contains("session=hs-restart"),
        "screen was: {screen}"
    );
    assert!(backend.awaiting().unwrap().is_empty());

    stop.stop();
    thread.join().unwrap();
}

#[test]
fn a_reported_conversation_id_becomes_the_hosts_restart_command() {
    use hq_host::Client;
    use hq_tools::harness_session::refresh_restart_command;
    use hq_tools::agent_host::{HostBackend, LaunchRequest};

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("run");
    let host = Arc::new(Host::new().with_state_dir(&dir));
    let server = Server::bind(&dir, host).unwrap();
    let stop = server.stop_handle();
    let thread = std::thread::spawn(move || server.serve());

    let backend: HostHandle = Arc::new(NativeBackend::new(&dir));
    backend
        .launch(&LaunchRequest {
            name: SESSION.into(),
            kind: "claude".into(),
            cwd: std::env::temp_dir().to_string_lossy().into(),
            label: "t".into(),
            env: vec![],
            args: vec![],
            command: Some("f() { echo started; cat; }; f".into()),
            resume_args: Some(vec!["-c".into()]),
            mcp: None,
            start_timeout: WAIT,
        })
        .unwrap();
    let mut client = Client::connect(&dir).unwrap();
    client
        .call(
            "agent.report",
            serde_json::json!({ "name": SESSION, "event": "SessionStart", "session_id": "conv-x" }),
        )
        .unwrap();

    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        registry::insert(
            c,
            &NewSession {
                id: SESSION,
                harness: "claude-code",
                label: "t",
                cwd: "/tmp",
                mission_id: None,
                placement: Placement {
                    host: "native",
                    agent_name: SESSION,
                    workspace_id: SESSION,
                    pane_id: SESSION,
                },
            },
        )
    })
    .unwrap();
    let row = db
        .with_conn(|c| registry::get(c, SESSION))
        .unwrap()
        .unwrap();
    let agent = backend.agent(SESSION).unwrap().unwrap();
    assert_eq!(agent.agent_session_id.as_deref(), Some("conv-x"));

    refresh_restart_command(tmp.path(), &backend, &row, &agent).unwrap();

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("session.json")).unwrap()).unwrap();
    let argv: Vec<&str> = saved["panes"][0]["resume_argv"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(argv[0], "claude");
    let at = argv
        .iter()
        .position(|a| *a == "--resume")
        .expect("--resume missing");
    assert_eq!(argv[at + 1], "conv-x");
    assert!(
        argv.contains(&"--settings"),
        "hooks stay on after a restart: {argv:?}"
    );
    assert!(
        !argv.contains(&"-c"),
        "the id replaces continue-latest: {argv:?}"
    );

    stop.stop();
    thread.join().unwrap();
}
