use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How the daemon notices edits made to notes outside HQ (an editor, git, a coding agent's
/// shell) so search sees them in seconds instead of at the next scheduled scan.
///
/// On by default. It only reads notes and updates the search index. The scheduled scan keeps
/// running either way, so a lost event is repaired and turning this off costs only latency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultWatchConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// How changes are detected. `auto` uses the operating system's file events and falls back
    /// to polling where they cannot be used (watch limits, or a Windows drive mounted into
    /// WSL2, which delivers none).
    #[serde(default)]
    pub mode: VaultWatchMode,

    /// Quiet time after a path's last event before it is read again. Editors save in bursts
    /// and by replacing files, so this keeps one save from being indexed several times.
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,

    /// How often to scan when polling.
    #[serde(default = "default_poll_secs")]
    pub poll_secs: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VaultWatchMode {
    #[default]
    Auto,
    Native,
    Poll,
}

fn default_enabled() -> bool {
    true
}

fn default_debounce_ms() -> u64 {
    300
}

fn default_poll_secs() -> u64 {
    10
}

impl Default for VaultWatchConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            mode: VaultWatchMode::default(),
            debounce_ms: default_debounce_ms(),
            poll_secs: default_poll_secs(),
        }
    }
}

impl VaultWatchConfig {
    /// The debounce window, kept within a range that still coalesces a save but stays prompt.
    pub fn debounce(&self) -> Duration {
        Duration::from_millis(self.debounce_ms.clamp(50, 10_000))
    }

    /// The polling interval, never tighter than 2 seconds.
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_secs.max(2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_by_default_with_automatic_mode() {
        let c = VaultWatchConfig::default();
        assert!(c.enabled);
        assert_eq!(c.mode, VaultWatchMode::Auto);
        assert_eq!(c.debounce(), Duration::from_millis(300));
        assert_eq!(c.poll_interval(), Duration::from_secs(10));
    }

    #[test]
    fn an_absent_section_deserializes_to_the_defaults() {
        let c: VaultWatchConfig = serde_yaml::from_str("{}").unwrap();
        assert!(c.enabled);
        assert_eq!(c.debounce_ms, 300);
    }

    #[test]
    fn modes_are_lowercase_words() {
        let c: VaultWatchConfig = serde_yaml::from_str("mode: poll\npoll_secs: 30").unwrap();
        assert_eq!(c.mode, VaultWatchMode::Poll);
        assert_eq!(c.poll_interval(), Duration::from_secs(30));
        assert!(serde_yaml::from_str::<VaultWatchConfig>("mode: sometimes").is_err());
    }

    #[test]
    fn extreme_values_are_clamped() {
        let c: VaultWatchConfig = serde_yaml::from_str("debounce_ms: 0\npoll_secs: 0").unwrap();
        assert_eq!(c.debounce(), Duration::from_millis(50));
        assert_eq!(c.poll_interval(), Duration::from_secs(2));
        let c: VaultWatchConfig = serde_yaml::from_str("debounce_ms: 99999999").unwrap();
        assert_eq!(c.debounce(), Duration::from_secs(10));
    }
}
