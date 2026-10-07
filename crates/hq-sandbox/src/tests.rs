use super::*;
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::process::{Command, Stdio};

fn sandbox(project: &Path) -> AgentSandbox {
    AgentSandbox {
        project: project.to_path_buf(),
        writable: vec![],
        writable_files: vec![],
        readonly_files: vec![],
        readonly_subpaths: vec![],
        denied_programs: vec![],
        denied_services: vec![],
        masked_files: vec![],
        hidden_dirs: vec![],
        hide_other_processes: false,
        network: Network::Full,
    }
}

fn argv(script: &str) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), script.into()]
}

/// Runs `script` inside the sandbox and returns its combined output.
fn inside(backend: &Backend, s: &AgentSandbox, script: &str) -> String {
    let wrapped = wrap(backend, s, &argv(script)).expect("the policy can be enforced here");
    let out = Command::new(&wrapped.program)
        .args(&wrapped.args)
        .stdin(Stdio::null())
        .output()
        .expect("the sandbox program starts");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A host directory like the real one: an operator token, a socket, and the
/// config files of this agent and a sibling.
struct Host {
    _tmp: tempfile::TempDir,
    dir: PathBuf,
    project: PathBuf,
    socket: PathBuf,
    own: PathBuf,
    sibling: PathBuf,
    _listener: UnixListener,
}

fn host() -> Host {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let dir = root.join("run");
    std::fs::create_dir_all(dir.join("mcp")).unwrap();
    std::fs::write(dir.join("operator.token"), "OPERATOR-TOKEN").unwrap();
    std::fs::write(dir.join("mcp/own.json"), "OWN-CONFIG").unwrap();
    std::fs::write(dir.join("mcp/sibling.json"), "SIBLING-TOKEN").unwrap();
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let socket = dir.join("host.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let accept = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        while let Ok((mut c, _)) = accept.accept() {
            let _ = c.write_all(b"hello-from-host");
        }
    });
    Host {
        own: dir.join("mcp/own.json"),
        sibling: dir.join("mcp/sibling.json"),
        _tmp: tmp,
        dir,
        project,
        socket,
        _listener: listener,
    }
}

fn hiding(h: &Host) -> AgentSandbox {
    let mut s = sandbox(&h.project);
    s.hidden_dirs = vec![HiddenDir {
        dir: h.dir.clone(),
        allow: vec![h.socket.clone(), h.own.clone()],
    }];
    s
}

macro_rules! need_backend {
    () => {
        match backend() {
            Some(b) => b,
            None => return,
        }
    };
}

#[test]
fn the_operator_token_and_a_siblings_config_cannot_be_read() {
    let backend = need_backend!();
    let h = host();
    let s = hiding(&h);
    let token = inside(backend, &s, &format!("cat {}", h.dir.join("operator.token").display()));
    assert!(!token.contains("OPERATOR-TOKEN"), "{token}");
    let sibling = inside(backend, &s, &format!("cat {}", h.sibling.display()));
    assert!(!sibling.contains("SIBLING-TOKEN"), "{sibling}");
}

#[test]
fn the_agents_own_config_and_the_host_socket_stay_usable() {
    let backend = need_backend!();
    let h = host();
    let s = hiding(&h);
    let own = inside(backend, &s, &format!("cat {}", h.own.display()));
    assert!(own.contains("OWN-CONFIG"), "{own}");
    let sock = inside(
        backend,
        &s,
        &format!(
            "python3 -c \"import socket; c=socket.socket(socket.AF_UNIX); c.connect('{}'); print(c.recv(50).decode())\"",
            h.socket.display()
        ),
    );
    assert!(sock.contains("hello-from-host"), "{sock}");
}

#[test]
fn writes_land_in_the_project_and_nowhere_else() {
    let backend = need_backend!();
    let h = host();
    let outside = tempfile::tempdir().unwrap().keep().canonicalize().unwrap();
    let mut s = sandbox(&h.project);
    s.hidden_dirs = vec![];
    let ok = inside(backend, &s, &format!("echo yes > {0}/new.txt && cat {0}/new.txt", h.project.display()));
    assert!(ok.contains("yes"), "{ok}");
    let _ = inside(backend, &s, &format!("echo no > {}/escaped.txt", outside.display()));
    assert!(!outside.join("escaped.txt").exists(), "a write outside the project got through");
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn other_processes_and_their_environments_are_hidden() {
    let backend = need_backend!();
    let h = host();
    let mut victim = Command::new("python3")
        .args(["-c", "import time; time.sleep(60)"])
        .env("VICTIM_MARKER", "victim-secret-xyz")
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(400));
    let mut s = sandbox(&h.project);
    s.hide_other_processes = true;
    let script = "ps eww -ax | grep -c victim-secret-xyz; true";
    // Positive control: without the sandbox the same command does see it.
    let outside = Command::new("/bin/sh").args(["-c", script]).output().unwrap();
    let outside = String::from_utf8_lossy(&outside.stdout).trim().to_string();
    let seen = inside(backend, &s, script);
    let _ = victim.kill();
    let _ = victim.wait();
    assert!(outside.parse::<u32>().is_ok_and(|n| n >= 1), "the control found nothing: {outside:?}");
    let count = seen.trim().lines().last().unwrap_or("").trim().parse::<u32>().unwrap_or(0);
    assert_eq!(count, 0, "a sibling's environment was visible: {seen}");
}

#[test]
fn network_off_refuses_a_direct_connection() {
    let backend = need_backend!();
    let h = host();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || while listener.accept().is_ok() {});
    let mut s = sandbox(&h.project);
    s.network = Network::Off;
    let out = inside(
        backend,
        &s,
        &format!("python3 -c \"import socket; socket.create_connection(('127.0.0.1',{port}),3); print('CONN'+'ECTED')\" 2>&1"),
    );
    assert!(!out.contains("CONNECTED"), "{out}");
}

#[test]
fn the_proxy_network_reaches_only_the_proxy_port() {
    let backend = need_backend!();
    if !matches!(backend, Backend::SandboxExec(_)) {
        return; // bubblewrap has no relay yet; see `bubblewrap_refuses_a_proxy_network`
    }
    let h = host();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let other = TcpListener::bind("127.0.0.1:0").unwrap();
    let (proxy_port, other_port) = (
        proxy.local_addr().unwrap().port(),
        other.local_addr().unwrap().port(),
    );
    std::thread::spawn(move || {
        while let Ok((mut c, _)) = proxy.accept() {
            let _ = c.write_all(b"proxy-here");
        }
    });
    std::thread::spawn(move || while other.accept().is_ok() {});
    let mut s = sandbox(&h.project);
    s.network = Network::Proxy { port: proxy_port, unix_sockets: vec![] };
    let connect = |port: u16| {
        inside(
            backend,
            &s,
            &format!("python3 -c \"import socket; c=socket.create_connection(('127.0.0.1',{port}),3); print(c.recv(20).decode() or 'CONN'+'ECTED')\" 2>&1"),
        )
    };
    assert!(connect(proxy_port).contains("proxy-here"), "the proxy port must work");
    let blocked = connect(other_port);
    assert!(!blocked.contains("CONNECTED") && !blocked.contains("proxy-here"), "{blocked}");
}

#[test]
fn a_file_can_be_replaced_atomically_when_it_and_its_temp_siblings_are_writable() {
    let backend = need_backend!();
    if !matches!(backend, Backend::SandboxExec(_)) {
        return;
    }
    let h = host();
    let home = h.project.parent().unwrap().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let config = home.join(".tool.json");
    std::fs::write(&config, "old").unwrap();
    let mut s = sandbox(&h.project);
    s.writable_files = vec![config.clone()];
    let script = format!(
        "echo new > {0}.tmp.123 && mv {0}.tmp.123 {0} && cat {0}; echo stray > {1}/stray.txt",
        config.display(),
        home.display()
    );
    let out = inside(backend, &s, &script);
    assert!(out.contains("new"), "{out}");
    assert!(!home.join("stray.txt").exists(), "only that file and its siblings were writable");
}

#[test]
fn bubblewrap_refuses_a_proxy_network_instead_of_pretending() {
    let mut s = sandbox(Path::new("/p"));
    s.network = Network::Proxy { port: 1, unix_sockets: vec![] };
    let err = bwrap_args(&s, &argv("true")).unwrap_err();
    assert!(err.0.contains("relay"), "{err}");
}

#[test]
fn the_profile_denies_before_it_allows_and_lists_every_rule() {
    let mut s = sandbox(Path::new("/proj"));
    s.writable = vec!["/w".into()];
    s.writable_files = vec!["/h/.c.json".into()];
    s.readonly_files = vec!["/w/ro".into()];
    s.masked_files = vec!["/m/secret".into()];
    s.hidden_dirs = vec![HiddenDir { dir: "/run".into(), allow: vec!["/run/sock".into()] }];
    s.hide_other_processes = true;
    s.network = Network::Proxy { port: 18080, unix_sockets: vec!["/run/sock".into()] };
    let p = seatbelt_profile(&s);
    let at = |needle: &str| p.find(needle).unwrap_or_else(|| panic!("{needle} missing in {p}"));
    assert!(at("(deny file-write*)") < at("(allow file-write*"));
    assert!(at("(deny file-read-data (subpath \"/run\"))") < at("(allow file-read-data (literal \"/run/sock\"))"));
    assert!(p.contains("(subpath \"/proj\")") && p.contains("(subpath \"/w\")"));
    assert!(p.contains("(literal \"/h/.c.json\")") && p.contains("(regex #\"^/h/\\.c\\.json\\..*\")"));
    assert!(p.contains("(deny file-write* (literal \"/w/ro\"))"));
    assert!(p.contains("(deny file-read* file-write* (literal \"/m/secret\"))"));
    assert!(p.contains("(deny process-info* (target others))"));
    assert!(at("(deny network-outbound)") < at("(allow network-outbound (remote ip \"localhost:18080\")"));
    assert!(p.contains("(literal \"/run/sock\")"));
}

#[test]
fn hiding_a_directory_never_hides_it_from_stat() {
    // Claude Code stats every parent of a file it reads and refuses what it cannot
    // examine, so only reading data is denied, never `file-read*` as a whole.
    let mut s = sandbox(Path::new("/proj"));
    s.hidden_dirs = vec![HiddenDir { dir: "/run".into(), allow: vec![] }];
    let p = seatbelt_profile(&s);
    assert!(p.contains("file-read-data") && !p.contains("(deny file-read* (subpath"), "{p}");
}

#[test]
fn bubblewrap_hides_a_directory_with_an_empty_tmpfs_and_binds_back_only_the_allowed_files() {
    let mut s = sandbox(Path::new("/proj"));
    s.hidden_dirs = vec![HiddenDir { dir: "/run".into(), allow: vec!["/run/sock".into()] }];
    s.network = Network::Off;
    let args: Vec<String> = bwrap_args(&s, &argv("true"))
        .unwrap()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let at = |a: &[&str]| args.windows(a.len()).position(|w| w == a).unwrap_or_else(|| panic!("{a:?} not in {args:?}"));
    assert!(at(&["--tmpfs", "/run"]) < at(&["--bind", "/run/sock", "/run/sock"]));
    assert!(args.contains(&"--unshare-net".to_string()) && args.contains(&"--unshare-pid".to_string()));
    assert_eq!(args[args.len() - 3..], ["/bin/sh", "-c", "true"]);
}

#[test]
fn canonical_resolves_dedups_and_puts_parents_first() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    let got = canonical([root.join("a/b"), root.join("a"), root.join("a/b/../b"), root.join("missing")]);
    assert_eq!(got, vec![root.join("a"), root.join("a/b")]);
}

#[test]
fn a_sandboxed_process_that_reads_nothing_still_exits_cleanly() {
    let backend = need_backend!();
    let h = host();
    let out = inside(backend, &sandbox(&h.project), "echo fine");
    assert!(out.contains("fine"), "{out}");
}

#[test]
fn an_allowance_inside_a_hidden_dir_survives_a_hidden_parent_listed_later() {
    let backend = need_backend!();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let inner = root.join("run");
    std::fs::create_dir(&inner).unwrap();
    std::fs::write(inner.join("mine.json"), "MINE-OK").unwrap();
    std::fs::write(inner.join("theirs.json"), "THEIRS-SECRET").unwrap();
    let mut s = sandbox(&root);
    s.hidden_dirs = vec![
        HiddenDir { dir: inner.clone(), allow: vec![inner.join("mine.json")] },
        HiddenDir { dir: root.clone(), allow: vec![] },
    ];
    let out = inside(backend, &s, &format!("cat {i}/mine.json; cat {i}/theirs.json", i = inner.display()));
    assert!(out.contains("MINE-OK"), "{out}");
    assert!(!out.contains("THEIRS-SECRET"), "{out}");
}

#[test]
fn readonly_subpaths_and_denied_programs_are_enforced() {
    let backend = need_backend!();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("hooks")).unwrap();
    let mut s = sandbox(&root);
    s.readonly_subpaths = vec![root.join("hooks")];
    let out = inside(
        backend,
        &s,
        &format!("echo x > {r}/hooks/pre-commit; echo y > {r}/ok && echo WROTE", r = root.display()),
    );
    assert!(!root.join("hooks/pre-commit").exists(), "{out}");
    assert!(out.contains("WROTE"), "{out}");
    if matches!(backend, Backend::SandboxExec(_)) {
        s.denied_programs = vec!["/usr/bin/open".into()];
        let out = inside(backend, &s, "/usr/bin/open -h && echo STARTED");
        assert!(!out.contains("STARTED"), "{out}");
    }
}
