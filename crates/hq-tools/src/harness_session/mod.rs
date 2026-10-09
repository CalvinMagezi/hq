//! Harness-agnostic session manager: spawn, monitor, steer, stop, and resume
//! long-lived external agent CLIs (claude-code, cursor, opencode, pi, kimi,
//! codex, qwen, antigravity, github-copilot) inside the host, on this machine or
//! on a remote host, tracked in the `harness_sessions` table.
//!
//! These tools only mutate sessions HQ launched. Agents a person started by
//! hand are visible through `host_agents`/`host_read` and steerable only
//! through `host_send`.

pub mod dismiss;
mod coalesce;
pub mod handoff;
pub mod mission;
mod preflight;
pub mod spec;
pub mod tools;
mod control;
mod cwd;
mod launch;
mod origin;

use crate::agent_host::{
    self, AgentInfo, AgentStatus, AgentHostError, Host, HostBackend, LaunchRequest, Launched, PromptOutcome,
};
use anyhow::{Result, bail};
use hq_db::Database;
use hq_db::harness_sessions_registry::{self as registry, HarnessSessionRow, Placement};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use spec::SPECS;
pub use launch::resume_awaiting;
pub use spec::{Harness, HarnessSessionSpec, ResumeStrategy, known_harnesses, resolve, resolve_in, spec_for};

/// Under the home of the host that runs pi, inside the state directory its sandbox may write.
const PI_SESSION_DIR: &str = ".pi/hq-sessions";

/// How long to wait for an agent to reach its prompt after a trust dialog was
/// accepted for it.
const TRUST_SETTLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Screen lines shown to the caller when a launch stops at a dialog.
const BLOCKED_SCREEN_LINES: usize = 40;

/// Longest a single `harness_session_wait` may block.
pub const MAX_WAIT: Duration = Duration::from_secs(300);

/// What one host said about its agents during a sweep: the agents, or why it
/// could not be asked.
pub type HostPoll = HashMap<String, std::result::Result<Vec<AgentInfo>, String>>;

/// Where a stored session stands right now.
#[derive(Debug, Clone, PartialEq)]
pub enum Liveness {
    Alive(Box<AgentInfo>),
    /// The host answered and the agent is not there.
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

pub fn liveness(polled: &HostPoll, row: &HarnessSessionRow) -> Liveness {
    match polled.get(&row.host) {
        Some(Ok(agents)) => agents
            .iter()
            .find(|a| a.name.as_deref() == Some(row.agent_name.as_str()))
            .map_or(Liveness::Gone, |a| Liveness::Alive(Box::new(a.clone()))),
        Some(Err(detail)) => Liveness::HostUnreachable(detail.clone()),
        None => Liveness::HostUnreachable(format!("host '{}' was not polled", row.host)),
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

pub use control::*;
pub use cwd::*;
pub use launch::*;
pub use origin::*;

#[cfg(test)]
mod tests;
