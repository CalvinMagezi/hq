use super::{
    AgentInfo, AgentStatus, HerdrError, HerdrHost, LaunchRequest, Launched, PromptOutcome,
};
use std::sync::Arc;
use std::time::Duration;

/// What HQ needs from whatever hosts coding agents: the herdr CLI today, a
/// built-in host later. Calls block, like the herdr CLI they wrap, so async
/// callers use [`super::blocking`].
pub trait HostBackend: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &str;
    /// Whether HQ can look for a harness binary itself.
    fn checks_binaries(&self) -> bool;
    /// Longest a launch waits for the agent to come up.
    fn launch_bound(&self) -> Duration;

    fn version(&self) -> Result<String, HerdrError>;
    fn agents(&self) -> Result<Vec<AgentInfo>, HerdrError>;
    fn agent(&self, target: &str) -> Result<Option<AgentInfo>, HerdrError>;
    fn launch(&self, req: &LaunchRequest) -> Result<Launched, HerdrError>;
    fn await_started(
        &self,
        name: &str,
        within: Duration,
    ) -> Result<Option<AgentInfo>, HerdrError>;
    fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, HerdrError>;
    fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, HerdrError>;
    fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), HerdrError>;
    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError>;
    fn read(&self, target: &str, lines: usize) -> Result<String, HerdrError>;
    fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), HerdrError>;
    fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, HerdrError>;
    fn close_workspace(&self, workspace_id: &str) -> Result<(), HerdrError>;
    fn shell_pid(&self, pane_id: &str) -> Option<u32>;

    /// The same host with a different launch ceiling.
    fn with_launch_bound_dyn(&self, bound: Duration) -> Host;
    /// The same host with a different ceiling for non-wait calls.
    fn with_command_timeout_dyn(&self, timeout: Duration) -> Host;
}

/// A shared handle to a host, as callers hold it.
pub type Host = Arc<dyn HostBackend>;

impl HostBackend for HerdrHost {
    fn name(&self) -> &str {
        HerdrHost::name(self)
    }
    fn checks_binaries(&self) -> bool {
        HerdrHost::checks_binaries(self)
    }
    fn launch_bound(&self) -> Duration {
        HerdrHost::launch_bound(self)
    }
    fn version(&self) -> Result<String, HerdrError> {
        HerdrHost::version(self)
    }
    fn agents(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        HerdrHost::agents(self)
    }
    fn agent(&self, target: &str) -> Result<Option<AgentInfo>, HerdrError> {
        HerdrHost::agent(self, target)
    }
    fn launch(&self, req: &LaunchRequest) -> Result<Launched, HerdrError> {
        HerdrHost::launch(self, req)
    }
    fn await_started(
        &self,
        name: &str,
        within: Duration,
    ) -> Result<Option<AgentInfo>, HerdrError> {
        HerdrHost::await_started(self, name, within)
    }
    fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, HerdrError> {
        HerdrHost::prompt(self, target, text, wait)
    }
    fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, HerdrError> {
        HerdrHost::submit(self, target, text)
    }
    fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), HerdrError> {
        HerdrHost::send_keys(self, target, keys)
    }
    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        HerdrHost::send_text(self, pane_id, text)
    }
    fn read(&self, target: &str, lines: usize) -> Result<String, HerdrError> {
        HerdrHost::read(self, target, lines)
    }
    fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), HerdrError> {
        HerdrHost::read_sourced(self, target, lines)
    }
    fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, HerdrError> {
        HerdrHost::wait(self, target, until, timeout)
    }
    fn close_workspace(&self, workspace_id: &str) -> Result<(), HerdrError> {
        HerdrHost::close_workspace(self, workspace_id)
    }
    fn shell_pid(&self, pane_id: &str) -> Option<u32> {
        HerdrHost::shell_pid(self, pane_id)
    }
    fn with_launch_bound_dyn(&self, bound: Duration) -> Host {
        Arc::new(self.clone().with_launch_bound(bound))
    }
    fn with_command_timeout_dyn(&self, timeout: Duration) -> Host {
        Arc::new(self.clone().with_command_timeout(timeout))
    }
}

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
    fn version(&self) -> Result<String, HerdrError> {
        (**self).version()
    }
    fn agents(&self) -> Result<Vec<AgentInfo>, HerdrError> {
        (**self).agents()
    }
    fn agent(&self, target: &str) -> Result<Option<AgentInfo>, HerdrError> {
        (**self).agent(target)
    }
    fn launch(&self, req: &LaunchRequest) -> Result<Launched, HerdrError> {
        (**self).launch(req)
    }
    fn await_started(
        &self,
        name: &str,
        within: Duration,
    ) -> Result<Option<AgentInfo>, HerdrError> {
        (**self).await_started(name, within)
    }
    fn prompt(
        &self,
        target: &str,
        text: &str,
        wait: Option<Duration>,
    ) -> Result<PromptOutcome, HerdrError> {
        (**self).prompt(target, text, wait)
    }
    fn submit(&self, target: &str, text: &str) -> Result<PromptOutcome, HerdrError> {
        (**self).submit(target, text)
    }
    fn send_keys(&self, target: &str, keys: &[String]) -> Result<(), HerdrError> {
        (**self).send_keys(target, keys)
    }
    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), HerdrError> {
        (**self).send_text(pane_id, text)
    }
    fn read(&self, target: &str, lines: usize) -> Result<String, HerdrError> {
        (**self).read(target, lines)
    }
    fn read_sourced(
        &self,
        target: &str,
        lines: usize,
    ) -> Result<(String, &'static str), HerdrError> {
        (**self).read_sourced(target, lines)
    }
    fn wait(
        &self,
        target: &str,
        until: &[AgentStatus],
        timeout: Duration,
    ) -> Result<AgentInfo, HerdrError> {
        (**self).wait(target, until, timeout)
    }
    fn close_workspace(&self, workspace_id: &str) -> Result<(), HerdrError> {
        (**self).close_workspace(workspace_id)
    }
    fn shell_pid(&self, pane_id: &str) -> Option<u32> {
        (**self).shell_pid(pane_id)
    }
    fn with_launch_bound_dyn(&self, bound: Duration) -> Host {
        (**self).with_launch_bound_dyn(bound)
    }
    fn with_command_timeout_dyn(&self, timeout: Duration) -> Host {
        (**self).with_command_timeout_dyn(timeout)
    }
}
