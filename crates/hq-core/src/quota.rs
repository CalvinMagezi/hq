//! What is left of each coding-agent lane's allowance, as the host reads it.
//!
//! A reading never carries a credential: `account` is a label such as a login name. Unknown is
//! its own answer (`Confidence::Unknown` with a `reason`), never a zero.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where a reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    Api,
    LocalLog,
    Cli,
    Ledger,
    Command,
}

/// How far a reading can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Live,
    Stale,
    Unknown,
}

/// What a window counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHour,
    Weekly,
    Monthly,
    Credits,
    Dollars,
}

/// One allowance window of a lane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub label: String,
    pub kind: WindowKind,
    /// `None` when the source does not say; never defaulted to zero.
    #[serde(default)]
    pub used_percent: Option<f64>,
    /// Human text for what is left, when a percentage does not tell it (`120 of 300 credits`).
    #[serde(default)]
    pub remaining: Option<String>,
    #[serde(default)]
    pub resets_at: Option<DateTime<Utc>>,
}

/// One lane's allowance reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneQuota {
    /// The agent kind (`claude`, `codex`, ...) or a profile name for a custom reader.
    pub lane: String,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub windows: Vec<QuotaWindow>,
    pub source: QuotaSource,
    pub observed_at: DateTime<Utc>,
    pub confidence: Confidence,
    #[serde(default)]
    pub reason: Option<String>,
}

impl LaneQuota {
    /// A reading that says the allowance is not known, and why.
    pub fn unknown(lane: &str, source: QuotaSource, reason: impl Into<String>) -> Self {
        Self {
            lane: lane.to_string(),
            account: None,
            windows: Vec::new(),
            source,
            observed_at: Utc::now(),
            confidence: Confidence::Unknown,
            reason: Some(reason.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_is_snake_case_and_unknown_has_no_windows() {
        let q = LaneQuota::unknown("codex", QuotaSource::LocalLog, "no log");
        let v = serde_json::to_value(&q).unwrap();
        assert_eq!(v["source"], "local_log");
        assert_eq!(v["confidence"], "unknown");
        assert!(v["windows"].as_array().unwrap().is_empty());
        let w = serde_json::to_value(WindowKind::FiveHour).unwrap();
        assert_eq!(w, "five_hour");
    }
}
