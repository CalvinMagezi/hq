//! A coding agent driven over the Agent Client Protocol instead of a terminal. Everything the host
//! reads off a screen for a pty agent (state, the dialog, whether a turn is done, the text it wrote)
//! comes here from the protocol, so there are no rule files and no keystrokes to lose.
//!
//! One thread owns the connection. Callers hand it commands and read a shared view, which the
//! thread keeps current and announces through the same event log the pty panes use.

use crate::detect::{AgentState, AnswerMethod, Dialog, DialogKind, DialogOption};
use crate::error::HostError;
use crate::events::{EventKind, EventLog};
use crate::host::{AcpFacts, PaneInfo};
use crate::pane::{ExitHook, PaneStatus};
use crate::sandbox::{Mode, SandboxSpec};
use hq_acp::{
    AgentState as WireState, Choice, Connection, OptionKind, StopReason, ToolKind, TurnEnd,
};
use serde_json::json;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(50);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const COMMAND_ACK_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_GRACE: Duration = Duration::from_secs(2);
/// Prompts held back while a turn runs; more are refused so a runaway caller cannot grow memory.
const MAX_QUEUED_PROMPTS: usize = 16;
const RULE: &str = "acp";
const CLIENT_NAME: &str = "agent-hq-host";
const MCP_SERVER_NAME: &str = "agent-hq";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) struct AcpLaunch {
    pub name: String,
    pub agent: Option<String>,
    /// What the caller asked to run, shown in listings.
    pub argv: Vec<String>,
    /// What actually runs: `argv`, or the sandbox wrapper around it.
    pub exec: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub sandbox: Option<SandboxSpec>,
    pub idle_ttl: Option<Duration>,
    pub mcp: Option<crate::host::McpEndpoint>,
    pub events: Arc<EventLog>,
    pub on_exit: ExitHook,
}

/// A dialog the agent waits at, with what it takes to answer it.
struct OpenDialog {
    /// The event number the host announced it with; what callers name when they answer.
    host_seq: u64,
    wire_seq: u64,
    dialog: Arc<Dialog>,
    option_ids: Vec<String>,
    decline: Choice,
}

struct View {
    state: AgentState,
    state_seq: u64,
    done: bool,
    pending: Option<OpenDialog>,
    lines: VecDeque<String>,
    dropped: u64,
    partial: String,
    status: PaneStatus,
    last_activity: Instant,
    bytes: u64,
    facts: AcpFacts,
}

enum Cmd {
    Prompt(String, Sender<Result<(), String>>),
    Answer(u64, Choice, Sender<Result<(), String>>),
    Cancel(Sender<Result<(), String>>),
    Stop,
}

pub(crate) struct AcpPane {
    pub agent: Option<String>,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub sandbox: Option<SandboxSpec>,
    pub idle_ttl: Option<Duration>,
    pub started: Instant,
    pid: u32,
    session_id: String,
    view: Arc<Mutex<View>>,
    cmds: Sender<Cmd>,
}

impl AcpPane {
    /// Starts the agent, completes the handshake and opens a session. Fails with the agent's own
    /// words when it needs a login or cannot start.
    pub(crate) fn launch(l: AcpLaunch) -> Result<Self, HostError> {
        let program = l.exec.first().cloned().unwrap_or_default();
        let spawn_error = |detail: String| HostError::Spawn {
            command: program.clone(),
            detail,
        };
        let mut cmd = std::process::Command::new(&program);
        cmd.args(&l.exec[1..])
            .env_clear()
            .envs(l.env.iter().cloned())
            .current_dir(&l.cwd);
        let mut conn = Connection::spawn(cmd).map_err(|e| spawn_error(e.to_string()))?;
        let started = conn
            .initialize(CLIENT_NAME, HANDSHAKE_TIMEOUT)
            .and_then(|caps| {
                let servers = mcp_servers(l.mcp.as_ref(), &caps);
                conn.new_session(&l.cwd.to_string_lossy(), servers, HANDSHAKE_TIMEOUT)
            });
        let session_id = started.map_err(|e| spawn_error(with_stderr(&e.to_string(), &conn)))?;
        let pid = conn.pid();
        let now = Instant::now();
        let view = Arc::new(Mutex::new(View {
            state: AgentState::Idle,
            state_seq: 0,
            done: false,
            pending: None,
            lines: VecDeque::new(),
            dropped: 0,
            partial: String::new(),
            status: PaneStatus::Running,
            last_activity: now,
            bytes: 0,
            facts: AcpFacts::default(),
        }));
        let (tx, rx) = channel();
        let driver = Driver {
            conn,
            cmds: rx,
            view: view.clone(),
            name: l.name,
            events: l.events,
            on_exit: l.on_exit,
        };
        std::thread::spawn(move || driver.run());
        Ok(Self {
            agent: l.agent,
            argv: l.argv,
            cwd: l.cwd,
            sandbox: l.sandbox,
            idle_ttl: l.idle_ttl,
            started: now,
            pid,
            session_id,
            view,
            cmds: tx,
        })
    }

    pub(crate) fn status(&self) -> PaneStatus {
        lock(&self.view).status
    }

    pub(crate) fn quiet_for(&self) -> Duration {
        lock(&self.view).last_activity.elapsed()
    }

    pub(crate) fn state(&self) -> AgentState {
        lock(&self.view).state
    }

    pub(crate) fn info(&self, name: &str) -> PaneInfo {
        let v = lock(&self.view);
        let process_sandbox = self.sandbox.as_ref().filter(|s| s.mode == Mode::Process);
        PaneInfo {
            name: name.to_string(),
            argv: self.argv.clone(),
            agent: self.agent.clone(),
            resumable: false,
            agent_session_id: Some(self.session_id.clone()),
            state_seq: v.state_seq,
            done: v.done,
            title: String::new(),
            state: Some(v.state),
            rule: Some(RULE.to_string()),
            pending: v.pending.as_ref().map(|p| p.dialog.clone()),
            pending_seq: v.pending.as_ref().map(|p| p.host_seq),
            cwd: self.cwd.clone(),
            sandbox: self
                .sandbox
                .as_ref()
                .map_or(Mode::None, |s| s.mode)
                .as_str(),
            sandbox_profile: process_sandbox.map_or("", |s| s.profile.as_str()),
            pid: Some(self.pid),
            status: v.status,
            rows: 0,
            cols: 0,
            bytes_seen: v.bytes,
            quiet_for: v.last_activity.elapsed(),
            age: self.started.elapsed(),
            acp: Some(v.facts.clone()),
        }
    }

    /// The transcript as lines (oldest first), the unfinished last line, and how many older lines
    /// were dropped.
    pub(crate) fn transcript(&self) -> (Vec<String>, String, u64) {
        let v = lock(&self.view);
        (
            v.lines.iter().cloned().collect(),
            v.partial.clone(),
            v.dropped,
        )
    }

    /// Sends a prompt. A turn already running keeps going: the prompt waits its turn, because an
    /// ACP agent cannot take a second prompt until the first ends. Refused at a dialog, as for a
    /// pty agent: the dialog is answered, not talked over.
    pub(crate) fn prompt(&self, name: &str, text: &str) -> Result<(), HostError> {
        if lock(&self.view).state == AgentState::Blocked {
            return Err(HostError::Blocked(name.to_string()));
        }
        self.command(name, |ack| Cmd::Prompt(text.to_string(), ack))
    }

    pub(crate) fn cancel(&self, name: &str) -> Result<(), HostError> {
        self.command(name, Cmd::Cancel)
    }

    /// Answers the open dialog `seq` with option `id` (numbered from 1), or declines it.
    pub(crate) fn answer(&self, name: &str, seq: u64, id: Option<u32>) -> Result<(), HostError> {
        let (wire_seq, choice) = {
            let v = lock(&self.view);
            let open = v
                .pending
                .as_ref()
                .ok_or_else(|| HostError::NoDialog(name.to_string()))?;
            if open.host_seq != seq {
                return Err(HostError::StaleDialog {
                    name: name.to_string(),
                    current: open.host_seq,
                });
            }
            let choice = match id {
                None => open.decline.clone(),
                Some(n) => {
                    let at = (n as usize).checked_sub(1);
                    let option = at.and_then(|i| open.option_ids.get(i));
                    Choice::Option(option.ok_or(HostError::NoSuchOption(n))?.clone())
                }
            };
            (open.wire_seq, choice)
        };
        match self.command(name, |ack| Cmd::Answer(wire_seq, choice, ack)) {
            // The dialog moved on between our read and the driver's turn: say which one is open now.
            Err(HostError::Io(_)) => match lock(&self.view).pending.as_ref() {
                Some(open) => Err(HostError::StaleDialog {
                    name: name.to_string(),
                    current: open.host_seq,
                }),
                None => Err(HostError::NoDialog(name.to_string())),
            },
            other => other,
        }
    }

    /// Stops the agent and everything it started. Returns at once; the driver finishes the job.
    pub(crate) fn kill(&self) {
        let _ = self.cmds.send(Cmd::Stop);
    }

    fn command(
        &self,
        name: &str,
        make: impl FnOnce(Sender<Result<(), String>>) -> Cmd,
    ) -> Result<(), HostError> {
        if self.status() != PaneStatus::Running {
            return Err(HostError::Exited(name.to_string()));
        }
        let (ack, done) = channel();
        self.cmds
            .send(make(ack))
            .map_err(|_| HostError::Exited(name.to_string()))?;
        match done.recv_timeout(COMMAND_ACK_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(why)) => Err(HostError::Io(why)),
            Err(RecvTimeoutError::Timeout) => Err(HostError::Timeout(COMMAND_ACK_TIMEOUT)),
            Err(RecvTimeoutError::Disconnected) => Err(HostError::Exited(name.to_string())),
        }
    }
}

impl Drop for AcpPane {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The error with the stderr line that explains it: the last one that names a problem, else the
/// last line. Agents log plenty of unrelated lines after the one that matters.
fn with_stderr(error: &str, conn: &Connection) -> String {
    const PROBLEM_WORDS: [&str; 5] = ["error", "denied", "not permitted", "failed", "traceback"];
    let tail = conn.stderr_tail();
    let problem = tail
        .iter()
        .rev()
        .find(|l| PROBLEM_WORDS.iter().any(|w| l.to_lowercase().contains(w)))
        .or(tail.last());
    match problem {
        Some(line) => format!("{error} ({line})"),
        None => error.to_string(),
    }
}

/// What the driver last announced, so it announces changes and nothing else.
#[derive(Default)]
struct Synced {
    wire_seq: u64,
    lines: usize,
    dropped: u64,
    partial: usize,
}

struct Driver {
    conn: Connection,
    cmds: Receiver<Cmd>,
    view: Arc<Mutex<View>>,
    name: String,
    events: Arc<EventLog>,
    on_exit: ExitHook,
}

impl Driver {
    fn run(mut self) {
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut synced = Synced::default();
        let code = loop {
            if !self.take_commands(&mut queue) {
                break self.stop();
            }
            let heard = match self.conn.pump(TICK) {
                Ok(heard) => heard,
                Err(_) => break self.conn.kill_group() as u32,
            };
            self.send_next_prompt(&mut queue);
            self.sync(&mut synced, heard);
        };
        self.finish(code);
    }

    /// Runs the commands waiting. `false` means stop.
    fn take_commands(&mut self, queue: &mut VecDeque<String>) -> bool {
        loop {
            match self.cmds.try_recv() {
                Ok(Cmd::Prompt(text, ack)) if queue.len() >= MAX_QUEUED_PROMPTS => {
                    let _ = ack.send(Err("too many prompts are waiting for this agent".into()));
                    drop(text);
                }
                Ok(Cmd::Prompt(text, ack)) => {
                    let _ = ack.send(Ok(()));
                    queue.push_back(text);
                }
                Ok(Cmd::Answer(seq, choice, ack)) => {
                    let result = self.conn.answer(seq, &choice).map_err(|e| e.to_string());
                    let _ = ack.send(result);
                }
                Ok(Cmd::Cancel(ack)) => {
                    queue.clear();
                    let _ = ack.send(self.conn.cancel().map_err(|e| e.to_string()));
                }
                Ok(Cmd::Stop) => return false,
                Err(std::sync::mpsc::TryRecvError::Empty) => return true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn send_next_prompt(&mut self, queue: &mut VecDeque<String>) {
        if self.conn.tracker.state() != WireState::Idle {
            return;
        }
        if let Some(text) = queue.pop_front()
            && let Err(e) = self.conn.prompt(&text)
        {
            self.conn
                .tracker
                .record_failure(&format!("prompt not sent: {e}"));
        }
    }

    fn sync(&mut self, synced: &mut Synced, heard: bool) {
        let t = &self.conn.tracker;
        let wire_seq = t.state_seq();
        let mut v = lock(&self.view);
        if heard {
            v.last_activity = Instant::now();
            v.facts = facts_of(t);
        }
        if wire_seq != synced.wire_seq {
            synced.wire_seq = wire_seq;
            let state = host_state(t.state());
            let open = t.pending().map(|p| (p.seq, open_dialog(&p.dialog)));
            let seq = match &open {
                Some(_) => self
                    .events
                    .push_dialog(&self.name, state, Some(RULE.into())),
                None => {
                    self.events
                        .push(&self.name, EventKind::State, Some(state), Some(RULE.into()))
                }
            };
            v.state = state;
            v.state_seq = seq;
            v.done = t.done();
            v.pending = open.map(|(wire_seq, (dialog, option_ids, decline))| OpenDialog {
                host_seq: seq,
                wire_seq,
                dialog: Arc::new(dialog),
                option_ids,
                decline,
            });
        }
        let (n, dropped, partial) = (t.lines().len(), t.dropped(), t.partial().len());
        if (n, dropped, partial) != (synced.lines, synced.dropped, synced.partial) {
            // Lines are numbered from the start of the session; only the ones not mirrored yet
            // are copied, and the mirror is trimmed to the tracker's window.
            let mirrored_total = synced.dropped + synced.lines as u64;
            let first_new = mirrored_total.saturating_sub(dropped) as usize;
            v.lines.extend(t.lines().iter().skip(first_new).cloned());
            while v.lines.len() > n {
                v.lines.pop_front();
            }
            *synced = Synced {
                wire_seq,
                lines: n,
                dropped,
                partial,
            };
            v.bytes += 1;
            v.partial = t.partial().to_string();
            v.dropped = dropped;
        }
    }

    /// Ends the process group: asks politely, then insists.
    fn stop(&mut self) -> u32 {
        self.conn.stop_group(STOP_GRACE) as u32
    }

    /// The process is gone: nothing it started outlives it, the state no longer claims it is
    /// working or at a dialog, and the exit is announced once.
    fn finish(&mut self, code: u32) {
        {
            let mut v = lock(&self.view);
            v.status = PaneStatus::Exited { code };
            v.state = AgentState::Unknown;
            v.pending = None;
        }
        (self.on_exit)();
    }
}

/// The MCP servers to hand the agent in its session: HQ's, when it was asked for and the agent
/// says it can reach an HTTP server. Otherwise none, rather than a server it would ignore.
fn mcp_servers(
    mcp: Option<&crate::host::McpEndpoint>,
    caps: &hq_acp::Capabilities,
) -> serde_json::Value {
    match mcp {
        Some(m) if caps.mcp_http => json!([{
            "type": "http",
            "name": MCP_SERVER_NAME,
            "url": m.url,
            "headers": [{ "name": "Authorization", "value": format!("Bearer {}", m.token) }],
        }]),
        _ => json!([]),
    }
}

fn facts_of(t: &hq_acp::Tracker) -> AcpFacts {
    let (context_used, context_size) = t.context_use();
    let (stop_reason, turn_tokens) = match t.end() {
        Some(TurnEnd::Stopped { reason, usage }) => (Some(stop_label(reason)), usage.total),
        Some(TurnEnd::Failed { error, .. }) => (Some(format!("failed: {}", error.message)), None),
        None => (None, None),
    };
    AcpFacts {
        touched: t.touched().into_iter().collect(),
        stop_reason,
        turn_tokens,
        context_used,
        context_size,
    }
}

fn stop_label(reason: &StopReason) -> String {
    match reason {
        StopReason::EndTurn => "turn done".into(),
        StopReason::Cancelled => "cancelled".into(),
        StopReason::MaxTokens => "out of tokens".into(),
        StopReason::Refusal => "refused".into(),
        StopReason::Other(s) => s.clone(),
    }
}

fn host_state(state: WireState) -> AgentState {
    match state {
        WireState::Idle => AgentState::Idle,
        WireState::Working => AgentState::Working,
        WireState::Blocked => AgentState::Blocked,
    }
}

/// The dialog the way the host shows every dialog, the ids of its options (numbered from 1 for
/// callers), and what declining it sends.
fn open_dialog(d: &hq_acp::Dialog) -> (Dialog, Vec<String>, Choice) {
    let kind = match d.tool {
        ToolKind::Edit => DialogKind::Edit,
        ToolKind::Execute => DialogKind::Command,
        ToolKind::Read => DialogKind::Path,
        ToolKind::Other => DialogKind::Other,
    };
    let options: Vec<DialogOption> = d
        .options
        .iter()
        .enumerate()
        .map(|(i, o)| DialogOption {
            id: i as u32 + 1,
            label: o.label.clone(),
            remember: o.kind.remembers(),
            typed: false,
            selected: false,
        })
        .collect();
    let reject = d.options.iter().find(|o| o.kind == OptionKind::RejectOnce);
    let decline = reject.map_or(Choice::Cancelled, |o| Choice::Option(o.id.clone()));
    let command = d.command.clone().or_else(|| d.paths.first().cloned());
    let mut text = vec![d.title.clone()];
    text.extend(options.iter().map(|o| format!("{}. {}", o.id, o.label)));
    let dialog = Dialog {
        kind,
        question: Some(d.title.clone()).filter(|t| !t.is_empty()),
        command,
        options,
        text: text.join("\n"),
        truncated: false,
        answer: AnswerMethod::Digit,
        decline: Vec::new(),
        cwd: None,
    };
    let ids = d.options.iter().map(|o| o.id.clone()).collect();
    (dialog, ids, decline)
}
