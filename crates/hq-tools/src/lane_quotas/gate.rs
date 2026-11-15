//! Whether a lane may start new work, from its stored reading. Pure: no clock, no I/O, no host.
//!
//! The gate only ever reads what the sampler stored. It never asks a host or a vendor, so a
//! spawn does not wait on a login, and an unknown or missing reading never blocks anything.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use hq_core::config::{AgentHostConfig, QuotaConfig};
use hq_core::quota::{Confidence, QuotaWindow};
use hq_db::lane_quota_samples::Sample;
use serde_json::{Value, json};

/// A reading older than this many sampling intervals is too old to refuse a start on.
const STALE_AFTER_INTERVALS: i64 = 2;

/// The lane name the readers use for a built-in harness, or `None` when it has no reader.
fn lane_of_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "claude-code" | "claude" => Some("claude"),
        "github-copilot" | "copilot" => Some("copilot"),
        "antigravity" | "agy" => Some("agy"),
        "codex" => Some("codex"),
        "cursor" => Some("cursor"),
        _ => None,
    }
}

/// The quota lane a harness name or profile draws on. A profile with its own `quota.command`
/// is its own lane; any other profile shares its base's login. `None` means HQ has no reading
/// for it and never gates it.
pub fn lane_for(cfg: &AgentHostConfig, harness: &str) -> Option<String> {
    let profile = cfg.harness_profiles.get(harness).or_else(|| {
        cfg.harness_profiles
            .get(crate::harness_session::spec::canonical_name(harness))
    });
    match profile {
        Some(p) if p.quota.is_some() => Some(harness.to_string()),
        Some(p) => lane_of_kind(&p.base).map(str::to_string),
        None => lane_of_kind(harness).map(str::to_string),
    }
}

/// The worst window of a lane, as the gate saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub lane: String,
    pub window: String,
    pub used_percent: f64,
    pub resets_at: Option<DateTime<Utc>>,
    /// The reading is older than the gate trusts, so it warns and never blocks.
    pub stale: bool,
}

impl Notice {
    /// The text a fallback gives as its reason.
    pub fn reason(&self) -> String {
        let until = self
            .resets_at
            .map(|r| r.to_rfc3339_opts(SecondsFormat::Secs, true))
            .unwrap_or_else(|| "its next reset".to_string());
        format!(
            "quota: {} at {:.0}% until {until}",
            self.window, self.used_percent
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "lane": self.lane,
            "window": self.window,
            "used_percent": self.used_percent,
            "resets_at": self.resets_at,
            "stale": self.stale,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Clear,
    Warn(Notice),
    Block(Notice),
}

impl Verdict {
    pub fn is_block(&self) -> bool {
        matches!(self, Verdict::Block(_))
    }

    /// The notice a start on this lane carries, if it has one.
    pub fn notice(&self) -> Option<&Notice> {
        match self {
            Verdict::Clear => None,
            Verdict::Warn(n) | Verdict::Block(n) => Some(n),
        }
    }
}

/// The window with the highest used percent that has not reset yet.
fn worst_window(windows: &[QuotaWindow], now: DateTime<Utc>) -> Option<(&QuotaWindow, f64)> {
    windows
        .iter()
        .filter(|w| w.resets_at.is_none_or(|r| r > now))
        .filter_map(|w| Some((w, w.used_percent?)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// What the stored reading of `lane` on `host` says about starting work there.
pub fn check(
    cfg: &QuotaConfig,
    stored: &[Sample],
    host: &str,
    lane: &str,
    now: DateTime<Utc>,
) -> Verdict {
    if !cfg.lane_enabled(lane) {
        return Verdict::Clear;
    }
    let Some(sample) = stored
        .iter()
        .find(|s| s.host == host && s.quota.lane == lane)
    else {
        return Verdict::Clear;
    };
    let q = &sample.quota;
    if q.confidence == Confidence::Unknown {
        return Verdict::Clear;
    }
    let Some((window, used)) = worst_window(&q.windows, now) else {
        return Verdict::Clear;
    };
    let max_age = Duration::minutes(cfg.interval_minutes.max(1) as i64 * STALE_AFTER_INTERVALS);
    let stale = q.confidence == Confidence::Stale || now - q.observed_at > max_age;
    let notice = Notice {
        lane: lane.to_string(),
        window: window.label.clone(),
        used_percent: used,
        resets_at: window.resets_at,
        stale,
    };
    if used >= cfg.block_percent && !stale {
        Verdict::Block(notice)
    } else if used >= cfg.warn_percent {
        Verdict::Warn(notice)
    } else {
        Verdict::Clear
    }
}
