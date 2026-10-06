//! The control socket: a Unix socket in a private directory, JSON lines, one
//! thread per connection. A connection must say `hello` with the operator
//! token before anything else.

use crate::detect::AgentState;
use crate::error::HostError;
use crate::host::{Host, PaneInfo, PaneStatus, ReadSource, SpawnSpec};
use crate::proto::{ErrorBody, MAX_LINE_BYTES, PROTOCOL_VERSION, Request, Response};
use crate::token;
use serde::Deserialize;
use serde_json::{Value, json};
use std::cell::Cell;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

const SOCKET_FILE: &str = "host.sock";
const LOCK_FILE: &str = "host.lock";
const SOCKET_MODE: u32 = 0o600;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_QUIET_MS: u64 = 1_000;
const DEFAULT_STABLE_MS: u64 = 300;
const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The accept loop checks the stop flag this often.
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// After an unexpected accept error (out of descriptors, say) wait before retrying.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(200);
/// Most text one `agent.read` returns. JSON escaping can double newlines and
/// quotes, and the whole reply must stay under `MAX_LINE_BYTES`.
const MAX_READ_TEXT_BYTES: usize = 400 * 1024;

/// Identity of the executable file the host was started from. If the file on
/// disk differs later (a new build was installed), the running host is stale.
#[derive(Clone, PartialEq, Eq)]
struct ExeStamp {
    path: PathBuf,
    dev: u64,
    ino: u64,
    mtime: i64,
    size: u64,
}

impl ExeStamp {
    fn of(path: &Path) -> Option<(u64, u64, i64, u64)> {
        let m = std::fs::metadata(path).ok()?;
        Some((m.dev(), m.ino(), m.mtime(), m.size()))
    }

    fn current() -> Option<Self> {
        Self::at(std::env::current_exe().ok()?)
    }

    fn at(path: PathBuf) -> Option<Self> {
        let (dev, ino, mtime, size) = Self::of(&path)?;
        Some(Self {
            path,
            dev,
            ino,
            mtime,
            size,
        })
    }

    fn is_stale(&self) -> bool {
        Self::of(&self.path) != Some((self.dev, self.ino, self.mtime, self.size))
    }
}

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_FILE)
}

/// Bounds on what a client can tie up.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Connections served at once; one more is told `too_many_connections`.
    pub max_connections: usize,
    /// How long a connection may stay silent before `hello`.
    pub auth_timeout: Duration,
    /// How long a reply may take to write before the connection is dropped.
    pub write_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            auth_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(10),
        }
    }
}

pub struct Server {
    listener: UnixListener,
    path: PathBuf,
    host: Arc<Host>,
    token: String,
    stop: Arc<AtomicBool>,
    limits: Limits,
    exe: Option<ExeStamp>,
    /// Held for the life of the server: one host per directory.
    _lock: File,
}

/// Lets another thread end `Server::serve`.
#[derive(Clone)]
pub struct StopHandle {
    stop: Arc<AtomicBool>,
}

impl StopHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Server {
    pub fn bind(dir: &Path, host: Arc<Host>) -> std::io::Result<Self> {
        Self::bind_with_limits(dir, host, Limits::default())
    }

    /// Binds `<dir>/host.sock` (0600), creating the directory (0700) and the
    /// operator token if needed. Only one host can hold `<dir>/host.lock`, so a
    /// second one is refused, and a socket file found while holding the lock is
    /// stale and replaced.
    pub fn bind_with_limits(dir: &Path, host: Arc<Host>, limits: Limits) -> std::io::Result<Self> {
        let token = token::load_or_create(dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(SOCKET_MODE)
            .open(dir.join(LOCK_FILE))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("a host is already serving {}", dir.display()),
                ));
            }
            Err(TryLockError::Error(e)) => return Err(e),
        }
        let path = socket_path(dir);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            path,
            host,
            token,
            stop: Arc::new(AtomicBool::new(false)),
            limits,
            exe: ExeStamp::current(),
            _lock: lock,
        })
    }

    pub fn stop_handle(&self) -> StopHandle {
        StopHandle {
            stop: self.stop.clone(),
        }
    }

    /// Accepts connections until stopped.
    pub fn serve(self) {
        let open = Arc::new(AtomicUsize::new(0));
        while !self.stop.load(Ordering::SeqCst) {
            match self.listener.accept() {
                Ok((stream, _)) => self.admit(stream, &open),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_POLL);
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::ConnectionAborted
                    ) => {}
                Err(_) => std::thread::sleep(ACCEPT_ERROR_BACKOFF),
            }
        }
        // Stop the agents but leave them listed for the next start.
        self.host.shutdown();
        let _ = std::fs::remove_file(&self.path);
    }

    fn admit(&self, stream: UnixStream, open: &Arc<AtomicUsize>) {
        // Some systems hand out sockets that inherit the listener's non-blocking mode.
        if stream.set_nonblocking(false).is_err() {
            return;
        }
        if open.fetch_add(1, Ordering::SeqCst) >= self.limits.max_connections {
            open.fetch_sub(1, Ordering::SeqCst);
            let mut stream = stream;
            let _ = stream.set_write_timeout(Some(self.limits.write_timeout));
            let _ = send(
                &mut stream,
                &Response::err(
                    Value::Null,
                    "too_many_connections",
                    "the host is serving as many connections as it allows",
                ),
            );
            return;
        }
        let conn = Conn {
            host: self.host.clone(),
            token: self.token.clone(),
            stop: self.stop_handle(),
            limits: self.limits.clone(),
            exe: self.exe.clone(),
            stop_after_reply: Cell::new(false),
        };
        let open = open.clone();
        std::thread::spawn(move || {
            conn.run(stream);
            open.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

struct Conn {
    host: Arc<Host>,
    token: String,
    stop: StopHandle,
    limits: Limits,
    exe: Option<ExeStamp>,
    stop_after_reply: Cell<bool>,
}

impl Conn {
    fn run(self, stream: UnixStream) {
        let Ok(write_half) = stream.try_clone() else {
            return;
        };
        let mut writer = write_half;
        let _ = writer.set_write_timeout(Some(self.limits.write_timeout));
        let _ = stream.set_read_timeout(Some(self.limits.auth_timeout));
        let mut reader = BufReader::new(stream);
        let mut authed = false;
        loop {
            let mut line = Vec::new();
            let read = reader
                .by_ref()
                .take(MAX_LINE_BYTES as u64 + 1)
                .read_until(b'\n', &mut line);
            match read {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            if line.len() > MAX_LINE_BYTES {
                let _ = send(
                    &mut writer,
                    &Response::err(Value::Null, "line_too_long", "request line too long"),
                );
                return;
            }
            let response = match serde_json::from_slice::<Request>(&line) {
                Ok(req) => self.dispatch(req, &mut authed),
                Err(e) => Response::err(Value::Null, "bad_request", e.to_string()),
            };
            if send(&mut writer, &response).is_err() {
                return;
            }
            if self.stop_after_reply.get() {
                self.stop.stop();
                return;
            }
            if authed {
                // Idle authenticated connections are normal; only the wait for hello is bounded.
                let _ = reader.get_ref().set_read_timeout(None);
            }
        }
    }

    fn dispatch(&self, req: Request, authed: &mut bool) -> Response {
        let id = req.id.clone();
        if req.method == "hello" {
            return match self.hello(&req.params) {
                Ok(v) => {
                    *authed = true;
                    Response::ok(id, v)
                }
                Err(e) => Response::err(id, &e.code, e.message),
            };
        }
        if !*authed {
            return Response::err(
                id,
                "unauthenticated",
                "say hello with the operator token first",
            );
        }
        match self.call(&req.method, req.params) {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::err(id, &e.code, e.message),
        }
    }

    fn hello(&self, params: &Value) -> Result<Value, ErrorBody> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Hello {
            protocol_version: u32,
            token: String,
        }
        let hello: Hello = parse(params)?;
        if !token::matches(&self.token, &hello.token) {
            return Err(body("unauthorized", "wrong operator token"));
        }
        if hello.protocol_version != PROTOCOL_VERSION {
            return Err(body(
                "protocol_mismatch",
                format!(
                    "host speaks protocol {PROTOCOL_VERSION}, client sent {}",
                    hello.protocol_version
                ),
            ));
        }
        Ok(json!({ "protocol_version": PROTOCOL_VERSION, "host_version": HOST_VERSION }))
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, ErrorBody> {
        match method {
            "host.status" => {
                let agents = self.host.list();
                let count = |state| agents.iter().filter(|a| a.state == Some(state)).count();
                Ok(json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "host_version": HOST_VERSION,
                    "pid": std::process::id(),
                    "agents": agents.len(),
                    "agents_working": count(AgentState::Working),
                    "agents_blocked": count(AgentState::Blocked),
                    "binary_stale": self.exe.as_ref().is_some_and(ExeStamp::is_stale),
                }))
            }
            // The reply goes out first; the connection loop stops the host after it.
            "host.stop" => {
                self.stop_after_reply.set(true);
                Ok(json!({}))
            }
            m if m.starts_with("agent.") => self.agent_call(m, &params),
            other => Err(body("unknown_method", format!("no method {other:?}"))),
        }
    }

    fn agent_call(&self, method: &str, params: &Value) -> Result<Value, ErrorBody> {
        let host = &self.host;
        match method {
            "agent.spawn" => {
                let p: SpawnParams = parse(params)?;
                let mut spec = SpawnSpec::new(p.name, p.argv, p.cwd);
                spec.agent = p.agent;
                spec.resume_argv = p.resume_argv;
                spec.env = p.env.into_iter().collect();
                spec.rows = p.rows.unwrap_or(spec.rows);
                spec.cols = p.cols.unwrap_or(spec.cols);
                spec.scrollback_rows = p.scrollback_rows.unwrap_or(spec.scrollback_rows);
                Ok(info_json(&host.spawn(spec).map_err(host_err)?))
            }
            "agent.list" => {
                Ok(json!({ "agents": host.list().iter().map(info_json).collect::<Vec<_>>() }))
            }
            "agent.get" => {
                let p: Named = parse(params)?;
                Ok(info_json(&host.info(&p.name).map_err(host_err)?))
            }
            "agent.read" => {
                let p: ReadParams = parse(params)?;
                let text = host
                    .read(&p.name, p.source.into(), p.lines.unwrap_or(0))
                    .map_err(host_err)?;
                let (text, truncated) = tail_that_fits(text);
                Ok(json!({ "text": text, "truncated": truncated }))
            }
            "agent.send_text" => {
                let p: TextParams = parse(params)?;
                done(host.send_text(&p.name, &p.text))
            }
            "agent.paste" => {
                let p: TextParams = parse(params)?;
                done(host.paste(&p.name, &p.text))
            }
            "agent.prompt" => {
                let p: TextParams = parse(params)?;
                done(host.prompt(&p.name, &p.text))
            }
            "agent.send_keys" => {
                let p: KeysParams = parse(params)?;
                done(host.send_keys(&p.name, &p.keys))
            }
            "agent.resize" => {
                let p: ResizeParams = parse(params)?;
                done(host.resize(&p.name, p.rows, p.cols))
            }
            "agent.wait" => self.wait(parse(params)?),
            "agent.kill" => {
                let p: Named = parse(params)?;
                done(host.kill(&p.name))
            }
            "agent.remove" => {
                let p: Named = parse(params)?;
                done(host.remove(&p.name))
            }
            other => Err(body("unknown_method", format!("no method {other:?}"))),
        }
    }

    fn wait(&self, p: WaitParams) -> Result<Value, ErrorBody> {
        let timeout = Duration::from_millis(p.timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS));
        match p.until.as_str() {
            "exit" => {
                let code = self.host.wait_exit(&p.name, timeout).map_err(host_err)?;
                Ok(json!({ "exit_code": code }))
            }
            "quiet" => {
                let quiet = Duration::from_millis(p.quiet_ms.unwrap_or(DEFAULT_QUIET_MS));
                self.host
                    .wait_quiet(&p.name, quiet, timeout)
                    .map_err(host_err)?;
                Ok(info_json(&self.host.info(&p.name).map_err(host_err)?))
            }
            "state" => {
                if p.states.is_empty() {
                    return Err(body(
                        "invalid_params",
                        "until state needs a non-empty states list",
                    ));
                }
                let stable = Duration::from_millis(p.stable_ms.unwrap_or(DEFAULT_STABLE_MS));
                let info = self
                    .host
                    .wait_state(&p.name, &p.states, stable, timeout)
                    .map_err(host_err)?;
                Ok(info_json(&info))
            }
            other => Err(body(
                "invalid_params",
                format!("until must be exit, quiet or state, got {other:?}"),
            )),
        }
    }
}

/// The last part of `text` that fits in one reply, cut at a line start.
fn tail_that_fits(text: String) -> (String, bool) {
    if text.len() <= MAX_READ_TEXT_BYTES {
        return (text, false);
    }
    let mut start = text.len() - MAX_READ_TEXT_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    if let Some(newline) = text[start..].find('\n') {
        start += newline + 1;
    }
    (text[start..].to_string(), true)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnParams {
    name: String,
    argv: Vec<String>,
    cwd: PathBuf,
    agent: Option<String>,
    resume_argv: Option<Vec<String>>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    rows: Option<u16>,
    cols: Option<u16>,
    scrollback_rows: Option<usize>,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum SourceParam {
    Visible,
    Recent,
    #[default]
    RecentUnwrapped,
}

impl From<SourceParam> for ReadSource {
    fn from(p: SourceParam) -> Self {
        match p {
            SourceParam::Visible => ReadSource::Visible,
            SourceParam::Recent => ReadSource::Recent,
            SourceParam::RecentUnwrapped => ReadSource::RecentUnwrapped,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadParams {
    name: String,
    #[serde(default)]
    source: SourceParam,
    lines: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextParams {
    name: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysParams {
    name: String,
    keys: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeParams {
    name: String,
    rows: u16,
    cols: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitParams {
    name: String,
    until: String,
    timeout_ms: Option<u64>,
    quiet_ms: Option<u64>,
    /// For `until: "state"`: the states to wait for.
    #[serde(default)]
    states: Vec<AgentState>,
    /// For `until: "state"`: how long the state must hold.
    stable_ms: Option<u64>,
}

fn body(code: &str, message: impl Into<String>) -> ErrorBody {
    ErrorBody {
        code: code.into(),
        message: message.into(),
    }
}

fn host_err(e: HostError) -> ErrorBody {
    body(e.code(), e.to_string())
}

fn done(result: Result<(), HostError>) -> Result<Value, ErrorBody> {
    result.map(|()| json!({})).map_err(host_err)
}

fn parse<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, ErrorBody> {
    serde_json::from_value(params.clone()).map_err(|e| body("invalid_params", e.to_string()))
}

fn send(w: &mut UnixStream, resp: &Response) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(resp).map_err(std::io::Error::other)?;
    line.push(b'\n');
    w.write_all(&line)
}

fn info_json(i: &PaneInfo) -> Value {
    let (status, exit_code) = match i.status {
        PaneStatus::Running => ("running", None),
        PaneStatus::Exited { code } => ("exited", Some(code)),
    };
    json!({
        "name": i.name,
        "argv": i.argv,
        "agent": i.agent,
        "resumable": i.resumable,
        "title": i.title,
        "state": i.state.map(AgentState::as_str),
        "rule": i.rule,
        "cwd": i.cwd.to_string_lossy(),
        "pid": i.pid,
        "status": status,
        "exit_code": exit_code,
        "rows": i.rows,
        "cols": i.cols,
        "bytes_seen": i.bytes_seen,
        "quiet_ms": i.quiet_for.as_millis() as u64,
        "age_ms": i.age.as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_or_removed_executable_makes_the_host_stale() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("hq");
        std::fs::write(&exe, "old build").unwrap();
        let stamp = ExeStamp::at(exe.clone()).unwrap();
        assert!(!stamp.is_stale());

        // An install replaces the file by renaming a new one over it.
        let new = dir.path().join("hq.new");
        std::fs::write(&new, "new build, different size").unwrap();
        std::fs::rename(&new, &exe).unwrap();
        assert!(stamp.is_stale());

        let fresh = ExeStamp::at(exe.clone()).unwrap();
        assert!(!fresh.is_stale());
        std::fs::remove_file(&exe).unwrap();
        assert!(fresh.is_stale());
    }
}
