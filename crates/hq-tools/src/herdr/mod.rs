//! Herdr (herdr.dev) as HQ's coding-agent runtime.
//!
//! Herdr owns the terminals, recognises which agent is running in a pane, and
//! reports its lifecycle (`idle`, `working`, `blocked`, `done`). HQ drives it
//! through the `herdr` CLI. A host is either this machine or a remote machine
//! reached over ssh (see `scripts/hq-herdr-gate`), so the same calls work on
//! the VPS and on a laptop that is only sometimes online.
//!
//! Every call is blocking and bounded by a deadline. `HerdrError::Unreachable`
//! means "could not ask"; callers must not read it as "the agent is gone".

mod backend;
mod native;
pub mod pairing;
mod sandbox;
pub mod tools;
mod transport;

use anyhow::Context;
use hq_core::config::{
    HerdrConfig, HerdrHostConfig, HostKind, HqConfig, LOCAL_HOST, NATIVE_HOST, native_host_dir, MAX_LAUNCH_BOUND_SECS, MIN_LAUNCH_BOUND_SECS,
};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use transport::{RawOutput, Transport};

pub use backend::{AwaitingAgent, Host, HostBackend, HostEvent, HostEvents};
pub use native::{NATIVE_GATE_COMMAND, NativeBackend};

/// Herdr rejects explicit timeouts outside this window.
const MIN_WAIT_MS: u64 = 3_000;
const MAX_WAIT_MS: u64 = 300_000;

/// Extra time granted to the process beyond a Herdr-side wait, so Herdr's own
/// timeout error arrives instead of ours.
const WAIT_MARGIN: Duration = Duration::from_secs(15);

const READ_SOURCE: &str = "recent-unwrapped";

/// Herdr refuses a `recent` read of more than ~25 lines while an agent runs on
/// the alternate screen; `visible` has no such limit and is the live screen.
const VISIBLE_SOURCE: &str = "visible";
const AGENT_NOT_IDLE_CODE: &str = "agent_not_idle";

/// Herdr's own stall detector fires after five seconds without activity, so a
/// confirmation wait has to outlast it.
const SUBMIT_CONFIRM: Duration = Duration::from_secs(10);

/// Pane lines quoted when a wrapper never turns into a recognizable agent.
const UNDETECTED_SCREEN_LINES: usize = 20;

/// How often a wrapper launch asks whether Herdr has recognized an agent yet.
const DETECT_POLL: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub enum HerdrError {
    #[error("herdr host '{host}' unreachable: {detail}")]
    Unreachable { host: String, detail: String },
    #[error("herdr {code}: {message}")]
    Api { code: String, message: String },
}

/// `HerdrError::Api` code for a key name that failed `validate_keys`.
pub const INVALID_KEYS_CODE: &str = "invalid_keys";

/// Longest logical key name herdr has (`ctrl+shift+pagedown` is 19).
const MAX_KEY_LEN: usize = 32;

/// Logical key names only (`enter`, `ctrl+c`, `f5`): the first character is
/// alphanumeric so a key can never be read as a herdr flag.
pub fn validate_keys(keys: &[String]) -> Result<(), String> {
    for key in keys {
        let mut chars = key.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
        let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '_' | '-'));
        if !first_ok || !rest_ok || key.len() > MAX_KEY_LEN {
            return Err(format!(
                "invalid key {key:?}: use logical names such as enter, esc, down or ctrl+c (letters, digits, '+', '_', '-'; at most {MAX_KEY_LEN} characters)"
            ));
        }
    }
    Ok(())
}

impl HerdrError {
    pub fn code(&self) -> Option<&str> {
        match self {
            HerdrError::Api { code, .. } => Some(code),
            HerdrError::Unreachable { .. } => None,
        }
    }

    pub fn is_unreachable(&self) -> bool {
        matches!(self, HerdrError::Unreachable { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "idle" => Self::Idle,
            "working" => Self::Working,
            "blocked" => Self::Blocked,
            "done" => Self::Done,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Unknown => "unknown",
        }
    }
}

/// One coding agent as Herdr reports it. Agents a person started by hand have
/// no `name`; those HQ launches always do.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentInfo {
    pub name: Option<String>,
    pub kind: String,
    pub status: AgentStatus,
    pub pane_id: String,
    pub workspace_id: String,
    pub cwd: String,
    pub title: Option<String>,
    /// Bumps on every lifecycle change; lets a caller notify once per change.
    pub state_change_seq: u64,
    /// True while Herdr is still waiting for the agent to reach its prompt.
    pub launch_pending: bool,
    /// The agent's own id for its conversation, when the host knows it (the
    /// built-in host learns it from the agent's hooks). Herdr never does.
    pub agent_session_id: Option<String>,
}

impl AgentInfo {
    /// False while Herdr is still waiting for a launch and cannot classify the
    /// agent: a missing binary leaves a pane in exactly this state forever. An
    /// `unknown` status without `launch_pending` is an agent Herdr just does
    /// not classify, which is running.
    pub fn is_started(&self) -> bool {
        !(self.launch_pending && self.status == AgentStatus::Unknown)
    }

    fn from_json(v: &Value) -> Option<Self> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Self {
            name: text("name"),
            kind: text("agent")?,
            status: AgentStatus::parse(v.get("agent_status")?.as_str()?),
            pane_id: text("pane_id")?,
            workspace_id: text("workspace_id")?,
            cwd: text("cwd").unwrap_or_default(),
            title: text("terminal_title_stripped"),
            state_change_seq: v
                .get("state_change_seq")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            launch_pending: v
                .get("launch_pending")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            agent_session_id: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct LaunchRequest {
    /// Herdr agent name: `[a-z][a-z0-9_-]{0,31}`, unique among live agents.
    pub name: String,
    /// Herdr agent kind (`claude`, `codex`, `cursor`, ...).
    pub kind: String,
    pub cwd: String,
    pub label: String,
    pub env: Vec<(String, String)>,
    /// Native arguments for the agent CLI.
    pub args: Vec<String>,
    /// Wrapper typed into the pane's shell in place of `herdr agent start`,
    /// for a launcher Herdr does not know by name. It must end up running a CLI
    /// Herdr recognizes as `kind`.
    pub command: Option<String>,
    /// Arguments that bring this agent back after the host itself restarts.
    /// Only the built-in host uses them; None means a restart leaves it gone.
    pub resume_args: Option<Vec<String>>,
    /// How the agent connects back to HQ as this session. Only the built-in host
    /// delivers it, as a private config file next to the agent.
    pub mcp: Option<McpAccess>,
    pub start_timeout: Duration,
}

/// The HQ endpoint and the session's own token.
#[derive(Clone)]
pub struct McpAccess {
    pub url: String,
    pub token: String,
}

impl std::fmt::Debug for McpAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpAccess")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Launched {
    pub workspace_id: String,
    pub pane_id: String,
    pub agent: Option<AgentInfo>,
    /// False when the agent is up but blocked on a dialog before its prompt.
    pub ready: bool,
}

#[derive(Debug, Clone)]
pub enum PromptOutcome {
    /// Sent; the caller did not ask to wait.
    Submitted,
    /// Herdr saw no activity, so Enter was pressed once more. Read the screen
    /// to confirm the turn began.
    Resubmitted,
    /// Waited until the agent settled (`idle`, `done` or `blocked`).
    Settled(AgentInfo),
    /// Herdr saw no `working`/`blocked` activity after submission. The prompt
    /// is very likely delivered (a fast agent can finish between polls), so
    /// read the screen before resending.
    Stalled(String),
    TimedOut(String),
}

#[derive(Debug, Clone)]
pub struct HerdrHost {
    name: String,
    transport: Transport,
    session: Option<String>,
    command_timeout: Duration,
    launch_bound: Duration,
    binary_preflight: bool,
}

/// Herdr calls block on a subprocess or ssh, so async callers hand them to the
/// blocking pool instead of stalling a runtime thread.
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> anyhow::Result<T> {
    Ok(tokio::task::spawn_blocking(f).await?)
}

/// The host a caller named, or the configured default.
pub fn host(name: Option<&str>) -> anyhow::Result<Host> {
    let cfg = HqConfig::load()
        .context("loading config for herdr hosts")?
        .herdr;
    build(&cfg, name.unwrap_or(&cfg.default_host))
}

fn build(cfg: &HerdrConfig, name: &str) -> anyhow::Result<Host> {
    if name == NATIVE_HOST {
        let launch = cfg
            .launch_bound_secs
            .clamp(MIN_LAUNCH_BOUND_SECS, MAX_LAUNCH_BOUND_SECS);
        let host = NativeBackend::new(native_host_dir())
            .with_launch_bound(Duration::from_secs(launch))
            .with_command_timeout(Duration::from_secs(cfg.command_timeout_secs))
            .with_sandbox(sandbox::plan(cfg))
        .with_idle_ttl(sandbox::idle_ttl_secs(cfg));
        return Ok(Arc::new(host));
    }
    if let Some(remote) = cfg.hosts.get(name)
        && remote.kind == HostKind::Native
    {
        return Ok(Arc::new(remote_native(cfg, name, remote)));
    }
    Ok(Arc::new(HerdrHost::from_config(cfg, name)?))
}

/// A built-in host on another machine. Its gate command defaults to
/// `hq host gate` unless the config names one.
fn remote_native(cfg: &HerdrConfig, name: &str, remote: &HerdrHostConfig) -> NativeBackend {
    let gate = match remote.gate_command.as_str() {
        "hq-herdr-gate" => NATIVE_GATE_COMMAND,
        other => other,
    };
    let mux_dir = cfg
        .ssh_multiplex
        .then(|| transport::prepare_mux_dir(&HqConfig::hq_dir().join("run").join("ssh")))
        .flatten();
    NativeBackend::remote(name, &remote.ssh, remote.port, remote.identity_file.clone(), gate, mux_dir)
        .with_launch_bound(Duration::from_secs(
            cfg.launch_bound_secs
                .clamp(MIN_LAUNCH_BOUND_SECS, MAX_LAUNCH_BOUND_SECS),
        ))
        .with_command_timeout(Duration::from_secs(cfg.command_timeout_secs))
        .with_sandbox(sandbox::plan(cfg))
        .with_idle_ttl(sandbox::idle_ttl_secs(cfg))
}

/// The machine HQ itself runs on.
pub fn local() -> anyhow::Result<Host> {
    host(Some(LOCAL_HOST))
}

/// Names of the built-in hosts HQ may talk to: this machine's, and any
/// configured remote with `kind: native`.
pub fn native_host_names() -> Vec<String> {
    let remotes = HqConfig::load().map(|c| c.herdr.hosts).unwrap_or_default();
    std::iter::once(NATIVE_HOST.to_string())
        .chain(
            remotes
                .into_iter()
                .filter(|(_, h)| h.kind == HostKind::Native)
                .map(|(name, _)| name),
        )
        .collect()
}

/// This machine plus every configured remote.
pub fn all_hosts() -> anyhow::Result<Vec<Host>> {
    let cfg = HqConfig::load()
        .context("loading config for herdr hosts")?
        .herdr;
    let mut names = vec![LOCAL_HOST.to_string()];
    names.extend(cfg.hosts.keys().cloned());
    let native_up = hq_host::socket_path(&native_host_dir()).exists();
    if native_up || cfg.default_host == NATIVE_HOST {
        names.push(NATIVE_HOST.to_string());
    }
    names.iter().map(|n| build(&cfg, n)).collect()
}

impl HerdrHost {
    pub fn from_config(cfg: &HerdrConfig, name: &str) -> anyhow::Result<Self> {
        let command_timeout = Duration::from_secs(cfg.command_timeout_secs);
        let launch_bound = Duration::from_secs(
            cfg.launch_bound_secs
                .clamp(MIN_LAUNCH_BOUND_SECS, MAX_LAUNCH_BOUND_SECS),
        );
        if name == LOCAL_HOST {
            return Ok(Self {
                name: name.to_string(),
                transport: Transport::Local {
                    binary: cfg.binary.clone(),
                },
                session: cfg.session.clone(),
                command_timeout,
                launch_bound,
                binary_preflight: true,
            });
        }
        let remote = cfg.hosts.get(name).with_context(|| {
            let known: Vec<&str> = cfg.hosts.keys().map(String::as_str).collect();
            format!(
                "unknown herdr host '{name}' (known: {LOCAL_HOST}, {})",
                known.join(", ")
            )
        })?;
        Ok(Self {
            name: name.to_string(),
            transport: Transport::Ssh {
                target: remote.ssh.clone(),
                port: remote.port,
                identity_file: remote.identity_file.clone(),
                gate_command: remote.gate_command.clone(),
                mux_dir: if cfg.ssh_multiplex {
                    transport::prepare_mux_dir(&HqConfig::hq_dir().join("run").join("ssh"))
                } else {
                    None
                },
            },
            session: remote.session.clone(),
            command_timeout,
            launch_bound,
            binary_preflight: true,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether HQ can look for a harness binary itself. A remote host answers
    /// only through its gate, so its binaries are not visible from here.
    pub fn checks_binaries(&self) -> bool {
        self.binary_preflight && matches!(self.transport, Transport::Local { .. })
    }

    /// Longest a launch waits for the agent to come up.
    pub fn launch_bound(&self) -> Duration {
        self.launch_bound
    }

    pub fn with_launch_bound(mut self, bound: Duration) -> Self {
        self.launch_bound = bound;
        self
    }

    /// For tests that drive a fake herdr whose harness binaries do not exist.
    #[cfg(test)]
    pub(crate) fn without_binary_preflight(mut self) -> Self {
        self.binary_preflight = false;
        self
    }

    /// Same host with a different ceiling for non-wait calls. The daemon's
    /// sweep uses a short one so a dead host cannot outlast its task timeout.
    pub fn with_command_timeout(mut self, timeout: Duration) -> Self {
        self.command_timeout = timeout;
        self
    }

    fn argv(&self, args: &[&str]) -> Vec<String> {
        let mut argv = Vec::new();
        if let Some(session) = &self.session {
            argv.extend(["--session".to_string(), session.clone()]);
        }
        argv.extend(args.iter().map(|a| a.to_string()));
        argv
    }

    fn run(&self, args: &[&str], timeout: Duration) -> Result<RawOutput, HerdrError> {
        self.transport.run(&self.name, &self.argv(args), timeout)
    }

    /// Herdr prints `{"id":..,"result":{..}}` on success and
    /// `{"error":{"code":..,"message":..}}` on failure.
    fn json(&self, args: &[&str], timeout: Duration) -> Result<Value, HerdrError> {
        let out = self.run(args, timeout)?;
        if out.exit_code != 0 {
            return Err(api_error(&out));
        }
        let value: Value =
            serde_json::from_str(out.stdout.trim()).map_err(|e| HerdrError::Api {
                code: "bad_response".into(),
                message: format!("not JSON ({e}): {}", truncate(&out.stdout, 200)),
            })?;
        Ok(value.get("result").cloned().unwrap_or(value))
    }

    fn text(&self, args: &[&str], timeout: Duration) -> Result<String, HerdrError> {
        let out = self.run(args, timeout)?;
        if out.exit_code != 0 {
            return Err(api_error(&out));
        }
        Ok(out.stdout)
    }

    pub fn version(&self) -> Result<String, HerdrError> {
        let out = self.text(&["status"], self.command_timeout)?;
        Ok(out
            .lines()
            .find_map(|l| l.trim().strip_prefix("version:"))
            .unwrap_or("unknown")
            .trim()
            .to_string())
    }

    pub fn agents(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        let result = self.json(&["agent", "list"], self.command_timeout)?;
        Ok(result
            .get("agents")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(AgentInfo::from_json).collect())
            .unwrap_or_default())
    }

    /// `Ok(None)` means the host answered and no such agent exists.
    pub fn agent(&self, target: &str) -> Result<Option<AgentInfo>, HerdrError> {
        match self.json(&["agent", "get", target], self.command_timeout) {
            Ok(result) => Ok(result.get("agent").and_then(AgentInfo::from_json)),
            Err(e) if e.code() == Some("agent_not_found") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Opens a workspace and starts the agent in its root pane. A workspace
    /// per session keeps each one closable without touching anyone else's.
    pub fn launch(&self, req: &LaunchRequest) -> Result<Launched, HerdrError> {
        let mut create = vec![
            "workspace",
            "create",
            "--cwd",
            &req.cwd,
            "--label",
            &req.label,
            "--no-focus",
        ];
        let env_pairs: Vec<String> = req.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        for pair in &env_pairs {
            create.extend(["--env", pair]);
        }
        let created = self.json(&create, self.command_timeout)?;
        let workspace_id = str_at(&created, &["workspace", "workspace_id"])?;
        let pane_id = str_at(&created, &["root_pane", "pane_id"])
            .map_err(|e| self.abandon(&workspace_id, e))?;

        let started = match &req.command {
            Some(command) => self.start_wrapped(req, &pane_id, command),
            None => self.start_native(req, &pane_id),
        };
        let result = started
            .and_then(|ready| self.launched(workspace_id.clone(), pane_id, &req.name, ready));
        result.map_err(|e| self.abandon(&workspace_id, e))
    }

    /// Closes a workspace whose launch failed and returns the error to report,
    /// saying so if the close failed too, so nothing is left behind silently.
    fn abandon(&self, workspace_id: &str, error: HerdrError) -> HerdrError {
        let Err(close) = self.close_workspace(workspace_id) else {
            return error;
        };
        let note =
            format!("; workspace {workspace_id} could not be closed ({close}), close it by hand");
        match error {
            HerdrError::Api { code, message } => HerdrError::Api {
                code,
                message: message + &note,
            },
            HerdrError::Unreachable { host, detail } => HerdrError::Unreachable {
                host,
                detail: detail + &note,
            },
        }
    }

    /// `herdr agent start`; `Ok(false)` when the agent is up but blocked on a
    /// dialog before its prompt.
    fn start_native(&self, req: &LaunchRequest, pane_id: &str) -> Result<bool, HerdrError> {
        let start_ms = clamp_ms(req.start_timeout).to_string();
        let mut start = vec![
            "agent",
            "start",
            &req.name,
            "--kind",
            &req.kind,
            "--pane",
            pane_id,
            "--timeout",
            &start_ms,
        ];
        if !req.args.is_empty() {
            start.push("--");
            start.extend(req.args.iter().map(String::as_str));
        }
        match self.json(&start, req.start_timeout + WAIT_MARGIN) {
            Ok(_) => Ok(true),
            Err(e) if e.code() == Some("agent_not_ready") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Types `command` and its arguments into the pane's shell (`pane run`
    /// presses Enter in the same write), waits for Herdr to recognize the agent
    /// that appears, and names it so every later call can address it.
    fn start_wrapped(
        &self,
        req: &LaunchRequest,
        pane_id: &str,
        command: &str,
    ) -> Result<bool, HerdrError> {
        let deadline = Instant::now() + req.start_timeout;
        let line = shell_line(command, &req.args);
        // `pane run` prints nothing on success, so there is no JSON to parse.
        self.text(&["pane", "run", pane_id, &line], self.command_timeout)?;
        if !self.await_agent(pane_id, deadline)? {
            let screen = self
                .read(pane_id, UNDETECTED_SCREEN_LINES)
                .unwrap_or_default();
            return Err(HerdrError::Api {
                code: "agent_not_detected".into(),
                message: format!(
                    "Herdr found no {} agent after running `{line}`; the command must end up running that CLI. Screen:\n{}",
                    req.kind,
                    screen.trim_end()
                ),
            });
        }
        self.json(
            &["agent", "rename", pane_id, &req.name],
            self.command_timeout,
        )?;
        let settle_for = deadline.saturating_duration_since(Instant::now());
        let until = [AgentStatus::Idle, AgentStatus::Done, AgentStatus::Blocked];
        match self.wait(&req.name, &until, settle_for) {
            Ok(agent) => Ok(matches!(
                agent.status,
                AgentStatus::Idle | AgentStatus::Done
            )),
            Err(e) if e.code() == Some("timeout") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// True once Herdr reports any agent in the pane, false if `deadline`
    /// passes first. A status of `unknown` still counts: the agent is there,
    /// Herdr just has not classified it yet.
    fn await_agent(&self, pane_id: &str, deadline: Instant) -> Result<bool, HerdrError> {
        loop {
            if self.agent(pane_id)?.is_some() {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(DETECT_POLL);
        }
    }

    /// Polls until Herdr reports the agent past its launch (see
    /// `AgentInfo::is_started`) or `within` runs out, and returns the last
    /// observation either way. At least one poll is made.
    pub fn await_started(
        &self,
        name: &str,
        within: Duration,
    ) -> Result<Option<AgentInfo>, HerdrError> {
        let deadline = Instant::now() + within;
        loop {
            let seen = self.agent(name)?;
            if seen.as_ref().is_some_and(AgentInfo::is_started) || Instant::now() >= deadline {
                return Ok(seen);
            }
            std::thread::sleep(DETECT_POLL);
        }
    }

    fn launched(
        &self,
        workspace_id: String,
        pane_id: String,
        name: &str,
        ready: bool,
    ) -> Result<Launched, HerdrError> {
        let agent = self.agent(name)?;
        Ok(Launched {
            workspace_id,
            pane_id,
            agent,
            ready,
        })
    }

    /// Submits text to the agent's prompt without confirming it started a turn;
    /// see `submit` for that.
    pub fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, HerdrError> {
        let safe = option_safe(text);
        let mut args = vec!["agent", "prompt", target, &safe];
        let ms = wait.map(|d| clamp_ms(d).to_string());
        if let Some(ms) = &ms {
            args.extend(["--wait", "--timeout", ms]);
        }
        let process_timeout = wait.map_or(self.command_timeout, |d| d + WAIT_MARGIN);
        match self.json(&args, process_timeout) {
            Ok(result) => Ok(result
                .get("agent")
                .and_then(AgentInfo::from_json)
                .map_or(PromptOutcome::Submitted, PromptOutcome::Settled)),
            Err(HerdrError::Api { code, message }) if code == "agent_prompt_stalled" => {
                Ok(PromptOutcome::Stalled(message))
            }
            Err(HerdrError::Api { code, message }) if code == "timeout" => {
                Ok(PromptOutcome::TimedOut(message))
            }
            Err(e) => Err(e),
        }
    }

    /// Submits a prompt and confirms the agent reacted. `agent prompt` sends the
    /// text and Enter together, but a TUI that is not listening yet can swallow
    /// the Enter and leave the text sitting in its input box. Herdr reports that
    /// as a stall, so press Enter once more. On an empty input Enter does
    /// nothing, which makes the retry safe when the agent had simply finished.
    pub fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, HerdrError> {
        let safe = option_safe(text);
        let ms = clamp_ms(SUBMIT_CONFIRM).to_string();
        let args = [
            "agent",
            "prompt",
            target,
            &safe,
            "--wait",
            "--until",
            "working",
            "--until",
            "blocked",
            "--until",
            "done",
            "--timeout",
            &ms,
        ];
        match self.json(&args, SUBMIT_CONFIRM + WAIT_MARGIN) {
            Ok(_) => Ok(PromptOutcome::Submitted),
            Err(HerdrError::Api { code, .. }) if code == "agent_prompt_stalled" => {
                self.send_keys(target, &["enter".to_string()])?;
                Ok(PromptOutcome::Resubmitted)
            }
            Err(HerdrError::Api { code, message }) if code == "timeout" => {
                Ok(PromptOutcome::TimedOut(message))
            }
            Err(e) => Err(e),
        }
    }

    /// Logical keys (`enter`, `esc`, `down`, `ctrl+c`) for answering a dialog.
    pub fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), HerdrError> {
        validate_keys(keys).map_err(|message| HerdrError::Api {
            code: INVALID_KEYS_CODE.to_string(),
            message,
        })?;
        let mut args = vec!["agent", "send-keys", target];
        args.extend(keys.iter().map(String::as_str));
        self.json(&args, self.command_timeout).map(|_| ())
    }

    /// Types text into a pane without pressing Enter.
    pub fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        self.json(&["pane", "send-text", pane_id, text], self.command_timeout)
            .map(|_| ())
    }

    /// Recent output with soft wraps joined. Works for any pane id, so it also
    /// reads agents a person started.
    pub fn read(&self, target: &str, lines: usize) -> Result<String, HerdrError> {
        self.read_sourced(target, lines).map(|(text, _)| text)
    }

    /// `read` plus the herdr `--source` that produced the text. A working
    /// agent refuses a deep `recent` read (`agent_not_idle`), so that one
    /// error, and only it, is retried once as a `visible` read. Reads are
    /// idempotent, and the retry is a separate logical call from the
    /// transport's pre-session ssh retry, so neither multiplies the other.
    pub fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), HerdrError> {
        match self.read_from(target, lines, READ_SOURCE) {
            Ok(text) => Ok((text, READ_SOURCE)),
            Err(HerdrError::Api { code, .. }) if code == AGENT_NOT_IDLE_CODE => self
                .read_from(target, lines, VISIBLE_SOURCE)
                .map(|text| (text, VISIBLE_SOURCE)),
            Err(e) => Err(e),
        }
    }

    fn read_from(&self, target: &str, lines: usize, source: &str) -> Result<String, HerdrError> {
        let n = lines.to_string();
        self.text(
            &["agent", "read", target, "--source", source, "--lines", &n],
            self.command_timeout,
        )
    }

    pub fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, HerdrError> {
        let ms = clamp_ms(timeout).to_string();
        let mut args = vec!["agent", "wait", target];
        for status in until {
            args.extend(["--until", status.as_str()]);
        }
        args.extend(["--timeout", &ms]);
        let result = self.json(&args, timeout + WAIT_MARGIN)?;
        result
            .get("agent")
            .and_then(AgentInfo::from_json)
            .ok_or_else(|| HerdrError::Api {
                code: "bad_response".into(),
                message: "wait returned no agent".into(),
            })
    }

    pub fn close_workspace(&self, workspace_id: &str) -> Result<(), HerdrError> {
        self.json(&["workspace", "close", workspace_id], self.command_timeout)
            .map(|_| ())
    }

    /// PID of the pane's shell, for callers that also track a local process.
    pub fn shell_pid(&self, pane_id: &str) -> Option<u32> {
        let result = self
            .json(
                &["pane", "process-info", "--pane", pane_id],
                self.command_timeout,
            )
            .ok()?;
        let pid = result.get("process_info")?.get("shell_pid")?.as_u64()?;
        u32::try_from(pid).ok()
    }
}

/// Text starting with `-` would be read as an option by Herdr's parser, so it
/// gets a leading space.
fn option_safe(text: &str) -> String {
    if text.starts_with('-') {
        format!(" {text}")
    } else {
        text.to_string()
    }
}

/// `command` is typed as written, so it may carry its own arguments or a `~`.
/// Arguments are quoted for a POSIX shell.
fn shell_line(command: &str, args: &[String]) -> String {
    let mut line = command.to_string();
    for arg in args {
        line.push(' ');
        line.push_str(&shell_quote(arg));
    }
    line
}

fn shell_quote(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c));
    if plain {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

fn clamp_ms(d: Duration) -> u64 {
    (d.as_millis() as u64).clamp(MIN_WAIT_MS, MAX_WAIT_MS)
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn str_at(v: &Value, path: &[&str]) -> Result<String, HerdrError> {
    let mut cur = v;
    for key in path {
        cur = cur.get(*key).unwrap_or(&Value::Null);
    }
    cur.as_str()
        .map(str::to_string)
        .ok_or_else(|| HerdrError::Api {
            code: "bad_response".into(),
            message: format!("missing {}", path.join(".")),
        })
}

fn api_error(out: &RawOutput) -> HerdrError {
    for stream in [&out.stderr, &out.stdout] {
        let parsed: Option<Value> = serde_json::from_str(stream.trim()).ok();
        let Some(err) = parsed.as_ref().and_then(|v| v.get("error")) else {
            continue;
        };
        return HerdrError::Api {
            code: err
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_string(),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        };
    }
    HerdrError::Api {
        code: format!("exit_{}", out.exit_code),
        message: truncate(&format!("{} {}", out.stderr.trim(), out.stdout.trim()), 300),
    }
}

#[cfg(test)]
mod tests;
