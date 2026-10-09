use serde::{Deserialize, Serialize};

/// Seconds an external work lease survives without a heartbeat.
pub const DEFAULT_LEASE_TTL_SECS: u64 = 900;

/// Hours of silence after which an in-progress task nobody holds is called stale.
pub const DEFAULT_STALE_AFTER_HOURS: u64 = 72;

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

    /// Hours without a write, comment or heartbeat after which an in-progress task nobody
    /// holds is reported as stale. Reporting only: nothing is ever changed for staleness.
    #[serde(default = "default_stale_after_hours")]
    pub stale_after_hours: u64,

    /// Whether a task's tags also route it to the agent mailbox of the same name, as they
    /// did before tasks had assignees. On by default so existing routing keeps working;
    /// turn it off once routing tags have been replaced by assignees, so tags stay topical.
    #[serde(default = "default_route_tags")]
    pub route_tags: bool,
}

impl TasksConfig {
    /// The ttl in force: the configured value, never below `MIN_LEASE_TTL_SECS`.
    pub fn lease_ttl(&self) -> u64 {
        self.lease_ttl_secs.max(MIN_LEASE_TTL_SECS)
    }

    /// The staleness window in force, at least one hour.
    pub fn stale_hours(&self) -> u64 {
        self.stale_after_hours.max(1)
    }
}

fn default_route_tags() -> bool {
    true
}

fn default_stale_after_hours() -> u64 {
    DEFAULT_STALE_AFTER_HOURS
}

fn default_lease_ttl_secs() -> u64 {
    DEFAULT_LEASE_TTL_SECS
}

impl Default for TasksConfig {
    fn default() -> Self {
        Self {
            require_lease: LeaseMode::default(),
            lease_ttl_secs: default_lease_ttl_secs(),
            stale_after_hours: default_stale_after_hours(),
            route_tags: default_route_tags(),
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
        assert_eq!(cfg.stale_hours(), DEFAULT_STALE_AFTER_HOURS);
        assert!(cfg.route_tags, "tags keep routing until someone turns it off");
    }

    #[test]
    fn the_stale_window_is_at_least_an_hour() {
        let cfg: TasksConfig = serde_yaml::from_str("stale_after_hours: 0").unwrap();
        assert_eq!(cfg.stale_hours(), 1);
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
