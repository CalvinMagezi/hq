use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

const DEFAULT_INTERVAL_MINUTES: u64 = 10;
const DEFAULT_WARN_PERCENT: f64 = 80.0;
const DEFAULT_BLOCK_PERCENT: f64 = 95.0;

/// Reading each lane's remaining allowance on the machine its agent runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaConfig {
    /// Master switch. Off means no reader runs and nothing is asked of any vendor.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// How often HQ refreshes readings in the background.
    #[serde(default = "default_interval_minutes")]
    pub interval_minutes: u64,
    /// Per agent kind (`claude`, `codex`, `copilot`, `agy`, `cursor`, ...): `false` turns that
    /// reader off. A kind not listed is on.
    #[serde(default)]
    pub lanes: BTreeMap<String, bool>,
    /// Used percent from which a lane is called low.
    #[serde(default = "default_warn_percent")]
    pub warn_percent: f64,
    /// Used percent from which a lane is skipped when choosing one.
    #[serde(default = "default_block_percent")]
    pub block_percent: f64,
}

fn default_enabled() -> bool {
    true
}

fn default_interval_minutes() -> u64 {
    DEFAULT_INTERVAL_MINUTES
}

fn default_warn_percent() -> f64 {
    DEFAULT_WARN_PERCENT
}

fn default_block_percent() -> f64 {
    DEFAULT_BLOCK_PERCENT
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            interval_minutes: default_interval_minutes(),
            lanes: BTreeMap::new(),
            warn_percent: default_warn_percent(),
            block_percent: default_block_percent(),
        }
    }
}

impl QuotaConfig {
    /// Whether the reader for `kind` may run.
    pub fn lane_enabled(&self, kind: &str) -> bool {
        self.enabled && self.lanes.get(kind).copied().unwrap_or(true)
    }

    /// The kinds the owner turned off, for the host to skip.
    pub fn disabled_lanes(&self) -> Vec<String> {
        self.lanes
            .iter()
            .filter(|(_, on)| !**on)
            .map(|(k, _)| k.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_on_and_documented() {
        let c = QuotaConfig::default();
        assert!(c.enabled && c.lanes.is_empty());
        assert_eq!(c.interval_minutes, 10);
        assert_eq!((c.warn_percent, c.block_percent), (80.0, 95.0));
        let empty: QuotaConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty.interval_minutes, 10);
        assert!(empty.lane_enabled("claude"));
    }

    #[test]
    fn a_lane_can_be_turned_off_and_so_can_everything() {
        let c: QuotaConfig = serde_yaml::from_str("lanes: {codex: false}").unwrap();
        assert!(!c.lane_enabled("codex") && c.lane_enabled("claude"));
        assert_eq!(c.disabled_lanes(), ["codex"]);
        let off: QuotaConfig = serde_yaml::from_str("enabled: false").unwrap();
        assert!(!off.lane_enabled("claude"));
    }
}
