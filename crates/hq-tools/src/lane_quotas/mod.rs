//! Each coding-agent lane's remaining allowance, asked of every host and kept.
//!
//! The login lives on the machine the agent runs on, so a host reads it and HQ asks. An answer is
//! stored; a host that cannot be reached, or a lane it cannot read, gives back the last stored
//! reading marked `stale` with its age. Unknown is never turned into zero. Owner-only data: it
//! is shown to the owner and to a session the owner drives, and goes nowhere else.

pub mod alerts;
pub mod gate;
pub mod pace;

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use hq_core::config::{HqConfig, QuotaConfig};
use hq_core::quota::{Confidence, LaneQuota, QuotaSource, QuotaWindow};
use hq_db::Database;
use hq_db::lane_quota_samples as store;
use serde::Serialize;
use serde_json::{Value, json};

use crate::agent_host::{self, Host, HostBackend, QuotaAsk, QuotaProfile, blocking};
use crate::registry::HqTool;
use crate::util::arg_str;
use pace::Pace;

const STALE_HOST_REASON: &str = "the host could not be read";
const STALE_LANE_REASON: &str = "the host could not read this lane now";

/// One window with its pace and the owner's thresholds applied.
#[derive(Debug, Clone, Serialize)]
pub struct WindowView {
    #[serde(flatten)]
    pub window: QuotaWindow,
    pub pace: Option<Pace>,
    /// Used percent is at or above `quota.warn_percent`.
    pub warn: bool,
    /// Used percent is at or above `quota.block_percent`.
    pub block: bool,
    /// The window reset after it was read, so its used percent no longer holds.
    pub expired: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaneView {
    pub host: String,
    pub lane: String,
    pub account: Option<String>,
    pub windows: Vec<WindowView>,
    pub source: QuotaSource,
    pub observed_at: DateTime<Utc>,
    pub confidence: Confidence,
    pub reason: Option<String>,
    /// Seconds since the reading was taken.
    pub age_seconds: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostView {
    pub host: String,
    pub reachable: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub enabled: bool,
    pub generated_at: DateTime<Utc>,
    pub warn_percent: f64,
    pub block_percent: f64,
    pub hosts: Vec<HostView>,
    pub lanes: Vec<LaneView>,
}

/// What a read is asked to do.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Ask the hosts even when a recent reading is stored.
    pub refresh: bool,
    pub host: Option<String>,
    pub lane: Option<String>,
}

/// The profile readers to send to a host: the `quota.command` entries of the harness profiles.
pub fn profiles(cfg: &HqConfig) -> Vec<QuotaProfile> {
    cfg.agent_host
        .harness_profiles
        .iter()
        .filter_map(|(name, p)| {
            let q = p.quota.as_ref()?;
            Some(QuotaProfile {
                lane: name.clone(),
                command: q.command.clone(),
                timeout_secs: q.timeout_secs,
            })
        })
        .collect()
}

/// A stored reading handed back in place of a live one.
fn as_stale(mut q: LaneQuota, why: &str) -> LaneQuota {
    q.confidence = Confidence::Stale;
    q.reason = Some(why.to_string());
    q
}

impl LaneView {
    /// A reading with each window's pace and flags worked out against the owner's thresholds.
    pub fn from_reading(
        host: &str,
        q: LaneQuota,
        now: DateTime<Utc>,
        cfg: &QuotaConfig,
    ) -> LaneView {
        let (warn, block) = (cfg.warn_percent, cfg.block_percent);
        let windows = q
            .windows
            .iter()
            .map(|w| {
                let expired = w.resets_at.is_some_and(|r| r <= now);
                let used = w.used_percent.filter(|_| !expired);
                WindowView {
                    window: w.clone(),
                    pace: pace::pace(w, q.observed_at, warn, block).filter(|_| !expired),
                    warn: used.is_some_and(|u| u >= warn),
                    block: used.is_some_and(|u| u >= block),
                    expired,
                }
            })
            .collect();
        LaneView {
            host: host.to_string(),
            lane: q.lane,
            account: q.account,
            windows,
            source: q.source,
            observed_at: q.observed_at,
            confidence: q.confidence,
            reason: q.reason,
            age_seconds: (now - q.observed_at).num_seconds().max(0),
        }
    }
}

/// What one host answered, after the stored readings were weighed against it.
struct HostOutcome {
    host: String,
    error: Option<String>,
    lanes: Vec<LaneQuota>,
}

fn settle(
    db: &Database,
    host: &str,
    asked: Result<Vec<LaneQuota>, agent_host::AgentHostError>,
    stored: &[store::Sample],
    lane: Option<&str>,
) -> HostOutcome {
    let mine = |s: &&store::Sample| {
        s.host == host && lane.is_none_or(|l| l == s.quota.lane) && !is_unknown(&s.quota)
    };
    let last = |name: &str| {
        stored
            .iter()
            .filter(mine)
            .find(|s| s.quota.lane == name)
            .map(|s| s.quota.clone())
    };
    match asked {
        Err(e) => {
            let mut lanes: Vec<LaneQuota> = stored
                .iter()
                .filter(mine)
                .map(|s| as_stale(s.quota.clone(), &format!("{STALE_HOST_REASON}: {e}")))
                .collect();
            if let (true, Some(name)) = (lanes.is_empty(), lane) {
                lanes.push(LaneQuota::unknown(
                    name,
                    QuotaSource::Cli,
                    format!("{STALE_HOST_REASON}: {e}"),
                ));
            }
            HostOutcome {
                host: host.to_string(),
                error: Some(e.to_string()),
                lanes,
            }
        }
        Ok(readings) => {
            let lanes = readings
                .into_iter()
                .map(|q| {
                    if !is_unknown(&q) {
                        // A failed write loses a sample, not the reading in hand.
                        let _ = db.with_conn(|c| store::insert(c, host, &q));
                        return q;
                    }
                    match last(&q.lane) {
                        Some(prior) => {
                            let why = q.reason.as_deref().unwrap_or(STALE_LANE_REASON);
                            as_stale(prior, &format!("{STALE_LANE_REASON}: {why}"))
                        }
                        None => q,
                    }
                })
                .collect();
            HostOutcome {
                host: host.to_string(),
                error: None,
                lanes,
            }
        }
    }
}

fn is_unknown(q: &LaneQuota) -> bool {
    q.confidence == Confidence::Unknown
}

/// Whether every host named already has a stored reading newer than `max_age`.
fn fresh_enough(
    stored: &[store::Sample],
    hosts: &[Host],
    max_age: Duration,
    now: DateTime<Utc>,
) -> bool {
    hosts.iter().all(|h| {
        stored
            .iter()
            .any(|s| s.host == h.name() && now - s.quota.observed_at < max_age)
    })
}

/// Reads every host (or the named one), stores what came back and returns the readings, with
/// stored ones in place of what could not be read. Blocks on the hosts: call it from a blocking
/// thread. `hosts` are all the configured hosts, reachable or not.
pub fn read_hosts(
    db: &Database,
    cfg: &HqConfig,
    hosts: &[Host],
    opts: &Options,
    now: DateTime<Utc>,
) -> Result<Report> {
    let q = &cfg.quota;
    let mut report = Report {
        enabled: q.enabled,
        generated_at: now,
        warn_percent: q.warn_percent,
        block_percent: q.block_percent,
        hosts: Vec::new(),
        lanes: Vec::new(),
    };
    if !q.enabled {
        return Ok(report);
    }
    let chosen: Vec<&Host> = hosts
        .iter()
        .filter(|h| opts.host.as_deref().is_none_or(|n| n == h.name()))
        .collect();
    if chosen.is_empty() {
        let known: Vec<&str> = hosts.iter().map(|h| h.name()).collect();
        bail!(
            "no host named '{}' (known: {})",
            opts.host.as_deref().unwrap_or_default(),
            known.join(", ")
        );
    }
    let stored = db.with_conn(store::latest_per_lane).unwrap_or_default();
    let interval = Duration::minutes(q.interval_minutes.max(1) as i64);
    let owned: Vec<Host> = chosen.iter().map(|h| (*h).clone()).collect();
    let lane = opts.lane.as_deref();
    let outcomes: Vec<HostOutcome> =
        if !opts.refresh && fresh_enough(&stored, &owned, interval, now) {
            owned
                .iter()
                .map(|h| HostOutcome {
                    host: h.name().to_string(),
                    error: None,
                    lanes: stored
                        .iter()
                        .filter(|s| s.host == h.name() && lane.is_none_or(|l| l == s.quota.lane))
                        .map(|s| s.quota.clone())
                        .collect(),
                })
                .collect()
        } else {
            let ask = QuotaAsk {
                lane: opts.lane.clone(),
                disabled: q.disabled_lanes(),
                custom: profiles(cfg),
            };
            std::thread::scope(|scope| {
                let workers: Vec<_> = owned
                    .iter()
                    .map(|h| {
                        let (ask, stored) = (&ask, &stored);
                        (
                            h.name().to_string(),
                            scope.spawn(move || settle(db, h.name(), h.quota(ask), stored, lane)),
                        )
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|(name, w)| {
                        w.join().unwrap_or_else(|_| HostOutcome {
                            host: name,
                            error: Some("the quota call panicked".into()),
                            lanes: Vec::new(),
                        })
                    })
                    .collect()
            })
        };
    for o in outcomes {
        report.hosts.push(HostView {
            host: o.host.clone(),
            reachable: o.error.is_none(),
            error: o.error,
        });
        let enabled = |l: &LaneQuota| q.lane_enabled(&l.lane);
        for l in o.lanes.into_iter().filter(enabled) {
            report
                .lanes
                .push(LaneView::from_reading(&o.host, l, now, q));
        }
    }
    Ok(report)
}

/// `read_hosts` against the configured hosts. Blocks.
pub fn read_all(db: &Database, cfg: &HqConfig, opts: &Options) -> Result<Report> {
    let hosts = agent_host::all_hosts()?;
    read_hosts(db, cfg, &hosts, opts, Utc::now())
}

/// Drops readings past retention. Called after each background sample.
pub fn prune(db: &Database, now: DateTime<Utc>) {
    let _ = db.with_conn(|c| store::prune(c, now));
}

/// Lanes whose reading is unknown, with the reason, for `hq doctor`.
pub fn unknown_lanes(report: &Report) -> BTreeMap<String, String> {
    report
        .lanes
        .iter()
        .filter(|l| l.confidence == Confidence::Unknown)
        .map(|l| {
            let why = l.reason.clone().unwrap_or_else(|| "no reason given".into());
            (format!("{}/{}", l.host, l.lane), why)
        })
        .collect()
}

pub struct LaneQuotasTool {
    db: std::sync::Arc<Database>,
}

impl LaneQuotasTool {
    pub fn new(db: std::sync::Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl HqTool for LaneQuotasTool {
    fn name(&self) -> &str {
        "lane_quotas"
    }
    fn description(&self) -> &str {
        "How much of each coding-agent lane's allowance is used (Claude, Codex, Copilot, agy, Cursor and any private gateway), per host: each window's used percent, when it resets, and whether use is on pace to pass the warn or block line by the reset. A reading from a host that cannot be reached comes back stale with its age; a lane that cannot be read is unknown, never zero. Read-only."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "host": { "type": "string", "description": "One host from host_list. Omit for all." },
                "lane": { "type": "string", "description": "One lane (claude, codex, copilot, agy, cursor, or a profile name). Omit for all." },
                "refresh": { "type": "boolean", "description": "Ask the hosts now instead of using a reading taken within the sampling interval." }
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
        let named = |k: &str| Some(arg_str(&args, k)).filter(|s| !s.is_empty());
        let opts = Options {
            refresh: args
                .get("refresh")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            host: named("host"),
            lane: named("lane"),
        };
        let db = self.db.clone();
        let cfg = HqConfig::load()?;
        let report = blocking(move || read_all(&db, &cfg, &opts)).await??;
        Ok(serde_json::to_value(report)?)
    }
}

#[cfg(test)]
mod tests;
