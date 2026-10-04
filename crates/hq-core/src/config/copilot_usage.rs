use serde::{Deserialize, Serialize};

use super::{BackendEntry, BackendKind, HqConfig};

const DEFAULT_INTERVAL_MINUTES: u64 = 10;
const COPILOT_HOST: &str = "githubcopilot.com";

/// Sampling of the Copilot credit balance for the burn-rate meter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotUsageConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_interval_minutes")]
    pub interval_minutes: u64,
    /// Show approximate credits per agent step in the web chat.
    #[serde(default = "default_enabled")]
    pub per_step: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_interval_minutes() -> u64 {
    DEFAULT_INTERVAL_MINUTES
}

impl Default for CopilotUsageConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            interval_minutes: default_interval_minutes(),
            per_step: default_enabled(),
        }
    }
}

fn is_copilot_backend(b: &BackendEntry) -> bool {
    match b.kind {
        BackendKind::GithubCopilotApi => true,
        BackendKind::OpenaiCompatible => b
            .endpoint
            .as_deref()
            .is_some_and(|e| e.contains(COPILOT_HOST)),
        _ => false,
    }
}

/// True when an enabled backend in the chain talks to GitHub Copilot.
pub fn copilot_active(config: &HqConfig) -> bool {
    config
        .backends
        .backends
        .iter()
        .any(|b| b.enabled && is_copilot_backend(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(yaml: &str) -> HqConfig {
        HqConfig {
            backends: serde_yaml::from_str(yaml).unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn detects_native_and_compatible_copilot_backends() {
        let native = "primary: a\nbackends:\n  - name: a\n    kind: github-copilot-api\n";
        let compat = "primary: a\nbackends:\n  - name: a\n    kind: openai-compatible\n    endpoint: https://api.githubcopilot.com\n";
        let other = "primary: a\nbackends:\n  - name: a\n    kind: openai-compatible\n    endpoint: https://api.deepseek.com\n";
        let off = "primary: a\nbackends:\n  - name: a\n    kind: github-copilot-api\n    enabled: false\n";
        assert!(copilot_active(&cfg(native)));
        assert!(copilot_active(&cfg(compat)));
        assert!(!copilot_active(&cfg(other)));
        assert!(!copilot_active(&cfg(off)));
        assert!(!copilot_active(&HqConfig::default()));
    }

    #[test]
    fn defaults_to_ten_minutes() {
        let c: CopilotUsageConfig = serde_yaml::from_str("{}").unwrap();
        assert!(c.enabled && c.interval_minutes == 10);
    }
}
