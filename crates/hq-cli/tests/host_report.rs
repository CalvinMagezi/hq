//! The hook path end to end: a pane runs the hook command Claude Code would
//! run, with a hook payload on stdin, and the host learns the agent's state.

use hq_host::{Client, Host, Server, claude_settings};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(15);

fn hq() -> &'static str {
    env!("CARGO_BIN_EXE_hq")
}

fn hook_command() -> String {
    let settings = claude_settings(&format!("'{}' host report", hq()));
    settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_hook_event_in_a_pane_reaches_the_host_through_hq_host_report() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("run");
    let server = Server::bind(&dir, Arc::new(Host::new())).unwrap();
    let stop = server.stop_handle();
    let thread = std::thread::spawn(move || server.serve());

    let payload = r#"{"hook_event_name":"UserPromptSubmit","session_id":"conv-e2e"}"#;
    let script = format!("printf '%s' '{payload}' | {}; sleep 60", hook_command());
    let mut client = Client::connect(&dir).unwrap();
    client
        .call(
            "agent.spawn",
            json!({ "name": "hooked", "argv": ["sh", "-c", script],
                    "cwd": std::env::temp_dir(), "agent": "claude" }),
        )
        .unwrap();

    let deadline = Instant::now() + WAIT;
    let info = loop {
        let info = client
            .call("agent.get", json!({ "name": "hooked" }))
            .unwrap();
        if info["agent_session_id"] == "conv-e2e" || Instant::now() > deadline {
            break info;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    stop.stop();
    thread.join().unwrap();

    assert_eq!(info["agent_session_id"], "conv-e2e", "{info}");
    assert_eq!(info["state"], "working");
    assert_eq!(info["rule"], "hook:UserPromptSubmit");
}

#[test]
fn the_report_command_is_silent_and_succeeds_without_a_host() {
    let out = std::process::Command::new(hq())
        .args(["host", "report"])
        .env_remove("HQ_HOST_TOKEN")
        .env_remove("HQ_HOST_DIR")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "a SessionStart hook's output reaches the model"
    );
}

/// A stand-in for `ssh`: runs the gate on this machine and passes stdin
/// through, as ssh would to a forced command.
fn fake_ssh(dir: &std::path::Path, run_dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-ssh");
    let script = format!(
        "#!/bin/sh\nHQ_HOST_DIR='{}' exec '{}' host gate\n",
        run_dir.display(),
        hq()
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn a_remote_native_host_works_through_the_gate() {
    use hq_tools::herdr::{HostBackend, LaunchRequest, NativeBackend};
    let tmp = tempfile::tempdir().unwrap();
    let run = tmp.path().join("run");
    let server = Server::bind(&run, Arc::new(Host::new())).unwrap();
    let stop = server.stop_handle();
    let thread = std::thread::spawn(move || server.serve());
    let ssh = fake_ssh(tmp.path(), &run);

    let b = NativeBackend::remote_with_program(
        "laptop",
        "me@laptop",
        None,
        None,
        "hq host gate",
        None,
        ssh.to_str().unwrap(),
    );
    assert_eq!(b.name(), "laptop");
    assert!(!b.checks_binaries());
    assert!(!b.version().unwrap().is_empty());

    let req = LaunchRequest {
        name: "hs-remote".into(),
        kind: "pi".into(),
        cwd: std::env::temp_dir().to_string_lossy().into(),
        label: "t".into(),
        env: vec![],
        args: vec![],
        command: Some("f() { echo started; cat; }; f".into()),
        resume_args: None,
        start_timeout: Duration::from_secs(10),
    };
    assert!(b.launch(&req).unwrap().ready);
    b.send_text("hs-remote", "over-ssh").unwrap();
    let deadline = Instant::now() + WAIT;
    while !b.read("hs-remote", 20).unwrap().contains("over-ssh") {
        assert!(Instant::now() < deadline, "text never arrived");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(b.agents().unwrap().len(), 1);

    // Errors keep their codes across the gate.
    let again = b.launch(&req).unwrap_err().to_string();
    assert!(again.contains("name_taken"), "{again}");
    b.close_workspace("hs-remote").unwrap();
    assert!(b.agent("hs-remote").unwrap().is_none());

    stop.stop();
    thread.join().unwrap();
}

#[test]
fn the_gate_refuses_what_a_remote_key_must_not_do() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    for request in [
        r#"["host.stop","{}"]"#,
        r#"["agent.report","{}"]"#,
        r#"["agent.list","[1]"]"#,
        "not json",
    ] {
        let mut child = std::process::Command::new(hq())
            .args(["host", "gate"])
            .env("HQ_HOST_DIR", tmp.path())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(64), "{request}");
        assert!(out.stdout.is_empty(), "{request}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("gate_denied"));
    }
}

/// Real ssh to a host whose key is pinned to `hq host gate`. Needs a server
/// that is already set up, so it is opt-in:
/// `HQ_E2E_SSH=user@host HQ_E2E_SSH_KEY=<key> [HQ_E2E_SSH_PORT=<port>] cargo test -p hq-cli --test host_report -- --ignored`
/// with `hq host serve` running there.
#[test]
#[ignore = "needs a real ssh server with the key pinned to `hq host gate`; see the doc comment"]
fn a_real_ssh_connection_reaches_the_gate() {
    use hq_tools::herdr::{HostBackend, NativeBackend};
    use std::os::unix::fs::PermissionsExt;
    let target = std::env::var("HQ_E2E_SSH").expect("HQ_E2E_SSH");
    let key = std::env::var("HQ_E2E_SSH_KEY").expect("HQ_E2E_SSH_KEY");
    let port = std::env::var("HQ_E2E_SSH_PORT").unwrap_or_else(|_| "22".into());
    let tmp = tempfile::tempdir().unwrap();
    let wrapper = tmp.path().join("ssh-with-port");
    std::fs::write(
        &wrapper,
        format!("#!/bin/sh\nexec ssh -p {port} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \"$@\"\n"),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let b = NativeBackend::remote_with_program(
        "e2e",
        &target,
        None,
        Some(key),
        "hq host gate",
        None,
        wrapper.to_str().unwrap(),
    );
    assert!(!b.version().unwrap().is_empty());
    assert!(b.agents().is_ok());
    let denied = b.send_text("nobody", "x").unwrap_err().to_string();
    assert!(denied.contains("agent_not_found"), "{denied}");
}
