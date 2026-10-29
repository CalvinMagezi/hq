//! Harness-agnostic session manager: spawn, monitor, steer, stop, and resume
//! long-lived external agent CLIs inside the host, on this machine or on a
//! remote host, tracked in the `harness_sessions` table. Two categories run
//! there (`category.rs`): coding agents (claude-code, cursor, opencode, pi,
//! kimi, codex, qwen, antigravity, github-copilot) and general-purpose agents
//! (hermes, openclaw), which never take code work.
//!
//! These tools only mutate sessions HQ launched. Agents a person started by
//! hand are visible through `host_agents`/`host_read` and steerable only
//! through `host_send`.

pub mod category;
mod coalesce;
mod connector_notice;
mod control;
mod cwd;
pub mod dismiss;
mod folder_request;
pub mod handoff;
mod launch;
pub mod mission;
mod origin;
mod preflight;
pub mod spec;
pub mod stack;
pub mod tools;

use crate::agent_host::{
    self, AgentHostError, AgentInfo, AgentStatus, Host, HostBackend, LaunchRequest, Launched,
    PromptOutcome,
};
use anyhow::{Result, bail};
use hq_db::Database;
use hq_db::harness_sessions_registry::{self as registry, HarnessSessionRow, Placement};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub use category::{HarnessCategory, Purpose};
pub use launch::resume_awaiting;
use spec::SPECS;
pub use spec::{
    Harness, HarnessSessionSpec, ResumeStrategy, agent_host_config, category_in, category_of,
    known_harnesses, known_in, names_in, resolve, resolve_in, spec_for,
};

/// Under the home of the host that runs pi, inside the state directory its sandbox may write.
const PI_SESSION_DIR: &str = ".pi/hq-sessions";

/// How long to wait for an agent to reach its prompt after a trust dialog was
/// accepted for it.
const TRUST_SETTLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Screen lines shown to the caller when a launch stops at a dialog.
const BLOCKED_SCREEN_LINES: usize = 40;

/// Longest a single `harness_session_wait` may block. An MCP client's call gives
/// up near 60 seconds, and a wait it abandoned tells it nothing, so a longer
/// wait is a loop of short ones.
pub const MAX_WAIT: Duration = Duration::from_secs(45);
/// What `harness_session_wait` waits when the caller names no timeout.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(40);
const _: () = assert!(MAX_WAIT.as_secs() < 60 && DEFAULT_WAIT.as_secs() <= MAX_WAIT.as_secs());

/// What one host said about its agents during a sweep: the agents, or why it
/// could not be asked.
pub type HostPoll = HashMap<String, std::result::Result<Vec<AgentInfo>, String>>;

/// Where a stored session stands right now.
#[derive(Debug, Clone, PartialEq)]
pub enum Liveness {
    Alive(Box<AgentInfo>),
    /// The host answered and the agent is not there, or its process has ended.
    Gone,
    /// The host could not be asked; nothing is known about the agent.
    HostUnreachable(String),
}

use agent_host::blocking;

/// Ask each host named by `rows` for its agents, once per host.
pub fn poll_hosts(rows: &[HarnessSessionRow]) -> HostPoll {
    poll_hosts_with(rows, |name| agent_host::host(Some(name)))
}

pub fn poll_hosts_with(
    rows: &[HarnessSessionRow],
    resolve: impl Fn(&str) -> anyhow::Result<Host>,
) -> HostPoll {
    let mut polled = HostPoll::new();
    for row in rows {
        polled.entry(row.host.clone()).or_insert_with(|| {
            resolve(&row.host)
                .map_err(|e| e.to_string())
                .and_then(|h| h.agents().map_err(|e| e.to_string()))
        });
    }
    polled
}

/// An agent whose process ended counts as gone even while the host still lists
/// its pane: a one-shot run ends that way, and nothing is left to drive.
pub fn liveness(polled: &HostPoll, row: &HarnessSessionRow) -> Liveness {
    match polled.get(&row.host) {
        Some(Ok(agents)) => agents
            .iter()
            .find(|a| a.name.as_deref() == Some(row.agent_name.as_str()))
            .filter(|a| !a.exited)
            .map_or(Liveness::Gone, |a| Liveness::Alive(Box::new(a.clone()))),
        Some(Err(detail)) => Liveness::HostUnreachable(detail.clone()),
        None => Liveness::HostUnreachable(format!("host '{}' was not polled", row.host)),
    }
}

/// The pane of a session whose process has ended but which the host still
/// lists: its screen can still be read once, and the pane should be closed.
pub fn exited_pane(polled: &HostPoll, row: &HarnessSessionRow) -> Option<AgentInfo> {
    match polled.get(&row.host) {
        Some(Ok(agents)) => agents
            .iter()
            .find(|a| a.name.as_deref() == Some(row.agent_name.as_str()) && a.exited)
            .cloned(),
        _ => None,
    }
}

/// host agent names are `[a-z][a-z0-9_-]{0,31}`; the prefix and the ten-digit
/// suffix leave this much room for the harness name.
const MAX_HARNESS_IN_ID: usize = 18;

fn new_session_id(harness: &str) -> String {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let slug: String = harness
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9' | '-') => c,
            _ => '-',
        })
        .take(MAX_HARNESS_IN_ID)
        .collect();
    format!("hs-{slug}-{:x}", nanos as u64 & 0xff_ffff_ffff)
}

pub use connector_notice::*;
pub use control::*;
pub use cwd::*;
pub use launch::*;
pub use origin::*;

#[cfg(test)]
mod tests;
