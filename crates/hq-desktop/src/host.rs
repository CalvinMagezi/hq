//! Starting, stopping and watching the local `hq` host: `hq start all`, or `hq host serve` (coding
//! agents only) on a laptop whose HQ lives on a paired phone.
//!
//! The app owns exactly one thing here: the child process it started. It never runs `hq stop`
//! (which kills by name and by port, and would take down a daemon or a session it did not
//! start). A host that answers its probe but is not this app's child is reported as
//! running elsewhere and cannot be stopped from here.

use hq_core::config::{HOST_SOCKET_FILE, HqConfig};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// `hq start all` waits up to 10 s for its cleanup before it exits.
pub const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(12);
const STOP_POLL: Duration = Duration::from_millis(100);
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);
const PROBE_READ_MAX: usize = 64;
const HQ_BINARY: &str = "hq";
/// What `hq --version` starts with, the same check the updater makes.
const VERSION_PREFIX: &str = "hq ";
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
/// A slow shell profile (version managers, plugin loaders) can take seconds.
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(5);
const PATH_MARKER: &str = "__HQ_DESKTOP_PATH__";

/// How the app tells that a host is up, which depends on what it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// `GET /health` on this web port (`hq start all`).
    Health(u16),
    /// A connection to the control socket (`hq host serve`, which has no web server).
    Socket(PathBuf),
}

#[derive(Debug, Clone)]
pub struct HostSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Where the host's output goes. Created owner-only.
    pub log: PathBuf,
    pub probe: Probe,
    /// How long a stop waits after the polite signal before it kills what is left.
    pub stop_grace: Duration,
}

impl HostSpec {
    pub fn for_hq(program: PathBuf, log: PathBuf, health_port: u16) -> Self {
        Self {
            program,
            args: vec!["start".into(), "all".into()],
            log,
            probe: Probe::Health(health_port),
            stop_grace: DEFAULT_STOP_GRACE,
        }
    }

    /// `hq host serve`: the coding-agent host alone, for a laptop whose HQ is on a paired phone.
    pub fn for_agents_only(program: PathBuf, log: PathBuf, socket: PathBuf) -> Self {
        Self {
            program,
            args: vec!["host".into(), "serve".into()],
            log,
            probe: Probe::Socket(socket),
            stop_grace: DEFAULT_STOP_GRACE,
        }
    }

    /// The host this machine needs: agents only when it serves a hub, else all of HQ.
    pub fn for_config(program: PathBuf, log: PathBuf, config: &HqConfig) -> Self {
        if config.bridge.serves_hub() {
            let socket = hq_core::config::native_host_dir().join(HOST_SOCKET_FILE);
            Self::for_agents_only(program, log, socket)
        } else {
            Self::for_hq(program, log, config.ws_port)
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
    /// Something answers where the host would that this app did not start.
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
    #[error("the app is closing")]
    Closing,
}

#[derive(Default)]
struct Inner {
    child: Option<Child>,
    last_exit: Option<Option<i32>>,
    /// The process group of the host this app started. It outlives the leader while any member
    /// lives, so a stop can still reach the survivors, and is forgotten the moment the group is
    /// empty, because a group id can be reused once it has no member.
    group: Option<u32>,
}

pub struct HostController {
    spec: HostSpec,
    inner: Mutex<Inner>,
    /// Held for the whole of a start or a stop, so two clicks cannot start two hosts and a start
    /// cannot slip in while a stop is still waiting.
    gate: tokio::sync::Mutex<()>,
    /// Set by the final stop, under the gate, so nothing starts a host while the app is closing.
    closed: std::sync::atomic::AtomicBool,
}

/// After a kill, how long a stop waits for the group to go before it reports what it sees.
const KILL_WAIT: Duration = Duration::from_secs(2);

impl HostController {
    pub fn new(spec: HostSpec) -> Self {
        Self {
            spec,
            inner: Mutex::new(Inner::default()),
            gate: tokio::sync::Mutex::new(()),
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub async fn status(&self) -> HostStatus {
        let own = self.reap_and_pid();
        let answering = host_answers(&self.spec.probe).await;
        let last_exit = self.inner.lock().unwrap().last_exit;
        match own {
            Some(pid) if answering => HostStatus::Running { pid },
            Some(pid) => HostStatus::Starting { pid },
            None if answering => HostStatus::External,
            None => match last_exit {
                Some(code) => HostStatus::Exited { code },
                None => HostStatus::Stopped,
            },
        }
    }

    pub async fn start(&self) -> Result<HostStatus, HostError> {
        let _gate = self.gate.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(HostError::Closing);
        }
        if self.reap_and_pid().is_some() {
            return Ok(self.status().await);
        }
        if host_answers(&self.spec.probe).await {
            return Err(HostError::AlreadyRunning);
        }
        let child = spawn(&self.spec)?;
        {
            let mut inner = self.inner.lock().unwrap();
            inner.last_exit = None;
            inner.group = Some(child.id());
            inner.child = Some(child);
        }
        Ok(self.status().await)
    }

    /// Stops the host this app started: a polite signal to its whole process group, then a kill
    /// of whatever is left of the group once the grace period is over, however the leader fared.
    /// Refuses when the host was not started here.
    pub async fn stop(&self) -> Result<HostStatus, HostError> {
        let _gate = self.gate.lock().await;
        self.stop_locked().await
    }

    /// The stop for when the app quits: nothing can start a host afterwards.
    pub async fn stop_for_good(&self) -> Result<HostStatus, HostError> {
        let _gate = self.gate.lock().await;
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        self.stop_locked().await
    }

    async fn stop_locked(&self) -> Result<HostStatus, HostError> {
        let Some(group) = self
            .reap_and_pid()
            .or_else(|| self.inner.lock().unwrap().group)
        else {
            return if host_answers(&self.spec.probe).await {
                Err(HostError::NotOurs)
            } else {
                // Nothing of ours is running; stopping also clears a note that it ended by itself.
                self.inner.lock().unwrap().last_exit = None;
                Ok(HostStatus::Stopped)
            };
        };
        signal_group(group, Signal::Term);
        let mut deadline = tokio::time::Instant::now() + self.spec.stop_grace;
        let mut killed = false;
        loop {
            let leader_gone = self.reap_and_pid().is_none();
            if leader_gone && !group_alive(group) {
                break;
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                if killed {
                    break;
                }
                signal_group(group, Signal::Kill);
                killed = true;
                deadline = now + KILL_WAIT;
            }
            tokio::time::sleep(STOP_POLL).await;
        }
        {
            let mut inner = self.inner.lock().unwrap();
            inner.last_exit = None;
            if !group_alive(group) {
                inner.group = None;
            }
        }
        Ok(self.status().await)
    }

    /// The pid of the live child, forgetting one that ended and a group that is empty.
    fn reap_and_pid(&self) -> Option<u32> {
        let mut inner = self.inner.lock().unwrap();
        let alive = match inner.child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => inner.child.as_ref().map(Child::id),
            Some(Ok(Some(status))) => {
                inner.last_exit = Some(status.code());
                inner.child = None;
                None
            }
            Some(Err(_)) => {
                inner.child = None;
                None
            }
            None => None,
        };
        if alive.is_none() && inner.group.is_some_and(|g| !group_alive(g)) {
            inner.group = None;
        }
        alive
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

/// Whether any process of the group is still alive. A group id is not reused while it has a
/// member, so this cannot be fooled by a new, unrelated process.
#[cfg(unix)]
fn group_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    unsafe { libc::killpg(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn group_alive(_pid: u32) -> bool {
    false
}

/// Whether a host answers the way `probe` says.
async fn host_answers(probe: &Probe) -> bool {
    match probe {
        Probe::Health(port) => probe_health(*port).await,
        Probe::Socket(path) => probe_socket(path).await,
    }
}

/// Whether something accepts a connection on the host's control socket. The connection is closed
/// at once without a word, so nothing is asked of the host.
#[cfg(unix)]
async fn probe_socket(path: &Path) -> bool {
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, tokio::net::UnixStream::connect(path)).await,
        Ok(Ok(_))
    )
}

#[cfg(not(unix))]
async fn probe_socket(_path: &Path) -> bool {
    false
}

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
        tried.extend(on_path(var));
    }
    tried
}

/// The first candidate that is HQ's own program. A configured path that is wrong is an error, not
/// a reason to quietly run some other `hq`.
pub fn locate_hq(configured: Option<&Path>, candidates: &[PathBuf]) -> Result<PathBuf, HostError> {
    if let Some(path) = configured {
        return if is_hq(path) {
            Ok(path.to_path_buf())
        } else {
            Err(HostError::ProgramNotFound(vec![path.to_path_buf()]))
        };
    }
    candidates
        .iter()
        .find(|p| is_hq(p))
        .cloned()
        .ok_or_else(|| HostError::ProgramNotFound(candidates.to_vec()))
}

/// Each directory on `path_var` with `hq` in it, for a search list.
pub fn on_path(path_var: &std::ffi::OsStr) -> impl Iterator<Item = PathBuf> + '_ {
    std::env::split_paths(path_var).map(|d| d.join(HQ_BINARY))
}

/// The PATH the person's login shell sets up, or None when it does not answer in time. An app
/// opened from the Dock gets only the system's short PATH, so an `hq` installed by a shell profile
/// is invisible to it otherwise.
pub fn login_shell_path(shell: &Path) -> Option<std::ffi::OsString> {
    let mut cmd = Command::new(shell);
    // Interactive too: most people extend PATH in the rc file, not the login profile.
    cmd.args([
        "-ilc",
        &format!("printf '%s\\n%s\\n%s\\n' {PATH_MARKER} \"$PATH\" {PATH_MARKER}"),
    ]);
    let out = output_within(cmd, LOGIN_SHELL_TIMEOUT)?;
    between_markers(&String::from_utf8_lossy(&out)).map(Into::into)
}

/// The text between the first two marker lines, so a profile that prints is ignored.
fn between_markers(out: &str) -> Option<String> {
    let mut lines = out.lines().skip_while(|l| *l != PATH_MARKER).skip(1);
    let path = lines.next()?;
    (lines.next()? == PATH_MARKER && !path.is_empty()).then(|| path.to_string())
}

/// Some systems ship an unrelated program called `hq`; only one whose `--version` says `hq <version>`
/// counts.
fn is_hq(path: &Path) -> bool {
    if !is_runnable(path) {
        return false;
    }
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    output_within(cmd, VERSION_TIMEOUT)
        .is_some_and(|out| String::from_utf8_lossy(&out).starts_with(VERSION_PREFIX))
}

/// What a short command printed, or None if it failed or ran past `limit`. On a timeout its whole
/// process group is killed, so a shell's children go too.
fn output_within(mut cmd: Command, limit: Duration) -> Option<Vec<u8>> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let child = cmd.spawn().ok()?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(out)) if out.status.success() => Some(out.stdout),
        Ok(_) => None,
        Err(_) => {
            // The waiting thread has not reaped the child, so the group id is still ours.
            signal_group(pid, Signal::Kill);
            None
        }
    }
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

/// A port nothing listens on, for tests in other modules.
#[cfg(test)]
pub(crate) fn tests_support_unused_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
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
            probe: Probe::Health(health_port),
            stop_grace: Duration::from_millis(600),
        }
    }

    fn spec_for(config: &HqConfig) -> HostSpec {
        HostSpec::for_config("hq".into(), "host.log".into(), config)
    }

    fn bridge(gate: bool, web_forward: bool) -> hq_core::config::BridgeServicesConfig {
        hq_core::config::BridgeServicesConfig {
            services: gate
                .then(|| ("gate".to_string(), "127.0.0.1:7747".to_string()))
                .into_iter()
                .collect(),
            forwards: web_forward
                .then(|| hq_core::config::BridgeForward {
                    peer: "abc".into(),
                    service: "web".into(),
                    listen: "127.0.0.1:47480".into(),
                    fail_fast_secs: None,
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }

    fn hub_config() -> HqConfig {
        HqConfig {
            bridge: bridge(true, true),
            ..HqConfig::default()
        }
    }

    #[test]
    fn a_laptop_that_serves_a_hub_runs_only_the_agent_host() {
        let hub = spec_for(&hub_config());
        assert_eq!(hub.args, ["host", "serve"]);
        let Probe::Socket(socket) = hub.probe else {
            panic!("an agent-only host has no /health");
        };
        assert_eq!(
            socket,
            hq_core::config::native_host_dir().join(HOST_SOCKET_FILE)
        );
    }

    #[test]
    fn a_machine_without_a_hub_runs_all_of_hq() {
        let mut config = HqConfig {
            ws_port: 6001,
            ..HqConfig::default()
        };
        let plain = spec_for(&config);
        assert_eq!(plain.args, ["start", "all"]);
        assert_eq!(plain.probe, Probe::Health(6001));
        // Offering the gate without reaching a phone's web app is not a hub.
        config.bridge = bridge(true, false);
        assert_eq!(spec_for(&config).args, ["start", "all"]);
    }

    #[test]
    fn the_socket_name_matches_the_host_s_own() {
        let dir = Path::new("/run/hq/host");
        assert_eq!(
            dir.join(HOST_SOCKET_FILE),
            hq_host::socket_path(dir),
            "hq-core and hq-host disagree on the control socket's name"
        );
    }

    #[tokio::test]
    async fn an_agent_host_is_up_while_its_socket_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.sock");
        let probe = Probe::Socket(path.clone());
        assert!(!host_answers(&probe).await);
        // A leftover file with nothing behind it is not a host. A closed listener is not used for
        // this: another test's fork can hold its descriptor open for a moment.
        std::fs::write(&path, b"").unwrap();
        assert!(!host_answers(&probe).await);
        std::fs::remove_file(&path).unwrap();
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert!(host_answers(&probe).await);
    }

    #[tokio::test]
    async fn a_socket_host_this_app_did_not_start_is_reported_and_never_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.sock");
        let _listener = tokio::net::UnixListener::bind(&path).unwrap();
        let mut spec = spec(dir.path(), "sleep 60", 0);
        spec.probe = Probe::Socket(path);
        let host = HostController::new(spec);
        assert_eq!(host.status().await, HostStatus::External);
        assert!(matches!(host.start().await, Err(HostError::AlreadyRunning)));
        assert!(matches!(host.stop().await, Err(HostError::NotOurs)));
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

    fn assert_send<T: Send>(_: &T) {}

    /// The shell spawns these on a multi-threaded runtime, so their futures must be `Send`.
    #[tokio::test]
    async fn the_futures_can_cross_threads() {
        let dir = tempfile::tempdir().unwrap();
        let host = HostController::new(spec(dir.path(), "exit 0", free_port()));
        assert_send(&host.status());
        assert_send(&host.start());
        assert_send(&host.stop());
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
    async fn stop_kills_group_members_that_ignore_the_polite_signal() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("stubborn.pid");
        // The leader dies on the polite signal; its child ignores it and would live on.
        let script = format!(
            "(trap '' TERM; exec sleep 60) & echo $! > {}; wait",
            pidfile.display()
        );
        let host = HostController::new(spec(dir.path(), &script, free_port()));
        host.start().await.unwrap();
        for _ in 0..50 {
            if std::fs::read_to_string(&pidfile).is_ok_and(|t| !t.trim().is_empty()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let stubborn: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(stubborn));
        assert_eq!(host.stop().await.unwrap(), HostStatus::Stopped);
        assert!(
            gone(stubborn).await,
            "a group member that ignored the signal survived"
        );
    }

    #[tokio::test]
    async fn stop_reaches_group_members_after_a_status_poll_reaped_the_leader() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("orphan.pid");
        // The leader exits at once and leaves a child that ignores the polite signal.
        let script = format!(
            "(trap '' TERM; exec sleep 60) & echo $! > {}",
            pidfile.display()
        );
        let host = HostController::new(spec(dir.path(), &script, free_port()));
        host.start().await.unwrap();
        for _ in 0..50 {
            if matches!(host.status().await, HostStatus::Exited { .. }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(host.status().await, HostStatus::Exited { code: Some(0) });
        let orphan: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(orphan), "the orphan ended on its own");
        assert_eq!(host.stop().await.unwrap(), HostStatus::Stopped);
        assert!(gone(orphan).await, "a group member outlived the stop");
    }

    #[tokio::test]
    async fn stop_kills_a_host_that_ignores_the_polite_signal() {
        let dir = tempfile::tempdir().unwrap();
        let host = HostController::new(spec(dir.path(), "trap '' TERM; sleep 60", free_port()));
        let HostStatus::Starting { pid } = host.start().await.unwrap() else {
            panic!("not starting");
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(host.stop().await.unwrap(), HostStatus::Stopped);
        assert!(
            gone(pid as i32).await,
            "the host ignored the signal and lived on"
        );
    }

    #[tokio::test]
    async fn nothing_starts_after_the_final_stop() {
        let dir = tempfile::tempdir().unwrap();
        let host = HostController::new(spec(dir.path(), "sleep 60", free_port()));
        host.stop_for_good().await.unwrap();
        assert!(matches!(host.start().await, Err(HostError::Closing)));
        assert_eq!(host.status().await, HostStatus::Stopped);
    }

    #[tokio::test]
    async fn two_starts_at_once_make_one_host() {
        let dir = tempfile::tempdir().unwrap();
        let marks = dir.path().join("starts");
        let script = format!("echo started >> {}; sleep 60", marks.display());
        let host = HostController::new(spec(dir.path(), &script, free_port()));
        let (first, second) = tokio::join!(host.start(), host.start());
        assert!(first.is_ok() && second.is_ok());
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(std::fs::read_to_string(&marks).unwrap().lines().count(), 1);
        host.stop().await.unwrap();
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

    fn script(path: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_path_buf()
    }

    #[test]
    fn only_a_program_that_says_it_is_hq_counts() {
        let root = tempfile::tempdir().unwrap();
        let other = script(&root.path().join("a/hq"), "echo 'hq: a window manager'");
        let failing = script(&root.path().join("b/hq"), "exit 1");
        let real = script(
            &root.path().join("c/hq"),
            "echo 'hq 0.9.1 (abcdef0 2026-11-02)'",
        );
        let candidates = vec![other.clone(), failing, real.clone()];
        assert_eq!(locate_hq(None, &candidates).unwrap(), real);
        assert!(locate_hq(Some(&other), &candidates).is_err());
    }

    #[test]
    fn a_program_that_hangs_is_passed_over_in_time() {
        let root = tempfile::tempdir().unwrap();
        let slow = script(&root.path().join("hq"), "sleep 30");
        let started = std::time::Instant::now();
        assert!(!is_hq(&slow));
        assert!(started.elapsed() < VERSION_TIMEOUT + Duration::from_secs(2));
    }

    #[test]
    fn the_login_shell_path_is_read_between_markers() {
        let root = tempfile::tempdir().unwrap();
        // A profile that prints before and after the PATH must not leak into it.
        let shell = script(
            &root.path().join("shell"),
            "echo 'Welcome back'\neval \"$2\"\necho 'bye'",
        );
        let path = login_shell_path(&shell).unwrap();
        assert!(!path.to_string_lossy().contains("Welcome"));
        assert_eq!(
            path,
            std::ffi::OsString::from(std::env::var("PATH").unwrap())
        );

        assert_eq!(between_markers("noise\n"), None);
        let m = PATH_MARKER;
        assert_eq!(between_markers(&format!("{m}\n\n{m}\n")), None);
        assert_eq!(
            between_markers(&format!("x\n{m}\n/a:/b\n{m}\ny\n")).as_deref(),
            Some("/a:/b")
        );
        let broken = script(&root.path().join("broken"), "exit 3");
        assert_eq!(login_shell_path(&broken), None);
    }

    #[test]
    fn hq_is_found_in_order_and_a_wrong_configured_path_is_not_replaced() {
        let make = |dir: &Path| script(&dir.join("hq"), "echo 'hq 0.9.1 (abcdef0)'");
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
