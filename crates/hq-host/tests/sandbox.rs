//! Runs real processes under the host's sandbox and checks what they can and
//! cannot reach. They need the macOS sandbox program; elsewhere only the
//! fail-closed behaviour is checked.

use hq_host::{Allow, Host, HostError, ReadSource, SandboxMode, SandboxSpec, SpawnSpec};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(20);
const SECRET: &str = "OPERATOR-TOKEN-VALUE";

fn sandbox_available() -> bool {
    cfg!(target_os = "macos") && hq_sandbox::backend().is_some()
}

fn spec(allow: Vec<Allow>) -> SandboxSpec {
    SandboxSpec {
        mode: SandboxMode::Process,
        allow,
        writable: Vec::new(),
    }
}

/// A run directory shaped like the real one: an operator token, two agents'
/// MCP configs and hook files.
fn run_dir() -> tempfile::TempDir {
    // Not under the temporary directory: that is writable to the agent, and the
    // host refuses a run directory an agent could rename.
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let p = dir.path();
    for sub in ["mcp", "hooks"] {
        std::fs::create_dir(p.join(sub)).unwrap();
    }
    std::fs::write(p.join("operator.token"), SECRET).unwrap();
    for name in ["a", "b"] {
        std::fs::write(p.join("mcp").join(format!("{name}.json")), format!("mcp-of-{name}")).unwrap();
        std::fs::write(p.join("hooks").join(format!("{name}.json")), "{}").unwrap();
    }
    std::fs::write(p.join("host.sock"), "").unwrap();
    dir
}

fn host_for(dir: &Path) -> Host {
    let host = Host::new();
    host.set_run_dir(dir);
    host
}

fn run(host: &Host, name: &str, cwd: &Path, sandbox: SandboxSpec, script: &str) -> String {
    let mut spec = SpawnSpec::new(name, vec!["sh".into(), "-c".into(), script.into()], cwd);
    spec.sandbox = Some(sandbox);
    host.spawn(spec).unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let text = host.read(name, ReadSource::Recent, 0).unwrap();
        if text.contains("DONE") {
            return text;
        }
        assert!(Instant::now() < deadline, "never finished:\n{text}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_agent_cannot_read_the_hosts_secrets_but_keeps_its_own_files() {
    if !sandbox_available() {
        return;
    }
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let d = dir.path().display();
    let script = format!(
        "t() {{ if eval \"$2\" >/dev/null 2>&1; then echo \"$1=yes\"; else echo \"$1=no\"; fi; }}; \
         t token 'cat {d}/operator.token'; t other_mcp 'cat {d}/mcp/b.json'; t own_mcp 'cat {d}/mcp/a.json'; \
         t own_hooks 'cat {d}/hooks/a.json'; t listing 'ls {d}/mcp'; t socket 'test -S {d}/host.sock || test -e {d}/host.sock'; \
         t write_project 'echo x > ./ok'; t write_home 'echo x > \"$HOME/hq-host-sandbox-probe\"'; \
         t ssh 'ls \"$HOME/.ssh\"'; echo DONE"
    );
    let host = host_for(dir.path());
    let out = run(&host, "a", project.path(), spec(vec![]), &script);
    let _ = std::fs::remove_file(std::env::var("HOME").unwrap() + "/hq-host-sandbox-probe");
    assert!(!out.contains(SECRET));
    for (probe, allowed) in [
        ("token", false),
        ("other_mcp", false),
        ("own_mcp", true),
        ("own_hooks", true),
        ("listing", false),
        ("socket", true),
        ("write_project", true),
        ("write_home", false),
    ] {
        let want = format!("{probe}={}", if allowed { "yes" } else { "no" });
        assert!(out.contains(&want), "expected {want}; got:\n{out}");
    }
    host.kill("a").unwrap();
}

#[test]
fn network_goes_only_through_the_allowlist_and_denials_are_logged() {
    if !sandbox_available() {
        return;
    }
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    let up = upstream.local_addr().unwrap().port();
    let accept = std::thread::spawn(move || {
        let _ = upstream.accept();
    });
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let allow = vec![Allow {
        host: "localhost".into(),
        ports: vec![up],
        private: true,
    }];
    let script = format!(
        "P=${{HTTPS_PROXY##*:}}; \
         if nc -z -w2 127.0.0.1 {up} 2>/dev/null; then echo direct=yes; else echo direct=no; fi; \
         printf 'CONNECT evil.test:443 HTTP/1.1\\r\\n\\r\\n' | nc -w3 127.0.0.1 $P | head -1; \
         printf 'CONNECT localhost:{up} HTTP/1.1\\r\\n\\r\\n' | nc -w3 127.0.0.1 $P | head -1; \
         echo \"autoupdate=$DISABLE_AUTOUPDATER\"; echo DONE"
    );
    let host = host_for(dir.path());
    let out = run(&host, "a", project.path(), spec(allow), &script);
    assert!(out.contains("direct=no"), "{out}");
    assert!(out.contains("403 Forbidden"), "{out}");
    assert!(out.contains("200 Connection Established"), "{out}");
    assert!(out.contains("autoupdate=1"), "{out}");
    let log = host.egress_decisions("a");
    assert!(log.iter().any(|d| d.host == "evil.test" && !d.allowed), "{log:?}");
    assert!(log.iter().any(|d| d.host == "localhost" && d.allowed), "{log:?}");
    host.kill("a").unwrap();
    drop(accept);
}

#[test]
fn a_sibling_environment_is_not_visible() {
    if !sandbox_available() {
        return;
    }
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let host = host_for(dir.path());
    let mut other = SpawnSpec::new("b", vec!["sh".into(), "-c".into(), "sleep 60".into()], project.path());
    other.env = vec![("SIBLING_SECRET".into(), SECRET.into())];
    host.spawn(other).unwrap();
    let out = run(&host, "a", project.path(), spec(vec![]), "ps eww -ax 2>&1 | grep -c SIBLING_SECRET; echo DONE");
    assert!(!out.contains(SECRET));
    assert!(out.lines().any(|l| l.trim() == "0") || out.contains("not permitted"), "{out}");
    host.kill("a").unwrap();
    host.kill("b").unwrap();
}

#[test]
fn the_listener_closes_when_the_agent_exits() {
    if !sandbox_available() {
        return;
    }
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let host = host_for(dir.path());
    run(&host, "a", project.path(), spec(vec![]), "echo $HTTPS_PROXY; echo DONE");
    host.wait_exit("a", WAIT).unwrap();
    let text = host.read("a", ReadSource::Recent, 0).unwrap();
    let port: u16 = text.lines().find_map(|l| l.trim().rsplit(':').next()?.parse().ok()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    assert!(std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err());
}

#[test]
fn a_sandbox_that_cannot_be_applied_is_an_error_not_an_unsandboxed_start() {
    let host = Host::new();
    let mut s = SpawnSpec::new("a", vec!["sleep".into(), "1".into()], std::env::temp_dir());
    s.sandbox = Some(spec(vec![]));
    assert!(matches!(host.spawn(s), Err(HostError::Sandbox(_))), "no run directory");
    assert!(host.list().is_empty());
    if !sandbox_available() {
        let dir = run_dir();
        let host = host_for(dir.path());
        let mut s = SpawnSpec::new("a", vec!["sleep".into(), "1".into()], std::env::temp_dir());
        s.sandbox = Some(spec(vec![]));
        assert!(matches!(host.spawn(s), Err(HostError::Sandbox(_))));
    }
}

#[test]
fn mode_none_runs_unsandboxed_and_says_so() {
    let host = Host::new();
    let mut s = SpawnSpec::new("a", vec!["sleep".into(), "30".into()], std::env::temp_dir());
    s.sandbox = Some(SandboxSpec::none());
    let info = host.spawn(s).unwrap();
    assert_eq!(info.sandbox, "none");
    host.kill("a").unwrap();
    let plain = host
        .spawn(SpawnSpec::new("b", vec!["sleep".into(), "30".into()], std::env::temp_dir()))
        .unwrap();
    assert_eq!(plain.sandbox, "none");
    host.kill("b").unwrap();
}

#[test]
fn a_writable_root_that_contains_the_host_directory_or_home_is_refused() {
    if !sandbox_available() {
        return;
    }
    let dir = run_dir();
    let host = host_for(dir.path());
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    for cwd in [home, dir.path().parent().unwrap().to_path_buf()] {
        let mut s = SpawnSpec::new("a", vec!["sleep".into(), "1".into()], &cwd);
        s.sandbox = Some(spec(vec![]));
        let err = host.spawn(s).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.contains("contains"), "{}: {err}", cwd.display());
    }
    assert!(host.list().is_empty());
}

#[test]
fn the_agent_cannot_plant_code_the_operator_runs_later() {
    if !sandbox_available() {
        return;
    }
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join(".git/hooks")).unwrap();
    let host = host_for(dir.path());
    let script = "t() { if eval \"$2\" >/dev/null 2>&1; then echo \"$1=yes\"; else echo \"$1=no\"; fi; }; \
        t hook 'echo x > .git/hooks/pre-commit'; t mcp 'echo {} > .mcp.json'; t cfg 'echo x >> .git/config'; \
        t src 'echo y > src.txt'; t open '/usr/bin/open -h'; echo DONE";
    let out = run(&host, "a", project.path(), spec(vec![]), script);
    for (probe, allowed) in [("hook", false), ("mcp", false), ("cfg", false), ("src", true), ("open", false)] {
        let want = format!("{probe}={}", if allowed { "yes" } else { "no" });
        assert!(out.contains(&want), "expected {want}; got:\n{out}");
    }
    host.kill("a").unwrap();
}
