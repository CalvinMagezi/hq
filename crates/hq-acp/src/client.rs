//! One ACP agent process driven over its stdin and stdout. Synchronous on purpose: the host that
//! will own these connections runs on plain threads, and a reader thread plus a channel is all a
//! JSON-RPC pipe needs.

use crate::dialog::Choice;
use crate::tracker::{AnswerError, Tracker};
use crate::wire::{Incoming, RpcError, parse_line, request};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: u32 = 1;
const STDERR_TAIL_LINES: usize = 50;
const STOP_POLL: Duration = Duration::from_millis(25);
const SIGNAL_EXIT_BASE: i32 = 128;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("the agent answered with an error: {} ({})", .0.message, .0.code)]
    Rpc(RpcError),
    #[error("no answer within the time given")]
    Timeout,
    #[error("the agent process ended")]
    Exited,
    #[error("no session yet: call new_session first")]
    NoSession,
    #[error("unexpected reply: {0}")]
    Protocol(&'static str),
    #[error("the agent does not offer the model '{wanted}' (it offers: {})", .available.join(", "))]
    UnknownModel {
        wanted: String,
        available: Vec<String>,
    },
    #[error("the agent offers no way to choose a model")]
    NoModelChoice,
}

/// How an agent lets a client choose its model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelChoice {
    /// `session/set_model`, with the ids from `models.availableModels`.
    SetModel,
    /// `session/set_config_option` on the config option of category `model`.
    ConfigOption { config_id: String },
}

/// The models a new session offers and the one it started on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelMenu {
    pub current: Option<String>,
    pub available: Vec<String>,
    pub choice: ModelChoice,
}

/// The values of a config option's choices, whether they are listed flat or in named groups.
fn option_values(list: Option<&Value>) -> Vec<String> {
    let items = list.and_then(Value::as_array).into_iter().flatten();
    items
        .flat_map(|item| match item.get("options") {
            Some(group) => option_values(Some(group)),
            None => item
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .into_iter()
                .collect(),
        })
        .collect()
}

/// Reads the model menu out of a `session/new` result, in either of the two shapes agents use.
pub fn parse_model_menu(result: &Value) -> Option<ModelMenu> {
    let strings = |list: Option<&Value>, key: &str| -> Vec<String> {
        let items = list.and_then(Value::as_array).into_iter().flatten();
        items
            .filter_map(|m| m.get(key).and_then(Value::as_str).map(str::to_owned))
            .collect()
    };
    if let Some(models) = result.get("models") {
        let available = strings(models.get("availableModels"), "modelId");
        if !available.is_empty() {
            let current = models.get("currentModelId").and_then(Value::as_str);
            return Some(ModelMenu {
                current: current.map(str::to_owned),
                available,
                choice: ModelChoice::SetModel,
            });
        }
    }
    let options = result.get("configOptions").and_then(Value::as_array)?;
    let model = options.iter().find(|o| {
        o.get("category").and_then(Value::as_str) == Some("model")
            || o.get("id").and_then(Value::as_str) == Some("model")
    })?;
    let available = option_values(model.get("options"));
    (!available.is_empty()).then(|| ModelMenu {
        current: model
            .get("currentValue")
            .and_then(Value::as_str)
            .map(str::to_owned),
        available,
        choice: ModelChoice::ConfigOption {
            config_id: model
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("model")
                .to_owned(),
        },
    })
}

/// What an agent said it can do at `initialize`. Both spellings of each key are accepted because
/// some agents send camelCase and snake_case side by side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub load_session: bool,
    pub resume: bool,
    pub fork: bool,
    pub close: bool,
    pub list: bool,
    pub mcp_http: bool,
    pub mcp_sse: bool,
    pub embedded_context: bool,
    pub agent_name: String,
    pub agent_version: String,
}

pub fn parse_capabilities(result: &Value) -> Capabilities {
    let caps = result.get("agentCapabilities").unwrap_or(&Value::Null);
    let flag = |paths: &[&str]| paths.iter().any(|p| truthy(caps.pointer(p)));
    let text = |p: &str| {
        let v = result.pointer(p).and_then(Value::as_str);
        v.unwrap_or("").to_owned()
    };
    Capabilities {
        load_session: flag(&["/loadSession", "/load_session"]),
        resume: flag(&[
            "/sessionCapabilities/resume",
            "/session_capabilities/resume",
        ]),
        fork: flag(&["/sessionCapabilities/fork", "/session_capabilities/fork"]),
        close: flag(&["/sessionCapabilities/close", "/session_capabilities/close"]),
        list: flag(&["/sessionCapabilities/list", "/session_capabilities/list"]),
        mcp_http: flag(&["/mcpCapabilities/http"]),
        mcp_sse: flag(&["/mcpCapabilities/sse"]),
        embedded_context: flag(&["/promptCapabilities/embeddedContext"]),
        agent_name: text("/agentInfo/name"),
        agent_version: text("/agentInfo/version"),
    }
}

/// `{}` counts as supported (an empty capability object), `false` and absent do not.
fn truthy(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null) | Some(Value::Bool(false)))
}

pub struct Connection {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Incoming>,
    next_id: u64,
    session_id: Option<String>,
    models: Option<ModelMenu>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    pub tracker: Tracker,
}

impl Connection {
    /// Starts `cmd` with piped stdio. The caller sets program, arguments, working folder and the
    /// environment, which is also where a process sandbox wraps it.
    pub fn spawn(mut cmd: Command) -> Result<Self, ClientError> {
        // The agent leads a process group of its own, so everything it starts can be stopped with it.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or(ClientError::Protocol("no stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(ClientError::Protocol("no stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(ClientError::Protocol("no stderr"))?;
        let (tx, rx) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                // A line that is not JSON-RPC is noise (a banner, a log): skipped, never fatal.
                if let Ok(msg) = parse_line(&line)
                    && tx.send(msg).is_err()
                {
                    break;
                }
            }
        });
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let sink = Arc::clone(&tail);
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut q = sink.lock().unwrap_or_else(|e| e.into_inner());
                if q.len() == STDERR_TAIL_LINES {
                    q.pop_front();
                }
                q.push_back(line);
            }
        });
        Ok(Self {
            child,
            stdin,
            rx,
            next_id: 0,
            session_id: None,
            models: None,
            stderr: tail,
            tracker: Tracker::default(),
        })
    }

    /// The process id, which the host uses to stop the agent's whole process group.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The exit code once the process has ended (128 plus the signal after a signal), `None`
    /// while it runs.
    pub fn exit_code(&mut self) -> Option<i32> {
        let status = self.child.try_wait().ok().flatten()?;
        Some(exit_code_of(status))
    }

    /// Stops the agent and everything it started: the group is asked to end, given `grace` for the
    /// agent itself to do so, then killed. Returns the exit code.
    #[cfg(unix)]
    pub fn stop_group(&mut self, grace: Duration) -> i32 {
        self.signal_group(nix::sys::signal::Signal::SIGTERM);
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline && self.exit_code().is_none() {
            thread::sleep(STOP_POLL);
        }
        self.kill_group()
    }

    /// Kills whatever is left of the agent's process group and reaps the agent.
    #[cfg(unix)]
    pub fn kill_group(&mut self) -> i32 {
        self.signal_group(nix::sys::signal::Signal::SIGKILL);
        let _ = self.child.kill();
        self.child.wait().map_or(1, exit_code_of)
    }

    #[cfg(unix)]
    fn signal_group(&self, signal: nix::sys::signal::Signal) {
        // The group id is the agent's pid because `spawn` made it the leader.
        let group = nix::unistd::Pid::from_raw(self.child.id() as i32);
        let _ = nix::sys::signal::killpg(group, signal);
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// The last lines the agent wrote to stderr, for a failure report.
    pub fn stderr_tail(&self) -> Vec<String> {
        let q = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        q.iter().cloned().collect()
    }

    pub fn initialize(
        &mut self,
        client: &str,
        timeout: Duration,
    ) -> Result<Capabilities, ClientError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "clientCapabilities": {},
            "clientInfo": {"name": client, "version": env!("CARGO_PKG_VERSION")},
        });
        let result = self.call("initialize", params, timeout)?;
        Ok(parse_capabilities(&result))
    }

    pub fn new_session(
        &mut self,
        cwd: &str,
        mcp_servers: Value,
        timeout: Duration,
    ) -> Result<String, ClientError> {
        let result = self.call(
            "session/new",
            json!({"cwd": cwd, "mcpServers": mcp_servers}),
            timeout,
        )?;
        let id = result.get("sessionId").and_then(Value::as_str);
        let id = id.ok_or(ClientError::Protocol("session/new returned no sessionId"))?;
        self.session_id = Some(id.to_owned());
        self.models = parse_model_menu(&result);
        self.tracker.set_cwd(cwd);
        Ok(id.to_owned())
    }

    /// The models the session offered when it was opened; None for a loaded session or an agent
    /// that offers no choice.
    pub fn models(&self) -> Option<&ModelMenu> {
        self.models.as_ref()
    }

    /// Switches the session to `wanted`, one of the ids it offered. Does nothing when it is the
    /// model the session is on already. An id the agent did not offer is refused with the list.
    pub fn select_model(&mut self, wanted: &str, timeout: Duration) -> Result<(), ClientError> {
        let session = self.session_id.clone().ok_or(ClientError::NoSession)?;
        let menu = self.models.clone().ok_or(ClientError::NoModelChoice)?;
        if menu.current.as_deref() == Some(wanted) {
            return Ok(());
        }
        if !menu.available.iter().any(|m| m == wanted) {
            return Err(ClientError::UnknownModel {
                wanted: wanted.to_owned(),
                available: menu.available,
            });
        }
        match &menu.choice {
            ModelChoice::SetModel => {
                let params = json!({"sessionId": session, "modelId": wanted});
                self.call("session/set_model", params, timeout)?;
            }
            ModelChoice::ConfigOption { config_id } => {
                let params = json!({"sessionId": session, "configId": config_id, "value": wanted});
                self.call("session/set_config_option", params, timeout)?;
            }
        }
        if let Some(m) = self.models.as_mut() {
            m.current = Some(wanted.to_owned());
        }
        Ok(())
    }

    /// Opens a stored session again. The agent replays its history before it answers; that replay
    /// is ignored, so only new work shows in the transcript and the edited files.
    pub fn load_session(
        &mut self,
        session_id: &str,
        cwd: &str,
        mcp_servers: Value,
        timeout: Duration,
    ) -> Result<(), ClientError> {
        let params = json!({"sessionId": session_id, "cwd": cwd, "mcpServers": mcp_servers});
        self.tracker.set_replaying(true);
        let loaded = self.call("session/load", params, timeout);
        self.tracker.set_replaying(false);
        loaded?;
        self.session_id = Some(session_id.to_owned());
        self.tracker.set_cwd(cwd);
        self.tracker.record_restored(session_id);
        Ok(())
    }

    /// Sends a prompt and returns at once; follow the turn with `pump` and the tracker.
    pub fn prompt(&mut self, text: &str) -> Result<u64, ClientError> {
        let session = self.session_id.clone().ok_or(ClientError::NoSession)?;
        let params = json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]});
        let id = self.take_id();
        self.tracker.record_prompt(text);
        self.tracker.start_prompt(id);
        self.write(&request(id, "session/prompt", params))?;
        Ok(id)
    }

    pub fn cancel(&mut self) -> Result<(), ClientError> {
        let session = self.session_id.clone().ok_or(ClientError::NoSession)?;
        for line in self.tracker.cancel(&session) {
            self.write(&line)?;
        }
        Ok(())
    }

    pub fn answer(&mut self, seq: u64, choice: &Choice) -> Result<(), ClientError> {
        let line = self.tracker.answer(seq, choice).map_err(answer_error)?;
        self.write(&line)
    }

    /// Waits up to `timeout` for one message from the agent and feeds it to the tracker.
    /// `Ok(false)` means nothing arrived; `Err(Exited)` means the process is gone.
    pub fn pump(&mut self, timeout: Duration) -> Result<bool, ClientError> {
        match self.rx.recv_timeout(timeout) {
            Ok(msg) => {
                self.handle(&msg)?;
                Ok(true)
            }
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(RecvTimeoutError::Disconnected) => Err(ClientError::Exited),
        }
    }

    fn call(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        let id = self.take_id();
        self.write(&request(id, method, params))?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = match self.rx.recv_timeout(left) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => return Err(ClientError::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(ClientError::Exited),
            };
            match msg {
                Incoming::Response { id: got, result } if got == id => {
                    return result.map_err(ClientError::Rpc);
                }
                other => self.handle(&other)?,
            }
        }
    }

    fn handle(&mut self, msg: &Incoming) -> Result<(), ClientError> {
        for line in self.tracker.feed(msg) {
            self.write(&line)?;
        }
        Ok(())
    }

    fn take_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn write(&mut self, line: &str) -> Result<(), ClientError> {
        let sent = writeln!(self.stdin, "{line}").and_then(|()| self.stdin.flush());
        sent.map_err(|_| ClientError::Exited)
    }
}

fn answer_error(e: AnswerError) -> ClientError {
    match e {
        AnswerError::NoDialog => ClientError::Protocol("no dialog is open"),
        AnswerError::Stale => ClientError::Protocol("the dialog changed"),
        AnswerError::UnknownOption(_) => ClientError::Protocol("no such option"),
    }
}

fn exit_code_of(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return SIGNAL_EXIT_BASE + signal;
    }
    status.code().unwrap_or(1)
}

impl Drop for Connection {
    fn drop(&mut self) {
        #[cfg(unix)]
        self.kill_group();
        #[cfg(not(unix))]
        {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_model_menu_shapes_are_read() {
        let set = json!({"models": {"currentModelId": "a", "availableModels": [{"modelId": "a"}, {"modelId": "b"}]}});
        let menu = parse_model_menu(&set).unwrap();
        assert_eq!(
            (menu.current.as_deref(), menu.available.len()),
            (Some("a"), 2)
        );
        assert_eq!(menu.choice, ModelChoice::SetModel);
        let config = json!({"configOptions": [
            {"id": "mode", "category": "mode", "options": [{"value": "x"}]},
            {"id": "model", "category": "model", "currentValue": "p/q", "options": [{"value": "p/q"}, {"value": "p/r"}]}]});
        let menu = parse_model_menu(&config).unwrap();
        assert_eq!(menu.available, vec!["p/q", "p/r"]);
        assert_eq!(
            menu.choice,
            ModelChoice::ConfigOption {
                config_id: "model".into()
            }
        );
        let grouped = json!({"configOptions": [{"id": "model", "category": "model", "currentValue": "a",
            "options": [{"group": "g1", "name": "G1", "options": [{"value": "a"}]}, {"value": "b"}]}]});
        assert_eq!(
            parse_model_menu(&grouped).unwrap().available,
            vec!["a", "b"]
        );
        assert_eq!(parse_model_menu(&json!({"sessionId": "s"})), None);
    }

    #[test]
    fn capabilities_accept_both_spellings_and_empty_objects() {
        let r = json!({"agentCapabilities": {"load_session": true,
            "sessionCapabilities": {"resume": {}, "close": {}}, "mcpCapabilities": {"http": true, "sse": false},
            "promptCapabilities": {"embeddedContext": true}}, "agentInfo": {"name": "x", "version": "1"}});
        let c = parse_capabilities(&r);
        assert!(c.load_session && c.resume && c.close && c.mcp_http && c.embedded_context);
        assert!(!c.fork && !c.list && !c.mcp_sse);
        assert_eq!(c.agent_name, "x");
    }
}
