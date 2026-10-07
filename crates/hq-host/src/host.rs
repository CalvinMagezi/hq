//! The set of agents this host runs, and the operations on them.

use crate::detect::{AgentState, Detector, Input};
use crate::emu::{Row, VtEmulator};
use crate::env::pane_env;
use crate::error::HostError;
use crate::events::{EventKind, EventLog};
use crate::keys::encode_key;
use crate::report;
use crate::sandbox::{self, Mode, SandboxSpec};
use crate::egress::{Decision, Egress};
use crate::token;
pub use crate::pane::PaneStatus;
use crate::pane::{ExitHook, LaunchArgs, Pane, Resume};
use crate::state::{PaneRecord, StateFile};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const MAX_NAME_LEN: usize = 32;
const DEFAULT_ROWS: u16 = 40;
const DEFAULT_COLS: u16 = 120;
const DEFAULT_SCROLLBACK_ROWS: usize = 10_000;
/// Rows and columns each must be in `1..=MAX_DIMENSION`; the screen grid is
/// allocated up front and a zero size panics inside the emulator.
const MAX_DIMENSION: u16 = 1000;
const MAX_SCROLLBACK_ROWS: usize = 100_000;
/// Pause between pasting a prompt and pressing Enter, so a TUI that handles the
/// paste first does not swallow the Enter.
const PROMPT_ENTER_DELAY: Duration = Duration::from_millis(150);
/// How often `wait_state` looks at the screen again.
const STATE_POLL: Duration = Duration::from_millis(100);
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

/// Agent names: `[a-z][a-z0-9_-]{0,31}`.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
        && name.len() <= MAX_NAME_LEN
}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    /// Variables the process gets in addition to the allowlisted ones.
    pub env: Vec<(String, String)>,
    /// Which agent this is (`claude`, `codex`, ...), so its state can be
    /// detected from the screen. None for a plain process.
    pub agent: Option<String>,
    /// How to start this agent again after a host restart (for example
    /// `claude --continue`). Without it the agent is not brought back.
    pub resume_argv: Option<Vec<String>>,
    pub rows: u16,
    pub cols: u16,
    pub scrollback_rows: usize,
    /// Confinement for the process; None starts it unsandboxed.
    pub sandbox: Option<SandboxSpec>,
    /// Stop the agent once it has been silent and not working for this long, so
    /// agents nobody is watching do not pile up. It stays resumable.
    pub idle_ttl: Option<Duration>,
}

impl SpawnSpec {
    pub fn new(name: impl Into<String>, argv: Vec<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            argv,
            cwd: cwd.into(),
            env: Vec::new(),
            agent: None,
            resume_argv: None,
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            scrollback_rows: DEFAULT_SCROLLBACK_ROWS,
            sandbox: None,
            idle_ttl: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadSource {
    /// Rows on screen now.
    Visible,
    /// Scrollback and screen, rows as displayed.
    Recent,
    /// Scrollback and screen, wrapped rows joined back into logical lines.
    RecentUnwrapped,
}

#[derive(Debug, Clone)]
pub struct PaneInfo {
    pub name: String,
    pub argv: Vec<String>,
    pub agent: Option<String>,
    /// Will be started again after a host restart.
    pub resumable: bool,
    /// The agent's own id for its conversation, once its hooks have reported
    /// it. This is what `--resume` takes for agents that have one.
    pub agent_session_id: Option<String>,
    /// Goes up each time the agent's state changes; 0 before the first look.
    pub state_seq: u64,
    /// A turn finished and the agent has not worked since (`done`).
    pub done: bool,
    /// The terminal title the program set, or empty.
    pub title: String,
    /// Detected state; None when the agent kind has no rule file.
    pub state: Option<AgentState>,
    /// The rule that decided the state; None when the default applied.
    pub rule: Option<String>,
    pub cwd: PathBuf,
    /// `process` or `none`; what the agent is confined by.
    pub sandbox: &'static str,
    pub pid: Option<u32>,
    pub status: PaneStatus,
    pub rows: u16,
    pub cols: u16,
    pub bytes_seen: u64,
    pub quiet_for: Duration,
    pub age: Duration,
}

/// Most agents the host runs at once, and the most bytes of command line one
/// may be started with. Both bound what a caller of the socket can make the host
/// hold.
const MAX_AGENTS: usize = 128;
const MAX_ARGV_BYTES: usize = 64 * 1024;
/// Longest MCP address and token the host will write into a config file.
const MAX_MCP_URL_BYTES: usize = 2048;
const MAX_MCP_TOKEN_BYTES: usize = 256;
/// Longest conversation id an agent may report. They are short tokens; anything
/// else would end up in a restart command line.
const MAX_SESSION_ID_CHARS: usize = 128;

/// How often the state watcher looks at every agent.
const STATE_WATCH_INTERVAL: Duration = Duration::from_millis(250);

/// Environment variables a pane gets so its hooks can reach the host.
pub const PANE_TOKEN_ENV: &str = "HQ_HOST_TOKEN";
pub const RUN_DIR_ENV: &str = "HQ_HOST_DIR";

type Registry = Arc<Mutex<BTreeMap<String, Arc<Pane>>>>;
/// Agents restored from the state file that cannot start until their
/// environment is supplied again.
type Awaiting = Arc<Mutex<BTreeMap<String, PaneRecord>>>;

/// An agent that is waiting for its environment before it can resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitingInfo {
    pub name: String,
    pub agent: Option<String>,
    pub cwd: PathBuf,
    /// The variables `Host::resume` must supply.
    pub env_keys: Vec<String>,
}

pub struct Host {
    panes: Registry,
    awaiting: Awaiting,
    max_agents: usize,
    events: Arc<EventLog>,
    /// Per-agent secret each pane gets so its hooks can report on itself and
    /// nothing else. Kept in memory only.
    pane_tokens: Mutex<BTreeMap<String, String>>,
    /// Where the control socket lives, passed to panes so hooks can find it.
    run_dir: Mutex<Option<PathBuf>>,
    detector: Detector,
    state: Option<Arc<StateFile>>,
    egress: Arc<Egress>,
    /// Refuse to start an agent that is not under the process sandbox.
    require_sandbox: bool,
    /// The `hq` binary a Linux sandbox runs its egress relay with; this process's
    /// own binary when unset (right for `hq host serve`).
    helper: Option<PathBuf>,
}

/// What `Host::restore` did.
#[derive(Debug, Default)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    /// `(name, reason)`; the name is `session.json` when the file itself was refused.
    pub skipped: Vec<(String, String)>,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unwrap_rows(rows: Vec<Row>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for row in rows {
        current.push_str(&row.text);
        if !row.wrapped {
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

impl Host {
    pub fn new() -> Self {
        Self::with_detector(Detector::builtin())
    }

    /// A host that detects agent state with `detector` (for example one with
    /// local rule-file overrides loaded).
    pub fn with_detector(detector: Detector) -> Self {
        Self {
            panes: Arc::new(Mutex::new(BTreeMap::new())),
            awaiting: Arc::new(Mutex::new(BTreeMap::new())),
            max_agents: MAX_AGENTS,
            events: Arc::new(EventLog::from_clock()),
            pane_tokens: Mutex::new(BTreeMap::new()),
            run_dir: Mutex::new(None),
            detector,
            state: None,
            egress: Arc::new(Egress::new()),
            require_sandbox: false,
            helper: None,
        }
    }

    /// A host that starts only agents under the process sandbox. A caller that
    /// asks for none, or says nothing, gets an error: the machine's owner decides
    /// this, not whoever drives the host.
    pub fn with_require_sandbox(mut self, require: bool) -> Self {
        self.require_sandbox = require;
        self
    }

    /// The binary a Linux sandbox starts as its egress relay. Needed only where this
    /// process is not itself `hq` (a test binary, an embedding).
    pub fn with_helper_binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.helper = Some(path.into());
        self
    }

    /// A host that runs at most `max` agents at once (default 128).
    pub fn with_max_agents(mut self, max: usize) -> Self {
        self.max_agents = max;
        self
    }

    /// A host that remembers its resumable agents in `<dir>/session.json`.
    pub fn with_state_dir(mut self, dir: &Path) -> Self {
        self.state = Some(Arc::new(StateFile::new(dir)));
        self
    }

    fn pane(&self, name: &str) -> Result<Arc<Pane>, HostError> {
        lock(&self.panes)
            .get(name)
            .cloned()
            .ok_or_else(|| HostError::NotFound(name.to_string()))
    }

    pub fn spawn(&self, spec: SpawnSpec) -> Result<PaneInfo, HostError> {
        if !valid_name(&spec.name) {
            return Err(HostError::InvalidName(spec.name));
        }
        check_size(spec.rows, spec.cols)?;
        if self.require_sandbox && spec.sandbox.as_ref().is_none_or(|s| s.mode != Mode::Process) {
            return Err(HostError::Sandbox(
                "this host only starts sandboxed agents; the machine's owner can start it with --allow-unsandboxed".into(),
            ));
        }
        let argv_bytes = |v: &[String]| v.iter().map(String::len).sum::<usize>();
        if argv_bytes(&spec.argv) > MAX_ARGV_BYTES
            || spec.resume_argv.as_deref().is_some_and(|a| argv_bytes(a) > MAX_ARGV_BYTES)
        {
            return Err(HostError::TooLarge("the command line"));
        }
        if lock(&self.panes).len() + lock(&self.awaiting).len() >= self.max_agents {
            return Err(HostError::TooManyAgents(self.max_agents));
        }

        if lock(&self.panes).contains_key(&spec.name)
            || lock(&self.awaiting).contains_key(&spec.name)
        {
            return Err(HostError::NameTaken(spec.name));
        }
        // Start the process without holding the registry lock: looking up the
        // program and forking can be slow and must not stall every other call.
        let scrollback_rows = spec.scrollback_rows.min(MAX_SCROLLBACK_ROWS);
        let resume = match spec.resume_argv {
            Some(argv) if argv.is_empty() => return Err(HostError::InvalidResume),
            Some(argv) => Some(Resume {
                argv,
                env: spec.env.clone(),
                scrollback_rows,
            }),
            None => None,
        };
        let token = token::random_hex().map_err(|e| HostError::Io(e.to_string()))?;
        let mut env = pane_env(&spec.env);
        env.push((PANE_TOKEN_ENV.to_string(), token.clone()));
        if let Some(dir) = lock(&self.run_dir).as_ref() {
            env.push((RUN_DIR_ENV.to_string(), dir.to_string_lossy().into_owned()));
        }
        let confined = match spec.sandbox.as_ref().filter(|s| s.mode == Mode::Process) {
            Some(sb) => {
                let run_dir = lock(&self.run_dir).clone();
                Some(sandbox::confine(
                    &self.egress,
                    sb,
                    &sandbox::Launch {
                        name: &spec.name,
                        cwd: &spec.cwd,
                        run_dir: run_dir.as_deref(),
                        argv: &spec.argv,
                        agent: spec.agent.as_deref(),
                        env: &spec.env,
                        helper: self.helper.as_deref(),
                    },
                )?)
            }
            None => None,
        };
        let (exec, egress_port) = match confined {
            Some(c) => {
                env.extend(c.env);
                (Some(c.argv), Some(c.egress_port))
            }
            None => (None, None),
        };
        let pane = Pane::spawn(
            LaunchArgs {
                argv: spec.argv,
                exec,
                sandbox: spec.sandbox,
                resume,
                on_exit: Some(self.exit_hook(&spec.name, egress_port)),
                agent: spec.agent,
                idle_ttl: spec.idle_ttl,
                cwd: spec.cwd,
                env,
                rows: spec.rows,
                cols: spec.cols,
            },
            Box::new(VtEmulator::new(spec.rows, spec.cols, scrollback_rows)),
        );
        let pane = match pane {
            Ok(p) => p,
            Err(e) => {
                if let Some(port) = egress_port {
                    self.egress.close_port(&spec.name, port);
                }
                return Err(e);
            }
        };
        let pane = Arc::new(pane);
        let mut panes = lock(&self.panes);
        if panes.contains_key(&spec.name) {
            pane.kill();
            return Err(HostError::NameTaken(spec.name));
        }
        // The token goes in before the pane is findable, so a leftover process
        // holding an earlier agent's token for this name never authenticates.
        lock(&self.pane_tokens).insert(spec.name.clone(), token);
        panes.insert(spec.name.clone(), pane.clone());
        drop(panes);
        self.events.push(&spec.name, EventKind::Spawned, None, None);
        self.save();
        Ok(info_of(&spec.name, pane.as_ref(), &self.detector))
    }

    /// The flags that point a Claude Code agent at a hook file for `name`,
    /// writing the file first. The caller puts them in the agent's command
    /// line, wherever that command ends up. Empty for other kinds, when the
    /// host does not know its socket directory, or when the file cannot be
    /// written (the screen still decides the state then).
    pub fn hook_flags(&self, name: &str, agent: &str) -> Vec<String> {
        let dir = lock(&self.run_dir).clone();
        let (true, Some(dir)) = (agent == "claude" && valid_name(name), dir) else {
            return Vec::new();
        };
        let mut flags = Vec::new();
        // An MCP config written for this agent goes along, so a restart or a
        // changed resume command keeps the agent connected to HQ.
        if let Some(mcp) = crate::hooks::claude_mcp_config_path(&dir, name) {
            flags.extend(["--mcp-config".to_string(), mcp.to_string_lossy().into_owned()]);
        }
        let exe = std::env::current_exe().map_or_else(
            |_| "hq".to_string(),
            |p| p.to_string_lossy().into_owned(),
        );
        let command = format!("{} host report", shell_quote(&exe));
        match crate::hooks::write_claude_settings(&dir, name, &command) {
            Ok(path) => {
                flags.extend(["--settings".to_string(), path.to_string_lossy().into_owned()]);
                flags
            }
            Err(e) => {
                eprintln!("hq host: no hook settings for '{name}': {e}");
                flags
            }
        }
    }

    /// Deletes the hook and MCP config files written for `name`. The MCP file
    /// holds the agent's HQ token, so it must not outlive the agent.
    fn forget_files(&self, name: &str) {
        self.egress.forget(name);
        if let Some(dir) = lock(&self.run_dir).clone() {
            crate::hooks::remove_agent_files(&dir, name);
        }
    }

    /// Deletes hook and MCP config files that belong to no agent the host
    /// knows, such as those a crash or an abandoned launch left behind.
    fn sweep_agent_files(&self) {
        let Some(dir) = lock(&self.run_dir).clone() else {
            return;
        };
        let mut known: Vec<String> = lock(&self.panes).keys().cloned().collect();
        known.extend(lock(&self.awaiting).keys().cloned());
        crate::hooks::sweep_agent_files(&dir, &known);
    }

    /// Writes the MCP config that connects a Claude Code agent to HQ as the
    /// launched session `name`, holding that session's token. `hook_flags`
    /// returns the flag that points at it. Does nothing for other kinds or a
    /// host that does not know its socket directory.
    pub fn write_mcp_config(
        &self,
        name: &str,
        agent: &str,
        url: &str,
        token: &str,
    ) -> Result<(), HostError> {
        let dir = lock(&self.run_dir).clone();
        let (true, Some(dir)) = (agent == "claude" && valid_name(name), dir) else {
            return Ok(());
        };
        if url.len() > MAX_MCP_URL_BYTES || token.len() > MAX_MCP_TOKEN_BYTES {
            return Err(HostError::TooLarge("the MCP address or token"));
        }
        crate::hooks::write_claude_mcp_config(&dir, name, url, token)
            .map(|_| ())
            .map_err(|e| HostError::Io(e.to_string()))
    }

    /// Rewrites the state file with the agents that are running and can be
    /// resumed. A failure is reported on stderr and never fails the caller: an
    /// agent that cannot be remembered still runs.
    fn save(&self) {
        save_state(&self.panes, &self.awaiting, self.state.as_deref());
    }

    fn exit_hook(&self, name: &str, egress_port: Option<u16>) -> ExitHook {
        // The registry is captured only to rewrite the state file. A host with
        // none must not be kept alive by its own panes, or dropping it would
        // leave the processes running.
        let saver = self
            .state
            .clone()
            .map(|state| (state, self.panes.clone(), self.awaiting.clone()));
        let (events, name) = (self.events.clone(), name.to_string());
        let egress = self.egress.clone();
        Arc::new(move || {
            if let Some(port) = egress_port {
                egress.close_port(&name, port);
            }
            if let Some((state, panes, awaiting)) = &saver {
                save_state(panes, awaiting, Some(state));
            }
            events.push(&name, EventKind::Exited, None, None);
        })
    }

    /// Starts again every agent the state file lists, using its resume
    /// command. Call once, before serving.
    pub fn restore(&self) -> RestoreReport {
        let mut report = RestoreReport::default();
        let Some(state) = &self.state else {
            return report;
        };
        let records = match state.read() {
            Ok(records) => records,
            Err(e) => {
                report
                    .skipped
                    .push(("session.json".to_string(), e.to_string()));
                return report;
            }
        };
        for rec in records {
            if !rec.env_keys.is_empty() {
                lock(&self.awaiting).insert(rec.name.clone(), rec);
                continue;
            }
            let name = rec.name.clone();
            match self.spawn(spec_of(rec, Vec::new())) {
                Ok(_) => report.restored.push(name),
                Err(e) => report.skipped.push((name, e.to_string())),
            }
        }
        self.sweep_agent_files();
        report
    }

    /// Agents restored from the state file that wait for their environment.
    pub fn awaiting(&self) -> Vec<AwaitingInfo> {
        lock(&self.awaiting)
            .values()
            .map(|r| AwaitingInfo {
                name: r.name.clone(),
                agent: r.agent.clone(),
                cwd: r.cwd.clone(),
                env_keys: r.env_keys.clone(),
            })
            .collect()
    }

    /// Starts a waiting agent with the environment its starter supplies again.
    /// It stays waiting if a variable is missing or the start fails.
    pub fn resume(&self, name: &str, env: Vec<(String, String)>) -> Result<PaneInfo, HostError> {
        let rec = lock(&self.awaiting)
            .get(name)
            .cloned()
            .ok_or_else(|| HostError::NotFound(name.to_string()))?;
        let missing: Vec<&str> = rec
            .env_keys
            .iter()
            .filter(|k| !env.iter().any(|(have, _)| have == *k))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            return Err(HostError::MissingEnv {
                name: name.to_string(),
                missing: missing.join(", "),
            });
        }
        // Spawn sees the name as taken while it is waiting, so release it first
        // and put it back if the start fails.
        lock(&self.awaiting).remove(name);
        match self.spawn(spec_of(rec.clone(), env)) {
            Ok(info) => Ok(info),
            Err(e) => {
                lock(&self.awaiting).insert(rec.name.clone(), rec);
                Err(e)
            }
        }
    }

    /// Keeps the state file as it is, then stops every agent. Call when the
    /// host is going away: the agents are stopped but still listed, so the next
    /// start brings them back.
    pub fn shutdown(&self) {
        self.save();
        if let Some(state) = &self.state {
            state.freeze();
        }
        let panes: Vec<Arc<Pane>> = lock(&self.panes).values().cloned().collect();
        for pane in panes {
            pane.kill();
        }
    }

    /// The recent allow and deny decisions of `name`'s egress proxy.
    pub fn egress_decisions(&self, name: &str) -> Vec<Decision> {
        self.egress.decisions(name)
    }

    pub fn events(&self) -> &EventLog {
        &self.events
    }

    /// Starts a thread that announces state changes the screen shows, such as
    /// a dialog appearing or an agent without hooks finishing. Changes an agent
    /// reports are announced at once by `report`. It ends when `stop` is set.
    pub fn watch_states(self: &Arc<Self>, stop: Arc<std::sync::atomic::AtomicBool>) {
        let host = Arc::downgrade(self);
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let Some(host) = host.upgrade() else { return };
                let names: Vec<String> = lock(&host.panes).keys().cloned().collect();
                for name in &names {
                    host.announce(name);
                }
                host.reap_idle(&names);
                drop(host);
                std::thread::sleep(STATE_WATCH_INTERVAL);
            }
        });
    }

    /// Sends a `state` event if the agent's state differs from the last one
    /// announced. A first sighting counts, so a report that lands before the
    /// first look is not missed. The agent's change counter goes up before
    /// `done` is set, so a reader that sees `done` also sees the number to
    /// alert on.
    fn announce(&self, name: &str) {
        let Ok(pane) = self.pane(name) else { return };
        let info = info_of(name, pane.as_ref(), &self.detector);
        let Some(state) = info.state else { return };
        let mut announced = pane.announced();
        if *announced == Some(state) {
            return;
        }
        let before = announced.replace(state);
        if state == AgentState::Working {
            pane.set_done(false);
        }
        let seq = self
            .events
            .push(name, EventKind::State, Some(state), info.rule);
        pane.set_state_seq(seq);
        if state == AgentState::Idle && before == Some(AgentState::Working) {
            pane.set_done(true);
        }
    }

    /// Stops agents that outlived their idle limit: silent that long, and not
    /// working. The pane is removed, so it is not restored at the next start; its
    /// owner finds it gone and can resume the conversation.
    fn reap_idle(&self, names: &[String]) {
        for name in names {
            let Ok(pane) = self.pane(name) else { continue };
            let Some(ttl) = pane.idle_ttl else { continue };
            if pane.quiet_for() < ttl || pane.status() != PaneStatus::Running {
                continue;
            }
            let state = info_of(name, pane.as_ref(), &self.detector).state;
            if state == Some(AgentState::Working) {
                continue;
            }
            eprintln!("hq host: stopping '{name}', idle for over {}s", ttl.as_secs());
            let _ = self.remove(name);
        }
    }

    /// Tells the host where its control socket is, so panes can be given the
    /// way to reach it. Called by the server when it binds.
    pub fn set_run_dir(&self, dir: &Path) {
        *lock(&self.run_dir) = Some(dir.to_path_buf());
    }

    /// The agent a pane token belongs to, if that agent still exists.
    pub fn agent_for_token(&self, given: &str) -> Option<String> {
        let tokens = lock(&self.pane_tokens);
        let name = tokens
            .iter()
            .find(|(_, t)| token::matches(t, given))
            .map(|(n, _)| n.clone())?;
        drop(tokens);
        lock(&self.panes).contains_key(&name).then_some(name)
    }

    /// Replaces the command that brings a resumable agent back after a host
    /// restart, for example once its conversation id is known.
    pub fn set_resume(&self, name: &str, argv: Vec<String>) -> Result<(), HostError> {
        let pane = self.pane(name)?;
        if argv.is_empty() || pane.resume.is_none() {
            return Err(HostError::InvalidResume);
        }
        pane.set_resume_argv(argv);
        self.save();
        Ok(())
    }

    /// Records what an agent's hook said: the state it implies, and the id of
    /// the agent's own conversation. An event that says nothing is ignored.
    pub fn report(
        &self,
        name: &str,
        event: &str,
        notification_type: Option<&str>,
        session_id: Option<String>,
    ) -> Result<(), HostError> {
        let pane = self.pane(name)?;
        // An id that is not a plain token is dropped, not stored: it ends up in
        // the agent's restart command line.
        let session_id = session_id.filter(|id| valid_session_id(id));
        pane.record_report(report::state_for(event, notification_type), event, session_id);
        self.announce(name);
        Ok(())
    }

    pub fn info(&self, name: &str) -> Result<PaneInfo, HostError> {
        Ok(info_of(name, self.pane(name)?.as_ref(), &self.detector))
    }

    pub fn list(&self) -> Vec<PaneInfo> {
        lock(&self.panes)
            .iter()
            .map(|(n, p)| info_of(n, p, &self.detector))
            .collect()
    }

    /// Forgets an agent and stops its process and the processes it started.
    pub fn remove(&self, name: &str) -> Result<(), HostError> {
        let Some(pane) = lock(&self.panes).remove(name) else {
            if lock(&self.awaiting).remove(name).is_some() {
                self.forget_files(name);
                self.save();
                self.events.push(name, EventKind::Removed, None, None);
                return Ok(());
            }
            return Err(HostError::NotFound(name.to_string()));
        };
        lock(&self.pane_tokens).remove(name);
        self.save();
        // Other threads may still hold the pane (a caller waiting on it), so
        // stopping it cannot be left to the last reference being dropped.
        pane.kill();
        self.forget_files(name);
        self.events.push(name, EventKind::Removed, None, None);
        Ok(())
    }

    /// The last `lines` lines of the chosen view (all of them when `lines` is 0).
    pub fn read(&self, name: &str, source: ReadSource, lines: usize) -> Result<String, HostError> {
        let pane = self.pane(name)?;
        let mut out: Vec<String> = match source {
            ReadSource::Visible => pane.rows(false).into_iter().map(|r| r.text).collect(),
            ReadSource::Recent => pane.rows(true).into_iter().map(|r| r.text).collect(),
            ReadSource::RecentUnwrapped => unwrap_rows(pane.rows(true)),
        };
        if lines > 0 && out.len() > lines {
            out.drain(..out.len() - lines);
        }
        Ok(out.join("\n"))
    }

    fn live_pane(&self, name: &str) -> Result<Arc<Pane>, HostError> {
        let pane = self.pane(name)?;
        if pane.status() != PaneStatus::Running {
            return Err(HostError::Exited(name.to_string()));
        }
        Ok(pane)
    }

    /// Raw bytes, exactly as given.
    pub fn send_text(&self, name: &str, text: &str) -> Result<(), HostError> {
        self.live_pane(name)?.write(text.as_bytes())
    }

    /// Text as a paste: bracketed when the program asked for it, so embedded
    /// newlines are not taken for Enter.
    pub fn paste(&self, name: &str, text: &str) -> Result<(), HostError> {
        let pane = self.live_pane(name)?;
        if pane.with_emu(|e| e.bracketed_paste()) {
            // The text cannot end the paste early: whatever followed a forged
            // end marker would be typed as keystrokes.
            let mut bytes = BRACKETED_PASTE_START.to_vec();
            bytes.extend_from_slice(without_paste_markers(text).as_bytes());
            bytes.extend_from_slice(BRACKETED_PASTE_END);
            pane.write(&bytes)
        } else {
            pane.write(text.as_bytes())
        }
    }

    /// Logical key names (`enter`, `esc`, `ctrl+c`, ...), validated before anything is sent.
    pub fn send_keys(&self, name: &str, keys: &[String]) -> Result<(), HostError> {
        let mut bytes = Vec::new();
        for key in keys {
            bytes.extend(encode_key(key)?);
        }
        self.live_pane(name)?.write(&bytes)
    }

    /// Pastes `text` and presses Enter.
    pub fn prompt(&self, name: &str, text: &str) -> Result<(), HostError> {
        self.paste(name, text)?;
        std::thread::sleep(PROMPT_ENTER_DELAY);
        self.send_keys(name, &["enter".to_string()])
    }

    pub fn resize(&self, name: &str, rows: u16, cols: u16) -> Result<(), HostError> {
        check_size(rows, cols)?;
        self.live_pane(name)?.resize(rows, cols)
    }

    pub fn wait_exit(&self, name: &str, timeout: Duration) -> Result<u32, HostError> {
        self.pane(name)?.wait_exit(timeout)
    }

    pub fn wait_quiet(
        &self,
        name: &str,
        quiet: Duration,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.pane(name)?.wait_quiet(quiet, timeout)
    }

    /// Waits until the detected state is one of `states` and has held for
    /// `stable`, so a flicker between two screens is not taken for a change.
    /// An agent kind without a rule file never matches and times out.
    pub fn wait_state(
        &self,
        name: &str,
        states: &[AgentState],
        stable: Duration,
        timeout: Duration,
    ) -> Result<PaneInfo, HostError> {
        let deadline = Instant::now() + timeout;
        let mut since: Option<(AgentState, Instant)> = None;
        loop {
            let info = self.info(name)?;
            match info.state {
                Some(state) if states.contains(&state) => {
                    let held = match since {
                        Some((s, at)) if s == state => at,
                        _ => Instant::now(),
                    };
                    since = Some((state, held));
                    if held.elapsed() >= stable {
                        return Ok(info);
                    }
                }
                _ => since = None,
            }
            if info.status != PaneStatus::Running && info.state.is_none() {
                return Err(HostError::Exited(name.to_string()));
            }
            if Instant::now() >= deadline {
                return Err(HostError::Timeout(timeout));
            }
            std::thread::sleep(STATE_POLL);
        }
    }

    pub fn kill(&self, name: &str) -> Result<(), HostError> {
        self.pane(name)?.kill();
        // Do not wait for the exit: a host that dies now must not bring it back.
        self.save();
        Ok(())
    }
}

/// Writes the resumable running agents to the state file, if there is one.
fn save_state(panes: &Registry, awaiting: &Awaiting, state: Option<&StateFile>) {
    let Some(state) = state else { return };
    let waiting: Vec<PaneRecord> = lock(awaiting).values().cloned().collect();
    let records: Vec<PaneRecord> = lock(panes)
        .iter()
        .filter(|(_, pane)| pane.status() == PaneStatus::Running && !pane.is_stopping())
        .filter_map(|(name, pane)| {
            let resume = pane.resume.as_ref()?;
            let resume_argv = pane.resume_argv()?;
            let (rows, cols) = pane.size();
            Some(PaneRecord {
                name: name.clone(),
                resume_argv,
                agent: pane.agent.clone(),
                cwd: pane.cwd.clone(),
                env_keys: resume.env.iter().map(|(k, _)| k.clone()).collect(),
                rows,
                cols,
                scrollback_rows: resume.scrollback_rows,
                sandbox: pane.sandbox.clone(),
                idle_ttl_secs: pane.idle_ttl.map(|t| t.as_secs()),
            })
        })
        .chain(waiting)
        .collect();
    if let Err(e) = state.write(&records) {
        eprintln!("hq host: could not save session.json: {e}");
    }
}

/// Whether `id` looks like an agent's conversation id: letters, digits, `-`, `_`
/// and `.`, at most `MAX_SESSION_ID_CHARS`, and not starting with `-` (which a
/// command line would read as a flag).
fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SESSION_ID_CHARS
        && !id.starts_with('-')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `text` without the bracketed-paste start and end sequences.
fn without_paste_markers(text: &str) -> String {
    let (start, end) = (
        String::from_utf8_lossy(BRACKETED_PASTE_START),
        String::from_utf8_lossy(BRACKETED_PASTE_END),
    );
    let mut clean = text.to_string();
    // Removing one can join the pieces around it into another, so repeat.
    while clean.contains(start.as_ref()) || clean.contains(end.as_ref()) {
        clean = clean.replace(start.as_ref(), "").replace(end.as_ref(), "");
    }
    clean
}

/// `arg` as one shell word.
fn shell_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+=:@%".contains(c));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

fn spec_of(rec: PaneRecord, env: Vec<(String, String)>) -> SpawnSpec {
    let mut spec = SpawnSpec::new(rec.name, rec.resume_argv.clone(), rec.cwd);
    spec.agent = rec.agent;
    spec.env = env;
    spec.resume_argv = Some(rec.resume_argv);
    spec.rows = rec.rows;
    spec.cols = rec.cols;
    spec.scrollback_rows = rec.scrollback_rows;
    spec.sandbox = rec.sandbox;
    spec.idle_ttl = rec.idle_ttl_secs.map(Duration::from_secs);
    spec
}

fn check_size(rows: u16, cols: u16) -> Result<(), HostError> {
    let ok = |n: u16| (1..=MAX_DIMENSION).contains(&n);
    if ok(rows) && ok(cols) {
        Ok(())
    } else {
        Err(HostError::InvalidSize {
            rows,
            cols,
            max: MAX_DIMENSION,
        })
    }
}

/// The screen as detection sees it: visible rows as text, plus the title.
fn screen_of(pane: &Pane) -> (String, String) {
    pane.with_emu(|e| {
        let rows: Vec<String> = e.visible().into_iter().map(|r| r.text).collect();
        (rows.join("\n"), e.title())
    })
}

fn info_of(name: &str, pane: &Pane, detector: &Detector) -> PaneInfo {
    let (rows, cols) = pane.size();
    let detection = pane.agent.as_deref().and_then(|agent| {
        let (screen, title) = screen_of(pane);
        detector.detect(
            agent,
            Input {
                screen: &screen,
                osc_title: &title,
            },
        )
    });
    let screen = detection.as_ref().map(|d| d.state);
    let reported = pane.reported();
    let (state, from_hook) = report::combine(
        reported.as_ref().map(|r| r.state),
        screen,
        pane.quiet_for(),
    );
    let rule = match (&reported, from_hook) {
        (Some(r), true) => Some(format!("hook:{}", r.event)),
        _ => detection.and_then(|d| d.rule),
    };
    PaneInfo {
        name: name.to_string(),
        argv: pane.argv.clone(),
        agent: pane.agent.clone(),
        resumable: pane.resume.is_some(),
        agent_session_id: pane.agent_session_id(),
        state_seq: pane.state_seq(),
        done: pane.is_done(),
        title: pane.with_emu(|e| e.title()),
        state,
        rule,
        cwd: pane.cwd.clone(),
        sandbox: pane.sandbox.as_ref().map_or(Mode::None, |s| s.mode).as_str(),
        pid: pane.pid,
        status: pane.status(),
        rows,
        cols,
        bytes_seen: pane.bytes_seen(),
        quiet_for: pane.quiet_for(),
        age: pane.started.elapsed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_agent_name_rule() {
        for ok in ["a", "hs-claude-abc123", "x_1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "A", "1a", "-a", "a b", "a.b", &"a".repeat(33)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }
}
