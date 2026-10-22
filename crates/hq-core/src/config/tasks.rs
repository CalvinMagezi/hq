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

/// How completion is verified (`tasks.verification.mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMode {
    /// Nothing changes. The default, so a fresh install needs no setup.
    #[default]
    Off,
    /// Tasks waiting for review get verdicts that move nothing, to measure the verifier first.
    Shadow,
    /// `complete` is reachable only through a passing verdict or the owner.
    Enforce,
}

/// Risk tiers a task can sit in. The default is every tier, so `mode: enforce` means all of them
/// until `enforce_tiers` narrows it for a staged rollout.
pub const ALL_TIERS: [u8; 3] = [0, 1, 2];

/// Completion verification (`tasks.verification:`). See `docs/plans/task-verification.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerificationConfig {
    pub mode: VerificationMode,
    /// Tiers whose completion is gated when `mode` is `enforce`. Others behave as before.
    pub enforce_tiers: Vec<u8>,
    /// Failed verifications after which a task goes to the owner as an exception.
    pub max_attempts: u32,
    /// Tags that make a task tier 2 (irreversible or high blast radius).
    pub tier2_tags: Vec<String>,
    /// Path prefixes in a task's change that make it tier 2.
    pub tier2_paths: Vec<String>,
    /// Hours the owner may reverse a tier 2 completion.
    pub veto_hours: u32,
    /// Share of auto-completed tasks pushed to the owner as an audit sample.
    pub audit_rate: f64,
    /// Fewest audits pushed in a week, whatever the rate gives.
    pub audit_min_per_week: u32,
    /// Verifications run at once.
    pub concurrency: u32,
    /// A sweep over more tasks than this always needs the owner's confirmation to apply.
    pub sweep_confirm_over: u32,
    /// Items waiting on the owner past which the briefing offers a bulk policy, not a list.
    pub owner_queue_alert: u32,
    /// Days the oldest owner-waiting item may age before one alert is raised.
    pub owner_queue_age_days: u32,
    /// Tasks given a reviewer run in shadow mode, to price the rest.
    pub shadow_e2_sample: u32,
    /// Monthly spend allowed on reviewer (model) runs, in USD. Unset: no model is called and only
    /// deterministic checks run.
    pub reviewer_monthly_ceiling_usd: Option<f64>,
    /// Models that review, as router aliases or model ids. Empty means the `premium` alias. A
    /// tier 2 task needs reviewers from two different model families, so two entries that route
    /// to one family leave it undecided rather than pretending to be independent.
    pub reviewer_models: Vec<String>,
    /// Seconds one reviewer call may take.
    pub reviewer_timeout_secs: u64,
    /// Let a code task pass on an independent review alone when its pull request has no checks to
    /// read. Off by default: code needs machine evidence.
    pub peer_only_code: bool,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            mode: VerificationMode::Off,
            enforce_tiers: ALL_TIERS.to_vec(),
            max_attempts: 3,
            tier2_tags: [
                "security",
                "payments",
                "production",
                "deploy",
                "migration",
                "outbound",
                "deletion",
            ]
            .map(String::from)
            .to_vec(),
            tier2_paths: [".github/workflows/", "deploy/", "migrations/", "auth/"]
                .map(String::from)
                .to_vec(),
            veto_hours: 24,
            audit_rate: 0.05,
            audit_min_per_week: 10,
            concurrency: 2,
            sweep_confirm_over: 25,
            owner_queue_alert: 20,
            owner_queue_age_days: 3,
            shadow_e2_sample: 50,
            reviewer_monthly_ceiling_usd: None,
            reviewer_models: Vec::new(),
            reviewer_timeout_secs: 120,
            peer_only_code: false,
        }
    }
}

impl VerificationConfig {
    /// Whether tier `tier` is gated by this configuration.
    pub fn enforces(&self, tier: u8) -> bool {
        self.mode == VerificationMode::Enforce && self.enforce_tiers.contains(&tier)
    }

    /// Whether a reviewer (model) may be called at all: only with a ceiling above zero.
    pub fn reviewer_enabled(&self) -> bool {
        self.reviewer_monthly_ceiling_usd.is_some_and(|c| c > 0.0)
    }

    /// The audit share, kept inside 0 to 1.
    pub fn audit_share(&self) -> f64 {
        if self.audit_rate.is_finite() {
            self.audit_rate.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The models to review with: those configured, else the `premium` alias.
    pub fn reviewers(&self) -> Vec<String> {
        let configured: Vec<String> = self
            .reviewer_models
            .iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        if configured.is_empty() {
            vec!["premium".to_string()]
        } else {
            configured
        }
    }

    /// Verifications at once, at least one.
    pub fn parallelism(&self) -> usize {
        self.concurrency.max(1) as usize
    }
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

    /// Verifying completion without a person reading each task.
    #[serde(default)]
    pub verification: VerificationConfig,
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
            verification: VerificationConfig::default(),
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
        assert!(
            cfg.route_tags,
            "tags keep routing until someone turns it off"
        );
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
    fn verification_is_off_and_asks_for_no_spend_by_default() {
        let cfg: TasksConfig = serde_yaml::from_str("{}").unwrap();
        let v = &cfg.verification;
        assert_eq!(v.mode, VerificationMode::Off);
        assert!(!v.enforces(0) && !v.enforces(1) && !v.enforces(2));
        assert!(
            !v.reviewer_enabled(),
            "no model is called without a ceiling"
        );
        assert_eq!(
            (v.max_attempts, v.veto_hours, v.sweep_confirm_over),
            (3, 24, 25)
        );
        assert_eq!(
            (
                v.owner_queue_alert,
                v.owner_queue_age_days,
                v.shadow_e2_sample
            ),
            (20, 3, 50)
        );
    }

    #[test]
    fn verification_reads_from_yaml_and_enforces_only_the_tiers_named() {
        let cfg: TasksConfig = serde_yaml::from_str(
            "verification:\n  mode: enforce\n  enforce_tiers: [0]\n  reviewer_monthly_ceiling_usd: 20\n",
        )
        .unwrap();
        let v = &cfg.verification;
        assert!(v.enforces(0));
        assert!(!v.enforces(1), "tier 1 is not staged in yet");
        assert!(v.reviewer_enabled());
        let shadow: TasksConfig = serde_yaml::from_str("verification:\n  mode: shadow\n").unwrap();
        assert!(!shadow.verification.enforces(0), "shadow gates nothing");
    }

    #[test]
    fn the_reviewer_defaults_to_the_premium_alias_and_reads_a_list() {
        let cfg: TasksConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(cfg.verification.reviewers(), ["premium"]);
        assert!(!cfg.verification.peer_only_code);
        let cfg: TasksConfig = serde_yaml::from_str(
            "verification:\n  reviewer_models: [\" a/b \", \"\", c]\n  peer_only_code: true\n",
        )
        .unwrap();
        assert_eq!(cfg.verification.reviewers(), ["a/b", "c"]);
        assert!(cfg.verification.peer_only_code);
    }

    #[test]
    fn nonsense_numbers_are_kept_in_range() {
        let cfg: TasksConfig = serde_yaml::from_str(
            "verification:\n  audit_rate: 7\n  concurrency: 0\n  reviewer_monthly_ceiling_usd: 0\n",
        )
        .unwrap();
        assert_eq!(cfg.verification.audit_share(), 1.0);
        assert_eq!(cfg.verification.parallelism(), 1);
        assert!(
            !cfg.verification.reviewer_enabled(),
            "a zero ceiling is no ceiling"
        );
    }

    #[test]
    fn modes_read_as_lowercase_words() {
        let cfg: TasksConfig = serde_yaml::from_str("require_lease: enforce").unwrap();
        assert_eq!(cfg.require_lease, LeaseMode::Enforce);
        let cfg: TasksConfig = serde_yaml::from_str("require_lease: warn").unwrap();
        assert_eq!(cfg.require_lease, LeaseMode::Warn);
    }
}
