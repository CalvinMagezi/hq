//! HQ's coding-agent runtime: the built-in host (`hq host`).
//!
//! The host owns the terminals, recognises which agent is running in a pane, and
//! reports its lifecycle (`idle`, `working`, `blocked`, `done`). HQ drives it
//! over its control socket. A host is either this machine or a remote machine
//! reached over ssh through `hq host gate`, so the same calls work on the VPS
//! and on a laptop that is only sometimes online.
//!
//! Every call is blocking and bounded by a deadline. `AgentHostError::Unreachable`
//! means "could not ask"; callers must not read it as "the agent is gone".

// Off Unix the host backend is not built, which leaves its helpers unused.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

mod backend;
pub mod bridge_hosts;
#[cfg(unix)]
mod native;
#[cfg(unix)]
pub mod pairing;
// Builds on the SSH transport, which is Unix-only.
mod sandbox;
#[cfg(all(unix, any(test, feature = "test-support")))]
pub mod scripted;
pub mod tools;
#[cfg(unix)]
mod transport;

use anyhow::Context;
use hq_core::config::{AgentHostConfig, HqConfig, LOCAL_HOST, NATIVE_HOST};
#[cfg(unix)]
use hq_core::config::{
    HostTransport, MAX_LAUNCH_BOUND_SECS, MIN_LAUNCH_BOUND_SECS, RemoteHostConfig, native_host_dir,
};
use serde::Serialize;
#[cfg(unix)]
use std::sync::Arc;
use std::time::Duration;

pub use backend::{AwaitingAgent, Host, HostBackend, HostEvent, HostEvents};

#[cfg(unix)]
pub use native::{NATIVE_GATE_COMMAND, NativeBackend};

#[derive(Debug, thiserror::Error)]
pub enum AgentHostError {
    #[error("host '{host}' unreachable: {detail}")]
    Unreachable { host: String, detail: String },
    #[error("host {code}: {message}")]
    Api { code: String, message: String },
}

/// `AgentHostError::Api` code for a key name that failed `validate_keys`.
pub const INVALID_KEYS_CODE: &str = "invalid_keys";

/// Longest logical key name the host accepts (`ctrl+shift+pagedown` is 19).
const MAX_KEY_LEN: usize = 32;

/// Logical key names only (`enter`, `ctrl+c`, `f5`): the first character is
/// alphanumeric so a key can never be read as a host flag.
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

impl AgentHostError {
    pub fn code(&self) -> Option<&str> {
        match self {
            AgentHostError::Api { code, .. } => Some(code),
            AgentHostError::Unreachable { .. } => None,
        }
    }

    pub fn is_unreachable(&self) -> bool {
        matches!(self, AgentHostError::Unreachable { .. })
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

/// One coding agent as the host reports it. Agents a person started by hand have
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
    /// True while the host is still waiting for the agent to reach its prompt.
    pub launch_pending: bool,
    /// The agent's own id for its conversation, when the host knows it (the
    /// built-in host learns it from the agent's hooks). The host never does.
    pub agent_session_id: Option<String>,
    /// The process has ended but the host still lists the pane (its screen can
    /// still be read). HQ treats such an agent as gone.
    pub exited: bool,
    /// How the process ended, when `exited` and the host says. Zero is a run
    /// that finished; anything else is a crash or a refusal.
    pub exit_code: Option<u32>,
    /// The rule that decided the status (`hook:<event>` when the agent's own hook did).
    pub rule: Option<String>,
    /// The dialog a blocked agent waits at, shortened; `HostBackend::pending` reads it whole.
    pub pending: Option<Box<PendingDialog>>,
    /// The number an answer to `pending` must name; a new dialog gets a new one.
    pub pending_seq: Option<u64>,
}

/// A dialog an agent waits at, as the host read it off the screen.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct PendingDialog {
    /// `command`, `edit`, `path`, `question`, `trust` or `other`.
    pub kind: String,
    pub question: Option<String>,
    /// The command, file or path the dialog is about.
    pub command: Option<String>,
    pub options: Vec<PendingOption>,
    /// Something was cut: the whole dialog was too long, or (in a listing) a field was shortened.
    #[serde(default)]
    pub truncated: bool,
    /// Every line of the dialog; only in a full read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct PendingOption {
    pub id: u32,
    pub label: String,
    /// Picking it lets the agent do more from now on without asking ("don't ask again").
    #[serde(default)]
    pub remember: bool,
    /// Picking it opens a text field instead of answering.
    #[serde(default)]
    pub typed: bool,
    /// The cursor is on it.
    #[serde(default)]
    pub selected: bool,
}

impl PendingDialog {
    pub fn option(&self, id: u32) -> Option<&PendingOption> {
        self.options.iter().find(|o| o.id == id)
    }
}

/// What answering a dialog did.
#[derive(Debug, Clone)]
pub struct AnswerOutcome {
    /// The dialog went away, or another took its place.
    pub registered: bool,
    /// The agent after the answer, when the host said.
    pub agent: Option<AgentInfo>,
}

/// Output read from a cursor: what scrolled off the screen since it, and the screen now.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LogsSince {
    /// Lines that scrolled off since the cursor, oldest first.
    pub lines: Vec<String>,
    /// New lines left out because there were more than asked for (the oldest).
    pub skipped: usize,
    pub screen: Vec<String>,
    /// The cursor for the next read.
    pub next: String,
    /// The cursor could not be placed: `lines` are the newest lines, not the new ones.
    pub reset: bool,
}

/// What a worktree is checked out at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeBase {
    /// A writer's branch: checked out if it exists, else made from the folder's current commit.
    Branch(String),
    /// A commit (or a ref naming one) a reviewer checks out detached.
    Detached(String),
}

/// A worktree the host made or found again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct WorktreeInfo {
    pub path: String,
    /// The repository's main folder.
    pub repo: String,
    pub branch: Option<String>,
    pub head: String,
    /// False when it already existed (a retry or a resume).
    #[serde(default)]
    pub created: bool,
}

/// The commit an agent's folder is at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct GitHead {
    pub sha: String,
    pub subject: String,
    pub branch: Option<String>,
}

/// What removing a worktree did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub struct WorktreeRemoved {
    pub removed: bool,
    /// It has changes nobody committed, so it was kept.
    pub dirty: bool,
}

/// A host reply that did not have the shape its method promises.
pub(crate) fn bad_reply(method: &str, e: impl std::fmt::Display) -> AgentHostError {
    AgentHostError::Api {
        code: "bad_reply".into(),
        message: format!("the host's {method} reply could not be read: {e}"),
    }
}

/// A dialog read whole, with what an answer needs.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRead {
    /// The number an answer must name.
    pub seq: u64,
    /// The rule that recognised the dialog.
    pub rule: Option<String>,
    pub dialog: PendingDialog,
}

/// A host's `agent.pending` reply; `None` when the agent waits at no dialog.
pub(crate) fn parse_pending(v: &serde_json::Value) -> Option<PendingRead> {
    Some(PendingRead {
        seq: v.get("pending_seq")?.as_u64()?,
        rule: v.get("rule").and_then(|r| r.as_str()).map(str::to_string),
        dialog: serde_json::from_value(v.get("pending")?.clone()).ok()?,
    })
}

impl AgentInfo {
    /// False while the host is still waiting for a launch and cannot classify the
    /// agent: a missing binary leaves a pane in exactly this state forever. An
    /// `unknown` status without `launch_pending` is an agent the host just does
    /// not classify, which is running.
    pub fn is_started(&self) -> bool {
        !(self.launch_pending && self.status == AgentStatus::Unknown)
    }
}

#[derive(Debug, Clone)]
pub struct LaunchRequest {
    /// host agent name: `[a-z][a-z0-9_-]{0,31}`, unique among live agents.
    pub name: String,
    /// host agent kind (`claude`, `codex`, `cursor`, ...).
    pub kind: String,
    pub cwd: String,
    pub label: String,
    pub env: Vec<(String, String)>,
    /// Native arguments for the agent CLI.
    pub args: Vec<String>,
    /// Wrapper typed into the pane's shell in place of `agent start`,
    /// for a launcher the host does not know by name. It must end up running a CLI
    /// The host recognizes as `kind`.
    pub command: Option<String>,
    /// Arguments that bring this agent back after the host itself restarts.
    /// Only the built-in host uses them; None means a restart leaves it gone.
    pub resume_args: Option<Vec<String>>,
    /// How the agent connects back to HQ as this session. Only the built-in host
    /// delivers it, as a private config file next to the agent.
    pub mcp: Option<McpAccess>,
    pub start_timeout: Duration,
    /// The command policy the agent starts under (`hq_core::command_policy::Policy` as JSON).
    /// The built-in host turns it into a Claude Code hook that refuses what it denies.
    pub policy: Option<serde_json::Value>,
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
    /// The host saw no activity, so Enter was pressed once more. Read the screen
    /// to confirm the turn began.
    Resubmitted,
    /// Waited until the agent settled (`idle`, `done` or `blocked`).
    Settled(Box<AgentInfo>),
    /// The host saw no `working`/`blocked` activity after submission. The prompt
    /// is very likely delivered (a fast agent can finish between polls), so
    /// read the screen before resending.
    Stalled(String),
    TimedOut(String),
}

/// The host calls block on a subprocess or ssh, so async callers hand them to the
/// blocking pool instead of stalling a runtime thread.
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> anyhow::Result<T> {
    Ok(tokio::task::spawn_blocking(f).await?)
}

/// The host a caller named, or the configured default.
pub fn host(name: Option<&str>) -> anyhow::Result<Host> {
    let cfg = HqConfig::load()
        .context("loading config for hosts")?
        .agent_host;
    let name = name.unwrap_or(&cfg.default_host).to_string();
    build_with(&cfg, &bridge_hosts::load(), &name)
}

/// A host by name, among the configured ones and those bridge setup made; configured ones win.
fn build_with(
    cfg: &AgentHostConfig,
    setup: &[bridge_hosts::SetupHost],
    name: &str,
) -> anyhow::Result<Host> {
    build(&bridge_hosts::for_host(cfg, setup, name), name)
}

#[cfg(not(unix))]
fn build(_cfg: &AgentHostConfig, name: &str) -> anyhow::Result<Host> {
    anyhow::bail!(
        "coding-agent host '{name}' is not available on this platform: the host runs on Linux, \
         macOS and WSL2. HQ Lite on Windows has no coding agents; use Full HQ in WSL2."
    )
}

#[cfg(unix)]
fn build(cfg: &AgentHostConfig, name: &str) -> anyhow::Result<Host> {
    // `local` was the host on this machine; it now names the built-in one.
    if name == NATIVE_HOST || name == LOCAL_HOST {
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
    let remote = cfg.hosts.get(name).with_context(|| {
        let known: Vec<&str> = cfg.hosts.keys().map(String::as_str).collect();
        format!(
            "unknown host '{name}' (known: {NATIVE_HOST}, {})",
            known.join(", ")
        )
    })?;
    Ok(Arc::new(remote_native(cfg, name, remote)?))
}

/// A built-in host on another machine, through ssh or the signed TCP gate. Its ssh gate command
/// defaults to `hq host gate` unless the config names one.
#[cfg(unix)]
fn remote_native(
    cfg: &AgentHostConfig,
    name: &str,
    remote: &RemoteHostConfig,
) -> anyhow::Result<NativeBackend> {
    let backend = match remote.transport {
        HostTransport::Ssh => ssh_backend(cfg, name, remote)?,
        HostTransport::Tcp => tcp_backend(name, remote)?,
    };
    Ok(backend
        .with_launch_bound(Duration::from_secs(
            cfg.launch_bound_secs
                .clamp(MIN_LAUNCH_BOUND_SECS, MAX_LAUNCH_BOUND_SECS),
        ))
        .with_command_timeout(Duration::from_secs(cfg.command_timeout_secs))
        .with_sandbox(sandbox::plan(cfg))
        .with_idle_ttl(sandbox::idle_ttl_secs(cfg)))
}

#[cfg(unix)]
fn ssh_backend(
    cfg: &AgentHostConfig,
    name: &str,
    remote: &RemoteHostConfig,
) -> anyhow::Result<NativeBackend> {
    anyhow::ensure!(
        !remote.ssh.trim().is_empty(),
        "host '{name}' uses the ssh transport and needs `ssh: user@address`"
    );
    let mux_dir = cfg
        .ssh_multiplex
        .then(|| transport::prepare_mux_dir(&HqConfig::hq_dir().join("run").join("ssh")))
        .flatten();
    Ok(NativeBackend::remote(
        name,
        &remote.ssh,
        remote.port,
        remote.identity_file.clone(),
        remote.gate_command.as_str(),
        mux_dir,
    ))
}

#[cfg(unix)]
fn tcp_backend(name: &str, remote: &RemoteHostConfig) -> anyhow::Result<NativeBackend> {
    let addr = remote.addr.as_deref().with_context(|| {
        format!("host '{name}' uses the tcp transport and needs `addr: <tunnel address>:<port>`")
    })?;
    let addr: std::net::SocketAddr = addr
        .parse()
        .with_context(|| format!("host '{name}': addr {addr:?} is not an address:port"))?;
    let key_path = match remote.gate_key_file.as_deref() {
        Some(path) => expand_home(path),
        None => gate_key_path(name),
    };
    Ok(NativeBackend::remote_tcp(name, addr, key_path))
}

/// Where this server keeps its gate key for the host `name`.
pub fn gate_key_path(name: &str) -> std::path::PathBuf {
    HqConfig::hq_dir().join("gate").join(format!("{name}.key"))
}

/// The key file for the host `name` as the config sets it, or the default location.
pub fn configured_gate_key_path(name: &str) -> std::path::PathBuf {
    let custom = HqConfig::load().ok().and_then(|c| {
        c.agent_host
            .hosts
            .get(name)
            .and_then(|h| h.gate_key_file.clone())
    });
    match custom {
        Some(path) => expand_home(&path),
        None => gate_key_path(name),
    }
}

/// The file that lists which keys may call this machine's TCP gate.
pub fn gate_authorized_keys_path(cfg: &AgentHostConfig) -> std::path::PathBuf {
    match cfg.gate_authorized_keys.as_deref() {
        Some(path) => expand_home(path),
        None => HqConfig::hq_dir().join("gate_authorized_keys"),
    }
}

/// A leading `~/` means the home directory, as people write it in a config.
pub fn expand_home(path: &str) -> std::path::PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::PathBuf::from(home).join(rest),
        _ => std::path::PathBuf::from(path),
    }
}

/// The machine HQ itself runs on.
pub fn local() -> anyhow::Result<Host> {
    host(Some(LOCAL_HOST))
}

/// Names of the hosts HQ may talk to: this machine's built-in host and every
/// configured remote.
pub fn native_host_names() -> Vec<String> {
    let cfg = HqConfig::load().map(|c| c.agent_host).unwrap_or_default();
    let remotes = bridge_hosts::merged(cfg, &bridge_hosts::load()).hosts;
    std::iter::once(NATIVE_HOST.to_string())
        .chain(remotes.into_keys())
        .collect()
}

/// This machine's host plus every configured remote.
pub fn all_hosts() -> anyhow::Result<Vec<Host>> {
    let cfg = HqConfig::load()
        .context("loading config for hosts")?
        .agent_host;
    let setup = bridge_hosts::load();
    let mut names = vec![NATIVE_HOST.to_string()];
    names.extend(bridge_hosts::merged(cfg.clone(), &setup).hosts.into_keys());
    names.iter().map(|n| build_with(&cfg, &setup, n)).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_bound_comes_from_config_and_stays_under_the_transport_limit() {
        let bound = |secs: u64| {
            let cfg = AgentHostConfig {
                launch_bound_secs: secs,
                ..AgentHostConfig::default()
            };
            build(&cfg, NATIVE_HOST).unwrap().launch_bound()
        };
        assert_eq!(AgentHostConfig::default().launch_bound_secs, 25);
        assert_eq!(bound(25), Duration::from_secs(25));
        assert_eq!(bound(1), Duration::from_secs(5));
        assert_eq!(bound(600), Duration::from_secs(50));
    }

    #[test]
    fn local_names_the_built_in_host_and_an_unknown_name_lists_the_known_ones() {
        let mut cfg = AgentHostConfig::default();
        cfg.hosts.insert(
            "laptop".into(),
            serde_yaml::from_str("ssh: \"me@100.64.0.1\"").unwrap(),
        );
        assert_eq!(build(&cfg, LOCAL_HOST).unwrap().name(), NATIVE_HOST);
        assert_eq!(build(&cfg, "laptop").unwrap().name(), "laptop");
        let err = build(&cfg, "nope")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(err.contains("laptop") && err.contains("native"), "{err}");
    }
}
