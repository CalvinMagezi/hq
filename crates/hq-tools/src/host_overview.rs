//! `host_overview`: every configured host, its agents and its lane allowances in one reply.
//!
//! Each host is checked on its own thread, so a laptop that is asleep cannot hold up the VPS.
//! A host reports back in stages (version, agents, quota) and the collector keeps whatever
//! arrived before the deadline: a late quota read does not erase a version that answered. A host
//! that says nothing in time is `unknown`, never `reachable`. Nothing here reads the filesystem
//! or a credential, and reachability is never taken from config.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use hq_core::config::HqConfig;
use hq_core::quota::{Confidence, QuotaSource};
use hq_db::Database;
use serde::Serialize;
use serde_json::{Value, json};

use crate::agent_host::tools::{agent_rows, managed_names};
use crate::agent_host::{AgentHostError, AgentInfo, Host, all_hosts, blocking};
use crate::harness_session as harness;
use crate::lane_quotas::{self, LaneView, Options};
use crate::registry::HqTool;
use crate::util::arg_str;
use anyhow::{Result, bail};
use async_trait::async_trait;
use std::sync::Arc;

/// Longest one host gets for all three checks. Native calls are bounded at 15s (quota 25s), so
/// this lets one slow step finish without letting a dead host stall the answer.
pub const HOST_TIMEOUT: Duration = Duration::from_secs(30);
/// Ceiling for the whole call, whatever the per-host bound is.
pub const OVERALL_BOUND: Duration = Duration::from_secs(45);

const TIMED_OUT: &str = "no answer from the host within the time limit";
const MISSING_STAGE: &str = "the host did not finish this check in time";

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub per_host: Duration,
    pub overall: Duration,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            per_host: HOST_TIMEOUT,
            overall: OVERALL_BOUND,
        }
    }
}

impl Bounds {
    /// The per-host limit can never stretch the call past the overall bound.
    fn effective(&self) -> Duration {
        self.per_host.min(self.overall)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reachability {
    Reachable,
    Unreachable,
    Unknown,
}

/// Why a reading is, or is not, a balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaStatus {
    Available,
    NotExposed,
    CheckFailed,
    Stale,
}

/// What one host reported, stage by stage, as it finished.
enum Stage {
    Version(Result<String, AgentHostError>, DateTime<Utc>),
    Agents(Result<Vec<Value>, AgentHostError>),
    Harnesses(Vec<Value>),
    Quota(QuotaOutcome),
}

struct QuotaOutcome {
    /// The host answered the quota question; `Some` when it did not and stored readings stand in.
    host_error: Option<String>,
    lanes: Vec<LaneView>,
    enabled: bool,
    /// The reading could not be taken at all (unknown lane name, store failure).
    failed: Option<String>,
}

/// What one overview call needs besides the hosts.
pub struct Inputs {
    pub db: std::sync::Arc<Database>,
    pub cfg: HqConfig,
    pub refresh: bool,
    pub bounds: Bounds,
}

/// Checks every host in parallel and returns one row each, in host order. Blocks up to the
/// overall bound: call it from a blocking thread.
pub fn overview(hosts: Vec<Host>, inputs: &Inputs) -> Value {
    let started = Instant::now();
    let managed = managed_names(&inputs.db).unwrap_or_default();
    let pending: Vec<_> = hosts
        .into_iter()
        .map(|h| {
            let (tx, rx) = mpsc::channel();
            let name = h.name().to_string();
            let worker = Worker {
                host: h,
                tx,
                db: inputs.db.clone(),
                cfg: inputs.cfg.clone(),
                refresh: inputs.refresh,
                managed: managed.clone(),
            };
            // Detached on purpose: a thread stuck on a dead ssh must not hold the reply, and it
            // ends by itself when the transport's own deadline fires.
            std::thread::spawn(move || worker.run());
            (name, rx)
        })
        .collect();
    let deadline = started + inputs.bounds.effective();
    let max_age = chrono::Duration::minutes(inputs.cfg.quota.interval_minutes.max(1) as i64);
    let rows: Vec<Value> = pending
        .into_iter()
        .map(|(name, rx)| collect(&name, rx, deadline, max_age))
        .collect();
    json!({
        "generated_at": Utc::now(),
        "mode": if inputs.refresh { "refresh" } else { "cached" },
        "hosts": rows,
    })
}

struct Worker {
    host: Host,
    tx: Sender<Stage>,
    db: std::sync::Arc<Database>,
    cfg: HqConfig,
    refresh: bool,
    managed: Vec<(String, String, String)>,
}

impl Worker {
    fn run(self) {
        let checked_at = Utc::now();
        let version = self.host.version();
        let up = version.is_ok();
        let _ = self.tx.send(Stage::Version(version, checked_at));
        if !up {
            // Stored quota readings are still worth showing, marked stale, for a host that is down.
            let _ = self.tx.send(Stage::Quota(self.quota()));
            return;
        }
        let name = self.host.name().to_string();
        let agents = self
            .host
            .agents()
            .map(|a: Vec<AgentInfo>| agent_rows(&name, &a, &self.managed));
        let _ = self.tx.send(Stage::Agents(agents));
        let _ = self.tx.send(Stage::Harnesses(harness_rows(&self.host)));
        let _ = self.tx.send(Stage::Quota(self.quota()));
    }

    fn quota(&self) -> QuotaOutcome {
        let opts = Options {
            refresh: self.refresh,
            host: Some(self.host.name().to_string()),
            lane: None,
        };
        let hosts = std::slice::from_ref(&self.host);
        match lane_quotas::read_hosts(&self.db, &self.cfg, hosts, &opts, Utc::now()) {
            Ok(report) => QuotaOutcome {
                host_error: report.hosts.first().and_then(|h| h.error.clone()),
                lanes: report.lanes,
                enabled: report.enabled,
                failed: None,
            },
            Err(e) => QuotaOutcome {
                host_error: None,
                lanes: Vec::new(),
                enabled: true,
                failed: Some(e.to_string()),
            },
        }
    }
}

/// Which harnesses the host can start. Only a host that answers the question gets `true` or
/// `false`; a profile that runs through a shell or a full path cannot be looked for, so it stays
/// `unknown` rather than being called installed.
fn harness_rows(host: &Host) -> Vec<Value> {
    let known = harness::known_harnesses();
    let program = |name: &str| {
        harness::spec::resolve(name)
            .ok()
            .and_then(|r| harness::plain_binary(&r))
            .filter(|p| !p.starts_with('/'))
    };
    let mut programs: Vec<String> = known.iter().filter_map(|n| program(n)).collect();
    programs.sort();
    programs.dedup();
    let found = host.installed_programs(&programs);
    known
        .iter()
        .map(
            |name| json!({ "name": name, "installed": installed(found.as_deref(), program(name)) }),
        )
        .collect()
}

fn installed(found: Option<&[String]>, program: Option<String>) -> Value {
    match (found, program) {
        (Some(found), Some(p)) => json!(found.contains(&p)),
        _ => json!("unknown"),
    }
}

/// What has arrived for one host so far.
#[derive(Default)]
struct Gathered {
    version: Option<(Result<String, AgentHostError>, DateTime<Utc>)>,
    agents: Option<Result<Vec<Value>, AgentHostError>>,
    harnesses: Option<Vec<Value>>,
    quota: Option<QuotaOutcome>,
}

impl Gathered {
    fn take(&mut self, stage: Stage) {
        match stage {
            Stage::Version(v, at) => self.version = Some((v, at)),
            Stage::Agents(a) => self.agents = Some(a),
            Stage::Harnesses(h) => self.harnesses = Some(h),
            Stage::Quota(q) => self.quota = Some(q),
        }
    }

    /// Quota is the last stage a worker sends, so having it means the host has said everything.
    fn complete(&self) -> bool {
        self.version.is_some() && self.quota.is_some()
    }
}

fn collect(
    name: &str,
    rx: mpsc::Receiver<Stage>,
    deadline: Instant,
    max_age: chrono::Duration,
) -> Value {
    let mut got = Gathered::default();
    while !got.complete() {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(stage) => got.take(stage),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    host_row(name, got, max_age)
}

fn host_row(name: &str, got: Gathered, max_age: chrono::Duration) -> Value {
    let (reachability, version, reason, checked_at) = match &got.version {
        Some((Ok(v), at)) => (Reachability::Reachable, Some(v.clone()), None, *at),
        Some((Err(e @ AgentHostError::Unreachable { .. }), at)) => {
            (Reachability::Unreachable, None, Some(short(e)), *at)
        }
        // The host answered with an error: reachable enough to refuse, not enough to trust.
        Some((Err(e), at)) => (Reachability::Unknown, None, Some(short(e)), *at),
        None => (
            Reachability::Unknown,
            None,
            Some(TIMED_OUT.to_string()),
            Utc::now(),
        ),
    };
    let up = reachability == Reachability::Reachable;
    let (agents, agents_error) = agents_part(up, got.agents);
    json!({
        "host": name,
        "reachability": reachability,
        "checked_at": checked_at,
        "version": version,
        "reason": reason,
        "agents": agents,
        "agents_error": agents_error,
        "harnesses": if up { json!(got.harnesses) } else { Value::Null },
        "quota": quota_part(got.quota, max_age),
    })
}

/// Agents are listed only for a host that answered; a failure of just this step is said, not
/// hidden behind an empty list.
fn agents_part(up: bool, agents: Option<Result<Vec<Value>, AgentHostError>>) -> (Value, Value) {
    if !up {
        return (Value::Null, Value::Null);
    }
    match agents {
        Some(Ok(rows)) => (json!(rows), Value::Null),
        Some(Err(e)) => (Value::Null, json!(short(&e))),
        None => (Value::Null, json!(MISSING_STAGE)),
    }
}

fn quota_part(quota: Option<QuotaOutcome>, max_age: chrono::Duration) -> Value {
    let Some(q) = quota else {
        return json!({ "status": QuotaStatus::CheckFailed, "reason": MISSING_STAGE, "rows": [] });
    };
    if let Some(why) = &q.failed {
        return json!({ "status": QuotaStatus::CheckFailed, "reason": why, "rows": [] });
    }
    if !q.enabled {
        return json!({ "enabled": false, "rows": [] });
    }
    let rows: Vec<Value> = q
        .lanes
        .iter()
        .flat_map(|l| lane_rows(l, q.host_error.as_deref(), max_age))
        .collect();
    json!({ "enabled": true, "host_error": q.host_error, "rows": rows })
}

/// One row per window; a lane with no window is one row that says why.
fn lane_rows(l: &LaneView, host_error: Option<&str>, max_age: chrono::Duration) -> Vec<Value> {
    let status = quota_status(l, host_error.is_some(), max_age);
    let base = |metric: Value, units: Value, value: Value, remaining: Value, resets: Value| {
        json!({
            "lane": l.lane,
            "account": l.account,
            "status": status,
            "metric": metric,
            "units": units,
            "used": value,
            "remaining": remaining,
            "resets_at": resets,
            "observed_at": l.observed_at,
            "age_seconds": l.age_seconds,
            "source": l.source,
            "confidence": l.confidence,
            "reason": l.reason,
        })
    };
    if l.windows.is_empty() {
        return vec![base(
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
        )];
    }
    l.windows
        .iter()
        .map(|w| {
            let metric = json!(w.window.kind);
            let units = w.window.used_percent.map(|_| "percent_used");
            let used = if w.expired {
                None
            } else {
                w.window.used_percent
            };
            base(
                metric,
                json!(units),
                json!(used),
                json!(w.window.remaining),
                json!(w.window.resets_at),
            )
        })
        .collect()
}

/// Live only when it was taken recently enough; anything older is stale whatever the source says.
pub fn quota_status(l: &LaneView, host_failed: bool, max_age: chrono::Duration) -> QuotaStatus {
    let too_old = l.age_seconds > max_age.num_seconds();
    match l.confidence {
        Confidence::Stale => QuotaStatus::Stale,
        Confidence::Live if too_old || host_failed => QuotaStatus::Stale,
        Confidence::Live => QuotaStatus::Available,
        // The ledger source means the lane has no balance API at all, which is not a failure.
        Confidence::Unknown if l.source == QuotaSource::Ledger => QuotaStatus::NotExposed,
        Confidence::Unknown => QuotaStatus::CheckFailed,
    }
}

/// The error without the host's own wrapping noise, short enough for a table cell.
fn short(e: &AgentHostError) -> String {
    const MAX: usize = 200;
    let text = match e {
        AgentHostError::Unreachable { detail, .. } => detail.clone(),
        AgentHostError::Api { code, message } => format!("{code}: {message}"),
    };
    text.chars().take(MAX).collect()
}

/// Read-only snapshot of every host. It asks the hosts; it never scans a filesystem.
pub struct HostOverviewTool {
    db: std::sync::Arc<Database>,
}

impl HostOverviewTool {
    pub fn new(db: std::sync::Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl HqTool for HostOverviewTool {
    fn name(&self) -> &str {
        "host_overview"
    }
    fn description(&self) -> &str {
        "One read-only snapshot of every configured host: whether it answers right now (reachable, unreachable, or unknown when it did not answer in time, never assumed from config), when that was checked, its version, the coding agents it runs, which agent programs it can start where it can tell (otherwise unknown), and each lane's allowance with status (available, not_exposed, check_failed, stale), metric, units, source and observed_at. Reachable means the host answered; it does not mean an agent is signed in or can start a session. A configured limit is not a remaining balance, and an old reading is stale, never live. One host failing keeps the others. It asks the hosts only: it does not scan any filesystem and reads no credentials. `mode` is cached (default: readings within the sampling interval, with their age) or refresh (ask now); both obey the time limits."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": { "type": "string", "enum": ["cached", "refresh"], "default": "cached", "description": "cached reuses quota readings taken within the sampling interval; refresh asks the hosts now" }
            }
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let refresh = match arg_str(&args, "mode").as_str() {
            "" | "cached" => false,
            "refresh" => true,
            other => bail!("mode must be 'cached' or 'refresh', not '{other}'"),
        };
        let inputs = Inputs {
            db: self.db.clone(),
            cfg: HqConfig::load()?,
            refresh,
            bounds: Bounds::default(),
        };
        let hosts = all_hosts()?;
        blocking(move || overview(hosts, &inputs)).await
    }
}

#[cfg(test)]
mod tests;

/// The host tools together with the overview, lane quota and helper setup tools. The overview
/// reads lane quotas and harness specs, so it lives above `agent_host` and callers use this
/// function instead of `agent_host::tools::create_host_tools` alone.
pub fn create_host_tools_with_extras(db: Arc<Database>) -> Vec<Box<dyn HqTool>> {
    let mut tools = crate::agent_host::tools::create_host_tools(db.clone());
    tools.push(Box::new(HostOverviewTool::new(db.clone())));
    tools.push(Box::new(crate::lane_quotas::LaneQuotasTool::new(
        db.clone(),
    )));
    tools.push(Box::new(crate::helper_setup::HelperSetupTool::new(db)));
    tools
}
