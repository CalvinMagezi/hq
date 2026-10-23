//! Runs real processes under the host's sandbox and checks what they can and
//! cannot reach. They need a working sandbox program (`sandbox-exec` on macOS,
//! `bwrap` on Linux, with user namespaces); without one only the fail-closed
//! behaviour is checked.
#![cfg(unix)]

use hq_host::{
    Allow, Host, HostError, ReadSource, SandboxMode, SandboxProfile, SandboxSpec, SpawnSpec,
};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(20);
const SECRET: &str = "OPERATOR-TOKEN-VALUE";

fn sandbox_available() -> bool {
    hq_sandbox::backend().is_some()
}

fn spec(allow: Vec<Allow>) -> SandboxSpec {
    SandboxSpec {
        mode: SandboxMode::Process,
        allow,
        writable: Vec::new(),
        readable: Vec::new(),
        ..SandboxSpec::none()
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
        std::fs::write(
            p.join("mcp").join(format!("{name}.json")),
            format!("mcp-of-{name}"),
        )
        .unwrap();
        std::fs::write(p.join("hooks").join(format!("{name}.json")), "{}").unwrap();
    }
    for sock in ["host.sock", "agent.sock"] {
        std::fs::write(p.join(sock), "").unwrap();
    }
    dir
}

fn host_for(dir: &Path) -> Host {
    let host = Host::new().with_helper_binary(env!("CARGO_BIN_EXE_hq"));
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
         t own_hooks 'cat {d}/hooks/a.json'; t listing 'ls {d}/mcp'; t sibling_listed 'ls {d}/mcp | grep -q b.json'; t socket 'test -e {d}/agent.sock'; \
         t write_project 'echo x > ./ok'; t write_home 'echo x > \"$HOME/hq-host-sandbox-probe\"'; \
         t ssh 'ls \"$HOME/.ssh\"'; t home_other 'cat \"$HOME/hq-host-readprobe\"'; t users_list 'ls /Users'; t volumes_list 'ls /Volumes'; t shared 'ls /Users/Shared'; echo DONE"
    );
    let home = std::env::var("HOME").unwrap();
    std::fs::write(format!("{home}/hq-host-readprobe"), "x").unwrap();
    let host = host_for(dir.path());
    let out = run(&host, "a", project.path(), spec(vec![]), &script);
    assert!(
        !Path::new(&format!("{home}/hq-host-sandbox-probe")).exists(),
        "a write under home reached the real home directory"
    );
    let _ = std::fs::remove_file(format!("{home}/hq-host-sandbox-probe"));
    let _ = std::fs::remove_file(format!("{home}/hq-host-readprobe"));
    assert!(!out.contains(SECRET));
    for (probe, allowed) in [
        ("token", false),
        ("other_mcp", false),
        ("own_mcp", true),
        ("own_hooks", true),
        // Seatbelt refuses the listing; bubblewrap shows a directory holding only
        // the agent's own file. Either way a sibling's file is not in it.
        ("listing", cfg!(target_os = "linux")),
        ("sibling_listed", false),
        ("socket", true),
        ("write_project", true),
        // Seatbelt denies the write; bubblewrap lets it land in a throwaway tmpfs.
        ("write_home", cfg!(target_os = "linux")),
        ("home_other", false),
        ("users_list", false),
        ("volumes_list", false),
        ("shared", cfg!(target_os = "macos")),
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
    assert!(
        log.iter().any(|d| d.host == "evil.test" && !d.allowed),
        "{log:?}"
    );
    assert!(
        log.iter().any(|d| d.host == "localhost" && d.allowed),
        "{log:?}"
    );
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
    let mut other = SpawnSpec::new(
        "b",
        vec!["sh".into(), "-c".into(), "sleep 60".into()],
        project.path(),
    );
    other.env = vec![("SIBLING_SECRET".into(), SECRET.into())];
    host.spawn(other).unwrap();
    let out = run(
        &host,
        "a",
        project.path(),
        spec(vec![]),
        "ps eww -ax 2>&1 | grep -c SIBLING_SECRET; echo DONE",
    );
    assert!(!out.contains(SECRET));
    assert!(
        out.lines().any(|l| l.trim() == "0") || out.contains("not permitted"),
        "{out}"
    );
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
    run(
        &host,
        "a",
        project.path(),
        spec(vec![]),
        "echo $HTTPS_PROXY; echo DONE",
    );
    host.wait_exit("a", WAIT).unwrap();
    let text = host.read("a", ReadSource::Recent, 0).unwrap();
    let port: u16 = text
        .lines()
        .find_map(|l| l.trim().rsplit(':').next()?.parse().ok())
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    assert!(std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err());
}

#[test]
fn a_sandbox_that_cannot_be_applied_is_an_error_not_an_unsandboxed_start() {
    let host = Host::new();
    let mut s = SpawnSpec::new("a", vec!["sleep".into(), "1".into()], std::env::temp_dir());
    s.sandbox = Some(spec(vec![]));
    assert!(
        matches!(host.spawn(s), Err(HostError::Sandbox(_))),
        "no run directory"
    );
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
        .spawn(SpawnSpec::new(
            "b",
            vec!["sleep".into(), "30".into()],
            std::env::temp_dir(),
        ))
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
        let err = host
            .spawn(s)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
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
    // bubblewrap can only protect what exists, as real projects have it.
    std::fs::write(project.path().join(".git/config"), "[core]\n").unwrap();
    std::fs::write(project.path().join(".mcp.json"), "{}").unwrap();
    let host = host_for(dir.path());
    let script = "t() { if eval \"$2\" >/dev/null 2>&1; then echo \"$1=yes\"; else echo \"$1=no\"; fi; }; \
        t hook 'echo x > .git/hooks/pre-commit'; t mcp 'echo {} > .mcp.json'; t cfg 'echo x >> .git/config'; \
        t src 'echo y > src.txt'; t open '/usr/bin/open -h'; echo DONE";
    let out = run(&host, "a", project.path(), spec(vec![]), script);
    for (probe, allowed) in [
        ("hook", false),
        ("mcp", false),
        ("cfg", false),
        ("src", true),
        ("open", false),
    ] {
        let want = format!("{probe}={}", if allowed { "yes" } else { "no" });
        assert!(out.contains(&want), "expected {want}; got:\n{out}");
    }
    host.kill("a").unwrap();
}

#[test]
fn a_launch_profile_keeps_its_own_claude_config_and_loses_the_default_one() {
    if !sandbox_available() {
        return;
    }
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    let pid = std::process::id();
    let mine = home.join(format!(".claude-hqtest-mine-{pid}"));
    let other = home.join(format!(".claude-hqtest-other-{pid}"));
    for d in [&mine, &other] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(mine.join("credentials"), "MINE").unwrap();
    // bubblewrap can only protect what exists, as a real profile has it.
    std::fs::write(mine.join("settings.json"), "{}").unwrap();
    std::fs::write(mine.join(".claude.json"), "{}").unwrap();
    std::fs::create_dir(mine.join("hooks")).unwrap();
    std::fs::write(other.join("credentials"), "OTHER").unwrap();
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let host = host_for(dir.path());
    let script = "t() { if eval \"$2\" >/dev/null 2>&1; then echo \"$1=yes\"; else echo \"$1=no\"; fi; }; \
        t read_own 'cat \"$CLAUDE_CONFIG_DIR/credentials\"'; t write_own 'echo x > \"$CLAUDE_CONFIG_DIR/state\"'; \
        t write_settings 'echo {} > \"$CLAUDE_CONFIG_DIR/settings.json\"'; t write_json 'echo {} > \"$CLAUDE_CONFIG_DIR/.claude.json\"'; \
        t write_hooks 'echo x > \"$CLAUDE_CONFIG_DIR/hooks/pre.sh\"'; \
        t read_default 'ls \"$HOME/.claude\" | grep -q .'; t read_other 'cat \"$HOME/.claude-hqtest-other-PID/credentials\"'; echo DONE"
        .replace("PID", &pid.to_string());
    let mut s = SpawnSpec::new("a", vec!["sh".into(), "-c".into(), script], project.path());
    s.env = vec![(
        "CLAUDE_CONFIG_DIR".into(),
        mine.to_string_lossy().into_owned(),
    )];
    s.sandbox = Some(spec(vec![]));
    host.spawn(s).unwrap();
    let deadline = Instant::now() + WAIT;
    let out = loop {
        let text = host.read("a", ReadSource::Recent, 0).unwrap();
        if text.contains("DONE") {
            break text;
        }
        assert!(Instant::now() < deadline, "never finished:\n{text}");
        std::thread::sleep(Duration::from_millis(50));
    };
    for d in [&mine, &other] {
        let _ = std::fs::remove_dir_all(d);
    }
    let default_exists = home.join(".claude").is_dir();
    for (probe, allowed) in [
        ("read_own", true),
        ("write_own", true),
        ("write_settings", false),
        ("write_json", false),
        ("write_hooks", false),
        ("read_other", false),
    ] {
        let want = format!("{probe}={}", if allowed { "yes" } else { "no" });
        assert!(out.contains(&want), "expected {want}; got:\n{out}");
    }
    if default_exists {
        assert!(out.contains("read_default=no"), "{out}");
    }
    host.kill("a").unwrap();
}

#[test]
fn an_agent_may_write_its_own_state_directory_and_no_other_agents() {
    if !sandbox_available() {
        return;
    }
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let host = host_for(dir.path());
    let probe = format!("hq-state-probe-{}", std::process::id());
    let script = format!(
        "t() {{ if eval \"$2\" >/dev/null 2>&1; then echo \"$1=yes\"; else echo \"$1=no\"; fi; }}; \
         t own 'echo x > \"$HOME/.gemini/{probe}\"'; t other 'echo x > \"$HOME/.codex/{probe}\"'; echo DONE"
    );
    let mut s = SpawnSpec::new("a", vec!["sh".into(), "-c".into(), script], project.path());
    s.agent = Some("agy".into());
    s.sandbox = Some(spec(vec![]));
    host.spawn(s).unwrap();
    let deadline = Instant::now() + WAIT;
    let out = loop {
        let text = host.read("a", ReadSource::Recent, 0).unwrap();
        if text.contains("DONE") {
            break text;
        }
        assert!(Instant::now() < deadline, "never finished:\n{text}");
        std::thread::sleep(Duration::from_millis(50));
    };
    let own = home.join(".gemini").join(&probe);
    let other = home.join(".codex").join(&probe);
    let (own_written, other_written) = (own.exists(), other.exists());
    let _ = std::fs::remove_file(&own);
    let _ = std::fs::remove_file(&other);
    assert!(out.contains("own=yes") && own_written, "{out}");
    assert!(!out.contains("other=yes") && !other_written, "{out}");
    host.kill("a").unwrap();
}

fn run_as(host: &Host, agent: &str, cwd: &Path, sandbox: SandboxSpec, script: &str) -> String {
    let mut spec = SpawnSpec::new(agent, vec!["sh".into(), "-c".into(), script.into()], cwd);
    spec.agent = Some(agent.into());
    spec.sandbox = Some(sandbox);
    host.spawn(spec).unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let text = host.read(agent, ReadSource::Recent, 0).unwrap();
        if text.contains("DONE") {
            return text;
        }
        assert!(Instant::now() < deadline, "never finished:\n{text}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_credential_directory_named_for_one_agent_is_readable_by_it_alone_and_never_writable() {
    if !sandbox_available() {
        return;
    }
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    // Another Claude profile is one of the directories hidden by default.
    let secret = home.join(format!(".claude-hqtest-reveal-{}", std::process::id()));
    std::fs::create_dir_all(&secret).unwrap();
    std::fs::write(secret.join("hosts.yml"), "REVEALED").unwrap();
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let host = host_for(dir.path());
    let mut sandbox = spec(vec![]);
    // HQ's own directory is never revealed, even when named.
    let hq_dir = home.join(".hq");
    sandbox
        .agent_readable
        .insert("hqtest-reader".into(), vec![secret.clone(), hq_dir.clone()]);
    let script = format!(
        "t() {{ if eval \"$2\" >/dev/null 2>&1; then echo \"$1=yes\"; else echo \"$1=no\"; fi; }}; \
         t read 'grep -q REVEALED \"{0}/hosts.yml\"'; t write 'echo x > \"{0}/hosts.yml\"'; t hq 'ls \"{1}\" | grep -q .'; echo DONE",
        secret.display(),
        hq_dir.display()
    );
    // A writable root nested inside it would open it up, so that launch is refused.
    let nested = secret.join("sub");
    std::fs::create_dir_all(&nested).unwrap();
    let mut planting = sandbox.clone();
    planting.writable = vec![nested];
    let mut s = SpawnSpec::new("hqtest-reader", vec!["true".into()], project.path());
    s.agent = Some("hqtest-reader".into());
    s.sandbox = Some(planting);
    let refused = host.spawn(s);
    let reader = run_as(
        &host,
        "hqtest-reader",
        project.path(),
        sandbox.clone(),
        &script,
    );
    let other = run_as(&host, "hqtest-other", project.path(), sandbox, &script);
    let untouched = std::fs::read_to_string(secret.join("hosts.yml")).unwrap();
    let _ = std::fs::remove_dir_all(&secret);
    assert!(
        reader.contains("read=yes") && reader.contains("write=no"),
        "{reader}"
    );
    let refused =
        refused.expect_err("a writable root inside a credential directory must be refused");
    assert!(refused.to_string().contains("lies inside"), "{refused}");
    // Seatbelt refuses a write into a hidden directory; bubblewrap lets it land in the
    // empty mount that hides it, and `untouched` below shows the real file is not reached.
    assert!(
        other.contains("read=no") && (other.contains("write=no") || cfg!(target_os = "linux")),
        "{other}"
    );
    if hq_dir.is_dir() {
        assert!(reader.contains("hq=no"), "{reader}");
    }
    assert_eq!(untouched, "REVEALED");
    host.kill("hqtest-reader").unwrap();
    host.kill("hqtest-other").unwrap();
}

#[test]
fn a_listed_loopback_port_is_reachable_on_macos_and_an_unlisted_one_is_not() {
    if !sandbox_available() || !cfg!(target_os = "macos") {
        return;
    }
    let listen = || {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            while let Ok((mut c, _)) = l.accept() {
                let _ = std::io::Write::write_all(&mut c, b"service-here");
            }
        });
        port
    };
    let (listed, unlisted) = (listen(), listen());
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let host = host_for(dir.path());
    let mut sandbox = spec(vec![]);
    sandbox.loopback_ports = vec![listed];
    let probe = |port: u16| {
        format!(
            "python3 -c \"import socket; c=socket.create_connection(('127.0.0.1',{port}),3); print('p{port}=' + c.recv(20).decode())\" 2>&1"
        )
    };
    let script = format!("{}; {}; echo DONE", probe(listed), probe(unlisted));
    let out = run(&host, "a", project.path(), sandbox, &script);
    assert!(out.contains(&format!("p{listed}=service-here")), "{out}");
    assert!(!out.contains(&format!("p{unlisted}=service-here")), "{out}");
    host.kill("a").unwrap();
}

fn open() -> SandboxSpec {
    SandboxSpec {
        mode: SandboxMode::Process,
        profile: SandboxProfile::Open,
        ..SandboxSpec::none()
    }
}

#[test]
fn the_open_profile_uses_the_machines_network_directly_and_opens_no_proxy() {
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
    let script = format!(
        "if nc -z -w2 127.0.0.1 {up} 2>/dev/null; then echo direct=yes; else echo direct=no; fi; \
         echo \"proxy=[$HTTPS_PROXY$HTTP_PROXY$NODE_USE_ENV_PROXY]\"; \
         echo \"autoupdate=$DISABLE_AUTOUPDATER\"; echo DONE"
    );
    let host = host_for(dir.path());
    let out = run(&host, "a", project.path(), open(), &script);
    assert!(out.contains("direct=yes"), "{out}");
    assert!(
        out.contains("proxy=[]"),
        "no proxy variables are set: {out}"
    );
    assert!(out.contains("autoupdate=1"), "{out}");
    assert!(
        host.egress_decisions("a").is_empty(),
        "there is no egress listener to log through"
    );
    host.kill("a").unwrap();
    drop(accept);
}

#[test]
fn the_open_profile_keeps_hq_out_of_reach_and_lets_the_project_be_set_up() {
    if !sandbox_available() {
        return;
    }
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join(".git/hooks")).unwrap();
    std::fs::write(project.path().join(".git/config"), "[core]\n").unwrap();
    std::fs::write(project.path().join(".mcp.json"), "{}").unwrap();
    let shared = std::env::temp_dir().join(format!("hq-open-shared-{}", std::process::id()));
    std::fs::write(&shared, "SHARED-FILE").unwrap();
    let written = std::env::temp_dir().join(format!("hq-open-written-{}", std::process::id()));
    let d = dir.path().display();
    let script = format!(
        "t() {{ if eval \"$2\" >/dev/null 2>&1; then echo \"$1=yes\"; else echo \"$1=no\"; fi; }}; \
         t token 'grep -q {SECRET} {d}/operator.token'; t other_mcp 'grep -q {SECRET} {d}/mcp/b.json'; \
         t own_mcp 'cat {d}/mcp/a.json'; t socket 'test -e {d}/agent.sock'; \
         t hook 'echo x > .git/hooks/pre-commit'; t mcp 'echo {{}} > .mcp.json'; t cfg 'echo x >> .git/config'; \
         t shared_read 'grep -q SHARED-FILE {shared}'; t shared_write 'echo x > {written}'; echo DONE",
        shared = shared.display(),
        written = written.display()
    );
    let host = host_for(dir.path());
    let out = run(&host, "a", project.path(), open(), &script);
    let wrote_shared = written.exists();
    let _ = std::fs::remove_file(&shared);
    let _ = std::fs::remove_file(&written);
    assert!(!out.contains(SECRET));
    for (probe, allowed) in [
        ("token", false),
        ("other_mcp", false),
        ("own_mcp", true),
        ("socket", true),
        // The strict profile refuses these three, which is what stops `git push -u`
        // from recording an upstream and a repository's own hook installer.
        ("hook", true),
        ("mcp", true),
        ("cfg", true),
        ("shared_read", true),
        ("shared_write", true),
    ] {
        let want = format!("{probe}={}", if allowed { "yes" } else { "no" });
        assert!(out.contains(&want), "expected {want}; got:\n{out}");
    }
    assert!(
        wrote_shared,
        "/tmp is the machine's own under the open profile"
    );
    host.kill("a").unwrap();
}

#[test]
fn the_open_profile_still_hides_a_siblings_environment() {
    if !sandbox_available() {
        return;
    }
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let host = host_for(dir.path());
    let mut other = SpawnSpec::new(
        "b",
        vec!["sh".into(), "-c".into(), "sleep 60".into()],
        project.path(),
    );
    other.env = vec![("SIBLING_SECRET".into(), SECRET.into())];
    host.spawn(other).unwrap();
    // Linux: every other process's /proc/<pid>/environ; macOS: `ps eww`.
    let out = run(
        &host,
        "a",
        project.path(),
        open(),
        "cat /proc/[0-9]*/environ 2>/dev/null | tr '\\0' '\\n' | grep -c SIBLING_SECRET; \
         ps eww -ax 2>&1 | grep -c SIBLING_SECRET; echo DONE",
    );
    assert!(!out.contains(SECRET));
    assert!(
        out.lines().filter(|l| l.trim() == "0").count() >= 1 || out.contains("not permitted"),
        "{out}"
    );
    host.kill("a").unwrap();
    host.kill("b").unwrap();
}

#[test]
fn the_profile_is_reported_per_agent() {
    if !sandbox_available() {
        return;
    }
    let dir = run_dir();
    let project = tempfile::tempdir().unwrap();
    let host = host_for(dir.path());
    for (name, sandbox, mode, profile) in [
        ("o", open(), "process", "open"),
        ("s", spec(vec![]), "process", "strict"),
        ("n", SandboxSpec::none(), "none", ""),
    ] {
        let mut s = SpawnSpec::new(name, vec!["sleep".into(), "30".into()], project.path());
        s.sandbox = Some(sandbox);
        let info = host.spawn(s).unwrap();
        assert_eq!(
            (info.sandbox, info.sandbox_profile),
            (mode, profile),
            "{name}"
        );
        host.kill(name).unwrap();
    }
}
