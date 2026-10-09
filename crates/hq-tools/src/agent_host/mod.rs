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
#[cfg(unix)]
mod native;
#[cfg(unix)]
pub mod pairing;
#[cfg(any(test, feature = "test-support"))]
pub mod scripted;
mod sandbox;
pub mod tools;
#[cfg(unix)]
mod transport;

use anyhow::Context;
use hq_core::config::{AgentHostConfig, HqConfig, LOCAL_HOST, NATIVE_HOST};
#[cfg(unix)]
use hq_core::config::{
    MAX_LAUNCH_BOUND_SECS, MIN_LAUNCH_BOUND_SECS, RemoteHostConfig, native_host_dir,
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
    Settled(AgentInfo),
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
    build(&cfg, name.unwrap_or(&cfg.default_host))
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
        format!("unknown host '{name}' (known: {NATIVE_HOST}, {})", known.join(", "))
    })?;
    Ok(Arc::new(remote_native(cfg, name, remote)))
}

/// A built-in host on another machine. Its gate command defaults to
/// `hq host gate` unless the config names one.
#[cfg(unix)]
fn remote_native(cfg: &AgentHostConfig, name: &str, remote: &RemoteHostConfig) -> NativeBackend {
    let gate = remote.gate_command.as_str();
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

/// Names of the hosts HQ may talk to: this machine's built-in host and every
/// configured remote.
pub fn native_host_names() -> Vec<String> {
    let remotes = HqConfig::load().map(|c| c.agent_host.hosts).unwrap_or_default();
    std::iter::once(NATIVE_HOST.to_string()).chain(remotes.into_keys()).collect()
}

/// This machine's host plus every configured remote.
pub fn all_hosts() -> anyhow::Result<Vec<Host>> {
    let cfg = HqConfig::load()
        .context("loading config for hosts")?
        .agent_host;
    let mut names = vec![NATIVE_HOST.to_string()];
    names.extend(cfg.hosts.keys().cloned());
    names.iter().map(|n| build(&cfg, n)).collect()
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
            let cfg = AgentHostConfig { launch_bound_secs: secs, ..AgentHostConfig::default() };
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
        let err = build(&cfg, "nope").err().map(|e| e.to_string()).unwrap_or_default();
        assert!(err.contains("laptop") && err.contains("native"), "{err}");
    }
}
