//! Lane quota readers. The host reads each agent's own login on this machine, so a credential
//! never crosses the gate and never reaches a model. Every reader fails soft: an error becomes a
//! `confidence: unknown` reading with a short reason, and every network or process call has a
//! timeout. Unknown is never reported as zero.

mod antigravity;
mod claude;
mod codex;
mod copilot;
mod cursor;
mod custom;
mod ledger;
mod run;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use hq_core::quota::{LaneQuota, QuotaSource};
use serde::Deserialize;

pub(crate) use run::{http_get_json, run_capture};

/// Total time one `host.quota` call may take, readers running side by side.
const TOTAL_DEADLINE: Duration = Duration::from_secs(15);
/// A reading is reused this long so repeated asks do not hammer vendor endpoints.
const CACHE_TTL: Duration = Duration::from_secs(60);
/// Longest wait for a short local command (a keychain read, a version probe).
pub(crate) const LOCAL_CMD_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest wait for one vendor HTTP call.
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
/// Most output any reader keeps from a process.
pub(crate) const OUTPUT_CAP_BYTES: usize = 64 * 1024;
const PERCENT_MAX: f64 = 100.0;

type Readings = Vec<LaneQuota>;
type Job = Box<dyn FnOnce() -> Readings + Send>;
type Cache = Mutex<HashMap<String, (Instant, Readings)>>;

/// The agent kinds with a reader, in the order they are reported.
const KINDS: &[&str] = &[
    "claude", "codex", "copilot", "agy", "cursor", "opencode", "pi", "kimi", "qwen", "hermes",
];

/// What `host.quota` takes. All optional.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Request {
    /// Read this one lane only.
    #[serde(default)]
    pub lane: Option<String>,
    /// Agent kinds the owner turned off (`quota.lanes`).
    #[serde(default)]
    pub disabled: Vec<String>,
    /// Harness profiles with their own reader command (`quota:` on a profile).
    #[serde(default)]
    pub custom: Vec<custom::Spec>,
}

/// Clamps a percentage into 0..=100; `None` for a value that is not a number.
pub(crate) fn clamp_percent(v: f64) -> Option<f64> {
    v.is_finite().then(|| v.clamp(0.0, PERCENT_MAX))
}

/// The programs whose presence means the lane is installed here.
fn binaries(kind: &str) -> Vec<String> {
    match kind {
        "cursor" => vec!["cursor-agent".into()],
        "copilot" => vec!["copilot".into(), "gh".into()],
        other => vec![other.to_string()],
    }
}

fn installed(kind: &str) -> bool {
    crate::programs::installed(&binaries(kind)).is_some_and(|found| !found.is_empty())
}

fn read_kind(kind: &str) -> Vec<LaneQuota> {
    match kind {
        "claude" => vec![claude::read()],
        "codex" => vec![codex::read()],
        "copilot" => vec![copilot::read()],
        "agy" => vec![antigravity::read()],
        "cursor" => vec![cursor::read()],
        other => vec![ledger::read(other)],
    }
}

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn cached(lane: &str) -> Option<Vec<LaneQuota>> {
    let map = cache().lock().ok()?;
    let (at, readings) = map.get(lane)?;
    (at.elapsed() < CACHE_TTL).then(|| readings.clone())
}

fn remember(lane: &str, readings: &[LaneQuota]) {
    if let Ok(mut map) = cache().lock() {
        map.insert(lane.to_string(), (Instant::now(), readings.to_vec()));
    }
}

/// Runs the enabled readers whose agent is installed, concurrently, within the total deadline.
pub(crate) fn collect(req: &Request) -> Vec<LaneQuota> {
    let wanted = |lane: &str| req.lane.as_deref().is_none_or(|l| l == lane);
    let mut jobs: Vec<(String, Job)> = Vec::new();
    for kind in KINDS {
        let off = req.disabled.iter().any(|d| d == kind);
        if off || !wanted(kind) || !installed(kind) {
            continue;
        }
        let kind = kind.to_string();
        jobs.push((kind.clone(), Box::new(move || read_kind(&kind))));
    }
    for spec in &req.custom {
        if !wanted(&spec.lane) || req.disabled.contains(&spec.lane) {
            continue;
        }
        let spec = spec.clone();
        jobs.push((spec.lane.clone(), Box::new(move || vec![spec.read()])));
    }

    let (tx, rx) = mpsc::channel();
    let mut results: Vec<(usize, Vec<LaneQuota>)> = Vec::new();
    let mut pending = Vec::new();
    for (index, (lane, job)) in jobs.into_iter().enumerate() {
        if let Some(hit) = cached(&lane) {
            results.push((index, hit));
            continue;
        }
        pending.push((index, lane.clone()));
        let tx = tx.clone();
        std::thread::spawn(move || {
            let readings = job();
            remember(&lane, &readings);
            let _ = tx.send((index, readings));
        });
    }
    drop(tx);

    let deadline = Instant::now() + TOTAL_DEADLINE;
    while pending_left(&pending, &results) > 0 {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(done) => results.push(done),
            Err(_) => break,
        }
    }
    for (index, lane) in &pending {
        if !results.iter().any(|(i, _)| i == index) {
            results.push((
                *index,
                vec![LaneQuota::unknown(
                    lane,
                    QuotaSource::Cli,
                    "the reading did not finish in time",
                )],
            ));
        }
    }
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().flat_map(|(_, r)| r).collect()
}

fn pending_left(pending: &[(usize, String)], done: &[(usize, Vec<LaneQuota>)]) -> usize {
    pending
        .iter()
        .filter(|(i, _)| !done.iter().any(|(d, _)| d == i))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_is_clamped_and_nan_is_not_a_number() {
        assert_eq!(clamp_percent(140.0), Some(100.0));
        assert_eq!(clamp_percent(-3.0), Some(0.0));
        assert_eq!(clamp_percent(f64::NAN), None);
    }

    #[test]
    fn a_turned_off_lane_is_not_read() {
        let req = Request {
            lane: None,
            disabled: KINDS.iter().map(|k| k.to_string()).collect(),
            custom: Vec::new(),
        };
        assert!(collect(&req).is_empty());
    }
}
