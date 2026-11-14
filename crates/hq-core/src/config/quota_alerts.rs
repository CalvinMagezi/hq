use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MIN_THRESHOLD_PCT: u8 = 1;
pub const MAX_THRESHOLD_PCT: u8 = 100;

/// Inbox alerts when a usage quota window crosses a configured percentage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaAlertsConfig {
    /// Disabled by default so unconfigured installs emit nothing.
    #[serde(default)]
    pub enabled: bool,
    /// Map of window id (for example `copilot:credits`) to percentage thresholds (clamped to 1..=100).
    #[serde(default)]
    pub thresholds: BTreeMap<String, Vec<u8>>,
}

impl QuotaAlertsConfig {
    /// Sorted, deduplicated thresholds for `window_id`, each clamped to `1..=100`.
    pub fn thresholds_for(&self, window_id: &str) -> Vec<u8> {
        let Some(raw) = self.thresholds.get(window_id) else {
            return Vec::new();
        };
        let mut out: Vec<u8> = raw
            .iter()
            .map(|&p| p.clamp(MIN_THRESHOLD_PCT, MAX_THRESHOLD_PCT))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// True when alerts are enabled and at least one window has thresholds configured.
    pub fn is_active(&self) -> bool {
        self.enabled && self.thresholds.values().any(|v| !v.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_disabled_and_empty() {
        let cfg: QuotaAlertsConfig = serde_yaml::from_str("{}").unwrap();
        assert!(!cfg.enabled);
        assert!(cfg.thresholds.is_empty());
        assert!(!cfg.is_active());
    }

    #[test]
    fn clamps_sorts_and_deduplicates_thresholds() {
        let yaml = "enabled: true\nthresholds:\n  'copilot:credits': [0, 95, 80, 150, 80]\n";
        let cfg: QuotaAlertsConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.is_active());
        assert_eq!(cfg.thresholds_for("copilot:credits"), vec![1, 80, 95, 100]);
        assert!(cfg.thresholds_for("other:window").is_empty());
    }

    #[test]
    fn enabled_without_thresholds_is_inactive() {
        let yaml = "enabled: true\nthresholds:\n  'copilot:credits': []\n";
        let cfg: QuotaAlertsConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!cfg.is_active());
    }
}
