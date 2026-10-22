//! Starting, stopping and watching the local `hq` host (`hq start all`).
//!
//! The app owns exactly one thing here: the child process it started. It never runs `hq stop`
//! (which kills by name and by port, and would take down a daemon or a session it did not
//! start). A host that answers on the web port but is not this app's child is reported as
//! running elsewhere and cannot be stopped from here.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// `hq start all` waits up to 10 s for its cleanup before it exits.
const STOP_GRACE: Duration = Duration::from_secs(12);
const STOP_POLL: Duration = Duration::from_millis(100);
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const PROBE_READ_MAX: usize = 64;
const HQ_BINARY: &str = "hq";

#[derive(Debug, Clone)]
pub struct HostSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Where the host's output goes. Created owner-only.
    pub log: PathBuf,
    /// The web port the host answers `/health` on (`ws_port` in the HQ config).
    pub health_port: u16,
}

impl HostSpec {
    pub fn for_hq(program: PathBuf, log: PathBuf, health_port: u16) -> Self {
        Self {
            program,
            args: vec!["start".into(), "all".into()],
            log,
            health_port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HostStatus {
    Stopped,
    /// This app started it and it does not answer yet.
    Starting {
        pid: u32,
    },
    Running {
        pid: u32,
    },
    /// Something answers on the web port that this app did not start.
    External,
    /// This app started it and it ended by itself.
    Exited {
        code: Option<i32>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(
        "an HQ host already answers on this machine; this app did not start it and will not stop it"
    )]
    AlreadyRunning,
    #[error("the HQ host was not started by this app, so this app will not stop it")]
    NotOurs,
    #[error("the hq program was not found; looked in: {}", .0.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))]
    ProgramNotFound(Vec<PathBuf>),
    #[error("could not start the HQ host: {0}")]
    Spawn(String),
}

#[derive(Default)]
struct Inner {
    child: Option<Child>,
    last_exit: Option<Option<i32>>,
}

pub struct HostController {
    spec: HostSpec,
    inner: Mutex<Inner>,
}

impl HostController {
    pub fn new(spec: HostSpec) -> Self {
        Self {
            spec,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub async fn status(&self) -> HostStatus {
        let own = self.reap_and_pid();
        let answering = probe_health(self.spec.health_port).await;
        match own {
            Some(pid) if answering => HostStatus::Running { pid },
            Some(pid) => HostStatus::Starting { pid },
            None if answering => HostStatus::External,
            None => match self.inner.lock().unwrap().last_exit {
                Some(code) => HostStatus::Exited { code },
                None => HostStatus::Stopped,
            },
        }
    }

    pub async fn start(&self) -> Result<HostStatus, HostError> {
        if self.reap_and_pid().is_some() {
            return Ok(self.status().await);
        }
        if probe_health(self.spec.health_port).await {
            return Err(HostError::AlreadyRunning);
        }
        let child = spawn(&self.spec)?;
        let mut inner = self.inner.lock().unwrap();
        inner.last_exit = None;
        inner.child = Some(child);
        drop(inner);
        Ok(self.status().await)
    }

    /// Stops the child this app started: a polite signal to its whole process group, then a kill
    /// after the grace period. Refuses when the host was not started here.
    pub async fn stop(&self) -> Result<HostStatus, HostError> {
        let Some(mut child) = self.inner.lock().unwrap().child.take() else {
            return if probe_health(self.spec.health_port).await {
                Err(HostError::NotOurs)
            } else {
                Ok(HostStatus::Stopped)
            };
        };
        let pid = child.id();
        signal_group(pid, Signal::Term);
        let deadline = tokio::time::Instant::now() + STOP_GRACE;
        while child.try_wait().ok().flatten().is_none() {
            if tokio::time::Instant::now() >= deadline {
                signal_group(pid, Signal::Kill);
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            tokio::time::sleep(STOP_POLL).await;
        }
        self.inner.lock().unwrap().last_exit = None;
        Ok(self.status().await)
    }

    /// The pid of the live child, forgetting one that ended.
    fn reap_and_pid(&self) -> Option<u32> {
        let mut inner = self.inner.lock().unwrap();
        let child = inner.child.as_mut()?;
        match child.try_wait() {
            Ok(None) => Some(child.id()),
            Ok(Some(status)) => {
                inner.last_exit = Some(status.code());
                inner.child = None;
                None
            }
            Err(_) => None,
        }
    }
}

fn spawn(spec: &HostSpec) -> Result<Child, HostError> {
    let log = open_log(&spec.log).map_err(|e| HostError::Spawn(format!("log file: {e}")))?;
    let err = log
        .try_clone()
        .map_err(|e| HostError::Spawn(e.to_string()))?;
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err));
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    cmd.spawn().map_err(|e| HostError::Spawn(e.to_string()))
}

fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

enum Signal {
    Term,
    Kill,
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: Signal) {
    let signal = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // SAFETY: killpg only sends a signal; the group id is the pid of a child this process
    // spawned with `process_group(0)` and has not yet reaped.
    unsafe {
        libc::killpg(pid as libc::pid_t, signal);
    }
}

#[cfg(not(unix))]
fn signal_group(_pid: u32, _signal: Signal) {}

/// Whether anything answers `GET /health` with a success on this machine's loopback.
pub async fn probe_health(port: u16) -> bool {
    let attempt = async {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .ok()?;
        stream
            .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await
            .ok()?;
        let mut head = [0u8; PROBE_READ_MAX];
        let n = stream.read(&mut head).await.ok()?;
        let line = std::str::from_utf8(&head[..n]).ok()?;
        let status = line.split_whitespace().nth(1)?;
        Some(status.starts_with('2'))
    };
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, attempt).await,
        Ok(Some(true))
    )
}

/// Where the app looks for `hq`, in order: next to the app, `~/bin`, `/usr/local/bin`, then
/// every directory on `PATH`.
pub fn search_path(
    app_dir: Option<&Path>,
    home: Option<&Path>,
    path_var: Option<&std::ffi::OsStr>,
) -> Vec<PathBuf> {
    let mut tried: Vec<PathBuf> = Vec::new();
    tried.extend(app_dir.map(|d| d.join(HQ_BINARY)));
    tried.extend(home.map(|h| h.join("bin").join(HQ_BINARY)));
    tried.push(Path::new("/usr/local/bin").join(HQ_BINARY));
    if let Some(var) = path_var {
        tried.extend(std::env::split_paths(var).map(|d| d.join(HQ_BINARY)));
    }
    tried
}

/// The first runnable candidate. A configured path that is wrong is an error, not a reason to
/// quietly run some other `hq`.
pub fn locate_hq(configured: Option<&Path>, candidates: &[PathBuf]) -> Result<PathBuf, HostError> {
    if let Some(path) = configured {
        return if is_runnable(path) {
            Ok(path.to_path_buf())
        } else {
            Err(HostError::ProgramNotFound(vec![path.to_path_buf()]))
        };
    }
    candidates
        .iter()
        .find(|p| is_runnable(p))
        .cloned()
        .ok_or_else(|| HostError::ProgramNotFound(candidates.to_vec()))
}

fn is_runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Answers `GET /health` with 200 until dropped, on a port the OS chose.
    struct FakeHealth {
        port: u16,
        task: tokio::task::JoinHandle<()>,
    }

    impl FakeHealth {
        async fn up() -> Self {
            Self::on(0).await
        }

        async fn on(port: u16) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let task = tokio::spawn(async move {
                while let Ok((mut s, _)) = listener.accept().await {
                    let mut buf = [0u8; 256];
                    let _ = s.read(&mut buf).await;
                    let _ = s.write_all(b"HTTP/1.0 200 OK\r\n\r\nok").await;
                }
            });
            Self { port, task }
        }
    }

    impl Drop for FakeHealth {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn spec(dir: &Path, script: &str, health_port: u16) -> HostSpec {
        HostSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            log: dir.join("logs").join("host.log"),
            health_port,
        }
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    async fn gone(pid: i32) -> bool {
        for _ in 0..50 {
            if !alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    #[tokio::test]
    async fn start_runs_the_child_and_stop_ends_its_whole_group() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("grandchild.pid");
        let script = format!("sleep 60 & echo $! > {}; wait", pidfile.display());
        let port = free_port();
        let host = HostController::new(spec(dir.path(), &script, port));

        let HostStatus::Starting { pid } = host.start().await.unwrap() else {
            panic!("expected Starting");
        };
        let health = FakeHealth::on(port).await;
        assert_eq!(host.status().await, HostStatus::Running { pid });
        for _ in 0..50 {
            if pidfile.exists() && !std::fs::read_to_string(&pidfile).unwrap().trim().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let grandchild: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(grandchild));

        drop(health);
        let after = host.stop().await.unwrap();
        assert_eq!(after, HostStatus::Stopped);
        assert!(gone(pid as i32).await, "the child is still running");
        assert!(
            gone(grandchild).await,
            "the child's own child is still running"
        );
    }

    #[tokio::test]
    async fn a_host_this_app_did_not_start_is_reported_and_never_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let health = FakeHealth::up().await;
        let mut stranger = Command::new("sleep").arg("60").spawn().unwrap();
        let host = HostController::new(spec(dir.path(), "sleep 60", health.port));

        assert_eq!(host.status().await, HostStatus::External);
        assert!(matches!(host.stop().await, Err(HostError::NotOurs)));
        assert!(matches!(host.start().await, Err(HostError::AlreadyRunning)));
        assert!(alive(stranger.id() as i32), "the stranger was killed");
        stranger.kill().unwrap();
        stranger.wait().unwrap();
    }

    #[tokio::test]
    async fn a_child_that_ends_by_itself_is_reported_with_its_code() {
        let dir = tempfile::tempdir().unwrap();
        let host = HostController::new(spec(dir.path(), "exit 3", free_port()));
        host.start().await.unwrap();
        for _ in 0..50 {
            if matches!(host.status().await, HostStatus::Exited { .. }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(host.status().await, HostStatus::Exited { code: Some(3) });
        assert_eq!(host.stop().await.unwrap(), HostStatus::Stopped);
    }

    #[tokio::test]
    async fn a_child_that_does_not_answer_yet_is_starting() {
        let dir = tempfile::tempdir().unwrap();
        let host = HostController::new(spec(dir.path(), "sleep 60", free_port()));
        assert!(matches!(
            host.start().await.unwrap(),
            HostStatus::Starting { .. }
        ));
        assert_eq!(host.stop().await.unwrap(), HostStatus::Stopped);
    }

    #[tokio::test]
    async fn starting_twice_keeps_one_child() {
        let dir = tempfile::tempdir().unwrap();
        let host = HostController::new(spec(dir.path(), "sleep 60", free_port()));
        let HostStatus::Starting { pid: first } = host.start().await.unwrap() else {
            panic!("not starting");
        };
        let HostStatus::Starting { pid: second } = host.start().await.unwrap() else {
            panic!("not starting");
        };
        assert_eq!(first, second);
        host.stop().await.unwrap();
    }

    #[tokio::test]
    async fn the_log_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let s = spec(dir.path(), "echo hello", free_port());
        let log = s.log.clone();
        let host = HostController::new(s);
        host.start().await.unwrap();
        host.stop().await.unwrap();
        assert_eq!(
            std::fs::metadata(&log).unwrap().permissions().mode() & 0o077,
            0
        );
    }

    #[test]
    fn hq_is_found_in_order_and_a_wrong_configured_path_is_not_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let make = |dir: &Path| {
            std::fs::create_dir_all(dir).unwrap();
            let p = dir.join("hq");
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let root = tempfile::tempdir().unwrap();
        let app = make(&root.path().join("app"));
        let on_path = make(&root.path().join("onpath"));
        let path_var = std::env::join_paths([root.path().join("onpath")]).unwrap();
        let home = root.path().join("home");
        let candidates = vec![app.clone(), home.join("bin").join("hq"), on_path.clone()];

        assert_eq!(locate_hq(None, &candidates).unwrap(), app);
        std::fs::remove_file(&app).unwrap();
        assert_eq!(locate_hq(None, &candidates).unwrap(), on_path);

        let missing = root.path().join("nope");
        let err = locate_hq(Some(&missing), &candidates).unwrap_err();
        assert!(matches!(err, HostError::ProgramNotFound(ref p) if p == &vec![missing]));

        let order = search_path(Some(&root.path().join("app")), Some(&home), Some(&path_var));
        assert_eq!(order[0], app);
        assert_eq!(order[1], home.join("bin").join("hq"));
        assert_eq!(order[2], Path::new("/usr/local/bin/hq"));
        assert_eq!(order[3], on_path);
    }
}
