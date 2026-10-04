use serde::{Deserialize, Serialize};

/// Disk/build-artifact threshold monitoring, ported from OpenClaw's
/// `disk-watchdog.sh` when OpenClaw was retired (2026-08-10). On by
/// default: it only ever reads and reports, never deletes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskWatchdogConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Overall volume % used.
    #[serde(default = "default_disk_pct_threshold")]
    pub disk_pct_threshold: u8,

    /// `~/.bun/install/cache` size, in MB.
    #[serde(default = "default_bun_cache_threshold_mb")]
    pub bun_cache_threshold_mb: u64,

    /// Any single `node_modules/` dir under a watch root, in MB.
    #[serde(default = "default_node_modules_threshold_mb")]
    pub node_modules_threshold_mb: u64,

    /// Any single `target/` dir under a watch root, in MB.
    #[serde(default = "default_cargo_target_threshold_mb")]
    pub cargo_target_threshold_mb: u64,

    /// Directories to scan for `node_modules`/`target` bloat. Empty means
    /// "just the agent-hq repo itself" (derived from the vault path at
    /// call time, not baked in here — this struct has no vault context).
    #[serde(default)]
    pub watch_roots: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_disk_pct_threshold() -> u8 {
    90
}

fn default_bun_cache_threshold_mb() -> u64 {
    6144
}

fn default_node_modules_threshold_mb() -> u64 {
    1024
}

fn default_cargo_target_threshold_mb() -> u64 {
    10240
}

impl Default for DiskWatchdogConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            disk_pct_threshold: default_disk_pct_threshold(),
            bun_cache_threshold_mb: default_bun_cache_threshold_mb(),
            node_modules_threshold_mb: default_node_modules_threshold_mb(),
            cargo_target_threshold_mb: default_cargo_target_threshold_mb(),
            watch_roots: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_by_default_with_the_openclaw_scripts_original_thresholds() {
        let c = DiskWatchdogConfig::default();
        assert!(c.enabled);
        assert_eq!(c.disk_pct_threshold, 90);
        assert_eq!(c.bun_cache_threshold_mb, 6144);
        assert_eq!(c.node_modules_threshold_mb, 1024);
        assert_eq!(c.cargo_target_threshold_mb, 10240);
        assert!(c.watch_roots.is_empty());
    }

    #[test]
    fn an_absent_section_deserializes_to_the_defaults() {
        let c: DiskWatchdogConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(
            c.disk_pct_threshold,
            DiskWatchdogConfig::default().disk_pct_threshold
        );
    }
}
