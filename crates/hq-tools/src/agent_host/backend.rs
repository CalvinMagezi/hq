use super::{
    AgentInfo, AgentStatus, AgentHostError, LaunchRequest, Launched, PromptOutcome,
};
use std::sync::Arc;
use std::time::Duration;

/// What HQ needs from whatever hosts coding agents: the host protocol today, a
/// built-in host later. Calls block, like the host protocol they wrap, so async
/// callers use [`super::blocking`].
pub trait HostBackend: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &str;
    /// Whether HQ can look for a harness binary itself.
    fn checks_binaries(&self) -> bool;
    /// Longest a launch waits for the agent to come up.
    fn launch_bound(&self) -> Duration;

    fn version(&self) -> Result<String, AgentHostError>;
    fn agents(&self) -> Result<Vec<AgentInfo>, AgentHostError>;
    fn agent(&self, target: &str) -> Result<Option<AgentInfo>, AgentHostError>;
    fn launch(&self, req: &LaunchRequest) -> Result<Launched, AgentHostError>;
    fn await_started(&self, name: &str, within: Duration) -> Result<Option<AgentInfo>, AgentHostError>;
    fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, AgentHostError>;
    fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, AgentHostError>;
    fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), AgentHostError>;
    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), AgentHostError>;
    fn read(&self, target: &str, lines: usize) -> Result<String, AgentHostError>;
    fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), AgentHostError>;
    fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, AgentHostError>;
    fn close_workspace(&self, workspace_id: &str) -> Result<(), AgentHostError>;
    fn shell_pid(&self, pane_id: &str) -> Option<u32>;

    /// Whether this host can hand an agent a private config pointing back at
    /// HQ (the built-in host can; an external host cannot).
    fn accepts_mcp(&self) -> bool {
        false
    }

    /// Waits up to `wait` for events after `after` (`None` just reads the
    /// current position). Only the built-in host can push events; others are
    /// swept by polling.
    fn poll_events(&self, _after: Option<u64>, _wait: Duration) -> Result<HostEvents, AgentHostError> {
        Err(AgentHostError::Api {
            code: "unsupported".into(),
            message: format!("host '{}' has no event stream", self.name()),
        })
    }

    /// Agents the host restored but is holding until their env is supplied.
    /// Only the built-in host has any.
    fn awaiting(&self) -> Result<Vec<AwaitingAgent>, AgentHostError> {
        Ok(Vec::new())
    }
    /// Tells the host the command that brings `name` back after the host
    /// restarts. Only the built-in host restarts agents, so others ignore it.
    fn update_resume(&self, _name: &str, _kind: &str, _args: Vec<String>) -> Result<(), AgentHostError> {
        Ok(())
    }
    fn resume_awaiting(&self, name: &str, _env: Vec<(String, String)>) -> Result<(), AgentHostError> {
        Err(AgentHostError::Api {
            code: "unsupported".into(),
            message: format!("host '{}' does not hold agents for resume ({name})", self.name()),
        })
    }

    /// The same host with a different launch ceiling.
    fn with_launch_bound_dyn(&self, bound: Duration) -> Host;
    /// The same host with a different ceiling for non-wait calls.
    fn with_command_timeout_dyn(&self, timeout: Duration) -> Host;
}

/// One thing that happened to an agent on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEvent {
    pub seq: u64,
    pub name: String,
    /// `spawned`, `state`, `exited` or `removed`.
    pub kind: String,
    pub state: Option<String>,
}

/// What a wait for host events returned.
#[derive(Debug, Clone, Default)]
pub struct HostEvents {
    pub events: Vec<HostEvent>,
    /// Pass this as `after` next time.
    pub last_seq: u64,
    /// The host dropped events the caller had not read; re-read everything.
    pub lost: bool,
}

/// A restored agent that cannot restart until HQ supplies its environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitingAgent {
    pub name: String,
    pub env_keys: Vec<String>,
}

/// A shared handle to a host, as callers hold it.
pub type Host = Arc<dyn HostBackend>;


impl<T: HostBackend + ?Sized> HostBackend for Arc<T> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn checks_binaries(&self) -> bool {
        (**self).checks_binaries()
    }
    fn launch_bound(&self) -> Duration {
        (**self).launch_bound()
    }
    fn version(&self) -> Result<String, AgentHostError> {
        (**self).version()
    }
    fn agents(&self) -> Result<Vec<AgentInfo>, AgentHostError> {
        (**self).agents()
    }
    fn agent(&self, target: &str) -> Result<Option<AgentInfo>, AgentHostError> {
        (**self).agent(target)
    }
    fn launch(&self, req: &LaunchRequest) -> Result<Launched, AgentHostError> {
        (**self).launch(req)
    }
    fn await_started(&self, name: &str, within: Duration) -> Result<Option<AgentInfo>, AgentHostError> {
        (**self).await_started(name, within)
    }
    fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, AgentHostError> {
        (**self).prompt(target, text, wait)
    }
    fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, AgentHostError> {
        (**self).submit(target, text)
    }
    fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), AgentHostError> {
        (**self).send_keys(target, keys)
    }
    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), AgentHostError> {
        (**self).send_text(pane_id, text)
    }
    fn read(&self, target: &str, lines: usize) -> Result<String, AgentHostError> {
        (**self).read(target, lines)
    }
    fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), AgentHostError> {
        (**self).read_sourced(target, lines)
    }
    fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, AgentHostError> {
        (**self).wait(target, until, timeout)
    }
    fn close_workspace(&self, workspace_id: &str) -> Result<(), AgentHostError> {
        (**self).close_workspace(workspace_id)
    }
    fn shell_pid(&self, pane_id: &str) -> Option<u32> {
        (**self).shell_pid(pane_id)
    }
    fn accepts_mcp(&self) -> bool {
        (**self).accepts_mcp()
    }
    fn poll_events(&self, after: Option<u64>, wait: Duration) -> Result<HostEvents, AgentHostError> {
        (**self).poll_events(after, wait)
    }
    fn update_resume(&self, name: &str, kind: &str, args: Vec<String>) -> Result<(), AgentHostError> {
        (**self).update_resume(name, kind, args)
    }
    fn awaiting(&self) -> Result<Vec<AwaitingAgent>, AgentHostError> {
        (**self).awaiting()
    }
    fn resume_awaiting(&self, name: &str, env: Vec<(String, String)>) -> Result<(), AgentHostError> {
        (**self).resume_awaiting(name, env)
    }
    fn with_launch_bound_dyn(&self, bound: Duration) -> Host {
        (**self).with_launch_bound_dyn(bound)
    }
    fn with_command_timeout_dyn(&self, timeout: Duration) -> Host {
        (**self).with_command_timeout_dyn(timeout)
    }
}
