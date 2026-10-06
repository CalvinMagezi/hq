//! The control socket: a Unix socket in a private directory, JSON lines, one
//! thread per connection. A connection must say `hello` with the operator
//! token before anything else.

use crate::error::HostError;
use crate::host::{Host, PaneInfo, PaneStatus, ReadSource, SpawnSpec};
use crate::proto::{ErrorBody, MAX_LINE_BYTES, PROTOCOL_VERSION, Request, Response};
use crate::token;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const SOCKET_FILE: &str = "host.sock";
const SOCKET_MODE: u32 = 0o600;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_QUIET_MS: u64 = 1_000;
const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_FILE)
}

pub struct Server {
    listener: UnixListener,
    path: PathBuf,
    host: Arc<Host>,
    token: String,
    stop: Arc<AtomicBool>,
}

/// Lets another thread end `Server::serve`.
#[derive(Clone)]
pub struct StopHandle {
    stop: Arc<AtomicBool>,
    path: PathBuf,
}

impl StopHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop.
        let _ = UnixStream::connect(&self.path);
    }
}

impl Server {
    /// Binds `<dir>/host.sock` (0600), creating the directory (0700) and the
    /// operator token if needed. A socket left by a dead host is replaced; one
    /// that still answers is an error.
    pub fn bind(dir: &Path, host: Arc<Host>) -> std::io::Result<Self> {
        let token = token::load_or_create(dir)?;
        let path = socket_path(dir);
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("a host is already serving {}", path.display()),
                ));
            }
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        Ok(Self {
            listener,
            path,
            host,
            token,
            stop: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn stop_handle(&self) -> StopHandle {
        StopHandle {
            stop: self.stop.clone(),
            path: self.path.clone(),
        }
    }

    /// Accepts connections until stopped.
    pub fn serve(self) {
        for stream in self.listener.incoming() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let ctx = Conn {
                host: self.host.clone(),
                token: self.token.clone(),
                stop: self.stop_handle(),
            };
            std::thread::spawn(move || ctx.run(stream));
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

struct Conn {
    host: Arc<Host>,
    token: String,
    stop: StopHandle,
}

impl Conn {
    fn run(self, stream: UnixStream) {
        let Ok(write_half) = stream.try_clone() else {
            return;
        };
        let mut writer = write_half;
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
        let host = &self.host;
        match method {
            "host.status" => Ok(json!({
                "protocol_version": PROTOCOL_VERSION,
                "host_version": HOST_VERSION,
                "pid": std::process::id(),
                "agents": host.list().len(),
            })),
            "host.stop" => {
                self.stop.stop();
                Ok(json!({}))
            }
            "agent.spawn" => {
                let p: SpawnParams = parse(&params)?;
                let mut spec = SpawnSpec::new(p.name, p.argv, p.cwd);
                spec.env = p.env.into_iter().collect();
                if let Some(rows) = p.rows {
                    spec.rows = rows;
                }
                if let Some(cols) = p.cols {
                    spec.cols = cols;
                }
                if let Some(n) = p.scrollback_rows {
                    spec.scrollback_rows = n;
                }
                Ok(info_json(&host.spawn(spec).map_err(host_err)?))
            }
            "agent.list" => {
                Ok(json!({ "agents": host.list().iter().map(info_json).collect::<Vec<_>>() }))
            }
            "agent.get" => {
                let p: Named = parse(&params)?;
                Ok(info_json(&host.info(&p.name).map_err(host_err)?))
            }
            "agent.read" => {
                let p: ReadParams = parse(&params)?;
                let text = host
                    .read(&p.name, p.source, p.lines.unwrap_or(0))
                    .map_err(host_err)?;
                Ok(json!({ "text": text }))
            }
            "agent.send_text" => {
                let p: TextParams = parse(&params)?;
                host.send_text(&p.name, &p.text).map_err(host_err)?;
                Ok(json!({}))
            }
            "agent.paste" => {
                let p: TextParams = parse(&params)?;
                host.paste(&p.name, &p.text).map_err(host_err)?;
                Ok(json!({}))
            }
            "agent.prompt" => {
                let p: TextParams = parse(&params)?;
                host.prompt(&p.name, &p.text).map_err(host_err)?;
                Ok(json!({}))
            }
            "agent.send_keys" => {
                let p: KeysParams = parse(&params)?;
                host.send_keys(&p.name, &p.keys).map_err(host_err)?;
                Ok(json!({}))
            }
            "agent.resize" => {
                let p: ResizeParams = parse(&params)?;
                host.resize(&p.name, p.rows, p.cols).map_err(host_err)?;
                Ok(json!({}))
            }
            "agent.wait" => {
                let p: WaitParams = parse(&params)?;
                let timeout =
                    Duration::from_millis(p.timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS));
                match p.until.as_str() {
                    "exit" => {
                        let code = host.wait_exit(&p.name, timeout).map_err(host_err)?;
                        Ok(json!({ "exit_code": code }))
                    }
                    "quiet" => {
                        let quiet = Duration::from_millis(p.quiet_ms.unwrap_or(DEFAULT_QUIET_MS));
                        host.wait_quiet(&p.name, quiet, timeout).map_err(host_err)?;
                        Ok(info_json(&host.info(&p.name).map_err(host_err)?))
                    }
                    other => Err(body(
                        "invalid_params",
                        format!("until must be exit or quiet, got {other:?}"),
                    )),
                }
            }
            "agent.kill" => {
                let p: Named = parse(&params)?;
                host.kill(&p.name).map_err(host_err)?;
                Ok(json!({}))
            }
            "agent.remove" => {
                let p: Named = parse(&params)?;
                host.remove(&p.name).map_err(host_err)?;
                Ok(json!({}))
            }
            other => Err(body("unknown_method", format!("no method {other:?}"))),
        }
    }
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct SpawnParams {
    name: String,
    argv: Vec<String>,
    cwd: PathBuf,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    rows: Option<u16>,
    cols: Option<u16>,
    scrollback_rows: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SourceParam {
    Visible,
    Recent,
    RecentUnwrapped,
}

#[derive(Deserialize)]
struct ReadRaw {
    name: String,
    source: Option<SourceParam>,
    lines: Option<usize>,
}

struct ReadParams {
    name: String,
    source: ReadSource,
    lines: Option<usize>,
}

impl<'de> Deserialize<'de> for ReadParams {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = ReadRaw::deserialize(d)?;
        let source = match raw.source.unwrap_or(SourceParam::RecentUnwrapped) {
            SourceParam::Visible => ReadSource::Visible,
            SourceParam::Recent => ReadSource::Recent,
            SourceParam::RecentUnwrapped => ReadSource::RecentUnwrapped,
        };
        Ok(Self {
            name: raw.name,
            source,
            lines: raw.lines,
        })
    }
}

#[derive(Deserialize)]
struct TextParams {
    name: String,
    text: String,
}

#[derive(Deserialize)]
struct KeysParams {
    name: String,
    keys: Vec<String>,
}

#[derive(Deserialize)]
struct ResizeParams {
    name: String,
    rows: u16,
    cols: u16,
}

#[derive(Deserialize)]
struct WaitParams {
    name: String,
    until: String,
    timeout_ms: Option<u64>,
    quiet_ms: Option<u64>,
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
        "cwd": i.cwd,
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
