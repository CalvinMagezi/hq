use serde::{Deserialize, Serialize};

/// Seconds an external work lease survives without a heartbeat.
pub const DEFAULT_LEASE_TTL_SECS: u64 = 900;

/// The shortest ttl honoured, so a typo of 0 or 1 does not expire every lease at once.
pub const MIN_LEASE_TTL_SECS: u64 = 60;

/// Whether an agent must hold a work lease to start a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseMode {
    /// Nothing is checked. The default, so a fresh install needs no setup.
    #[default]
    Off,
    /// Starting a task without a lease succeeds and the reply carries a warning.
    Warn,
    /// Starting a task without a lease is refused with how to claim one.
    Enforce,
}

/// Task system settings (`tasks:` in the config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksConfig {
    /// Applies to agents calling `task_update` over MCP. People using the web
    /// board are never asked for a lease.
    #[serde(default)]
    pub require_lease: LeaseMode,

    /// How long an external lease lives without a heartbeat before it counts
    /// as ended at its last heartbeat.
    #[serde(default = "default_lease_ttl_secs")]
    pub lease_ttl_secs: u64,
}

impl TasksConfig {
    /// The ttl in force: the configured value, never below `MIN_LEASE_TTL_SECS`.
    pub fn lease_ttl(&self) -> u64 {
        self.lease_ttl_secs.max(MIN_LEASE_TTL_SECS)
    }
}

fn default_lease_ttl_secs() -> u64 {
    DEFAULT_LEASE_TTL_SECS
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            require_lease: LeaseMode::default(),
            lease_ttl_secs: default_lease_ttl_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_ask_for_nothing() {
        let cfg: TasksConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.require_lease, LeaseMode::Off);
        assert_eq!(cfg.lease_ttl_secs, DEFAULT_LEASE_TTL_SECS);
    }

    #[test]
    fn a_tiny_ttl_is_raised_to_the_floor() {
        let cfg: TasksConfig = serde_yaml::from_str("lease_ttl_secs: 0").unwrap();
        assert_eq!(cfg.lease_ttl(), MIN_LEASE_TTL_SECS);
        let cfg: TasksConfig = serde_yaml::from_str("lease_ttl_secs: 3600").unwrap();
        assert_eq!(cfg.lease_ttl(), 3600);
    }

    #[test]
    fn modes_read_as_lowercase_words() {
        let cfg: TasksConfig = serde_yaml::from_str("require_lease: enforce").unwrap();
        assert_eq!(cfg.require_lease, LeaseMode::Enforce);
        let cfg: TasksConfig = serde_yaml::from_str("require_lease: warn").unwrap();
        assert_eq!(cfg.require_lease, LeaseMode::Warn);
    }
}
