use serde::{Deserialize, Serialize};

/// What the inbox keeps quiet. Today only the fleet of helper sessions, under `inbox.fleet`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InboxConfig {
    #[serde(default)]
    pub fleet: FleetInboxConfig,
}

/// Which helper requests the owner is asked about. Only the owner can answer a sign-in, a
/// connector approval or a call that loses data; this controls the rest.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperPolicy {
    /// The owner's agents and command policies handle helper prompts. The owner hears of a
    /// helper only through a decision that is theirs alone, or one nobody answered in time.
    #[default]
    Nothing,
    /// As `nothing`, and a prompt that would lose work reaches the owner at once.
    LosesWork,
    /// Every helper prompt reaches the owner, as before the fleet rules existed.
    Everything,
}

impl HelperPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            HelperPolicy::Nothing => "nothing",
            HelperPolicy::LosesWork => "loses_work",
            HelperPolicy::Everything => "everything",
        }
    }
}

pub const DEFAULT_GRACE_MINUTES: u64 = 15;
pub const DEFAULT_TTL_HOURS: u64 = 24;
pub const DEFAULT_SIGNIN_TTL_HOURS: u64 = 24;
/// Longest grace accepted, so a typo cannot hide a block for days.
const MAX_GRACE_MINUTES: u64 = 24 * 60;
/// Bounds for the tidy's lifetimes, so 0 cannot expire everything at once and a huge value
/// cannot overflow a duration.
const MIN_TTL_HOURS: u64 = 1;
const MAX_TTL_HOURS: u64 = 24 * 365;

/// `inbox.fleet`: how quiet the Inbox is about helper sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetInboxConfig {
    /// Master switch. Off restores the old behaviour: every helper prompt and every finished
    /// session becomes an inbox item.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Minutes a prompt that a policy, a driving chat, a parent or a task lead can answer may
    /// stand unanswered before it becomes one inbox item.
    #[serde(default = "default_grace_minutes")]
    pub grace_minutes: u64,
    /// Hours after which a helper request still waiting is moved out of Needs you by the
    /// daily tidy (it stays restorable for 30 days).
    #[serde(default = "default_ttl_hours")]
    pub ttl_hours: u64,
    /// Hours after which a new-device sign-in request is moved out of Needs you.
    #[serde(default = "default_signin_ttl_hours")]
    pub signin_ttl_hours: u64,
    /// Fold finished and stopped helpers into one rolling item per day instead of one each.
    #[serde(default = "default_true")]
    pub digest: bool,
    /// Which helper prompts the owner is asked about.
    #[serde(default)]
    pub helper_policy: HelperPolicy,
}

fn default_true() -> bool {
    true
}
fn default_grace_minutes() -> u64 {
    DEFAULT_GRACE_MINUTES
}
fn default_ttl_hours() -> u64 {
    DEFAULT_TTL_HOURS
}
fn default_signin_ttl_hours() -> u64 {
    DEFAULT_SIGNIN_TTL_HOURS
}

impl Default for FleetInboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            grace_minutes: DEFAULT_GRACE_MINUTES,
            ttl_hours: DEFAULT_TTL_HOURS,
            signin_ttl_hours: DEFAULT_SIGNIN_TTL_HOURS,
            digest: true,
            helper_policy: HelperPolicy::Nothing,
        }
    }
}

impl FleetInboxConfig {
    /// `ttl_hours` kept within `1..=24*365`.
    pub fn ttl(&self) -> chrono::Duration {
        chrono::Duration::hours(self.ttl_hours.clamp(MIN_TTL_HOURS, MAX_TTL_HOURS) as i64)
    }

    /// `signin_ttl_hours` kept within `1..=24*365`.
    pub fn signin_ttl(&self) -> chrono::Duration {
        chrono::Duration::hours(self.signin_ttl_hours.clamp(MIN_TTL_HOURS, MAX_TTL_HOURS) as i64)
    }

    pub fn grace(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.grace_minutes.min(MAX_GRACE_MINUTES) * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_quiet_and_generic() {
        let c = InboxConfig::default().fleet;
        assert!(c.enabled && c.digest);
        assert_eq!(c.grace_minutes, 15);
        assert_eq!((c.ttl_hours, c.signin_ttl_hours), (24, 24));
        assert_eq!(c.helper_policy, HelperPolicy::Nothing);
    }

    #[test]
    fn a_partial_section_keeps_the_other_defaults() {
        let c: InboxConfig =
            serde_yaml::from_str("fleet:\n  helper_policy: loses_work\n  grace_minutes: 3")
                .unwrap();
        assert_eq!(c.fleet.helper_policy, HelperPolicy::LosesWork);
        assert_eq!(c.fleet.grace().as_secs(), 180);
        assert_eq!(c.fleet.ttl_hours, 24);
    }

    #[test]
    fn lifetimes_are_clamped_so_they_cannot_panic_or_expire_everything() {
        let wild = FleetInboxConfig {
            ttl_hours: 0,
            signin_ttl_hours: u64::MAX,
            ..Default::default()
        };
        assert_eq!(wild.ttl(), chrono::Duration::hours(1));
        assert_eq!(wild.signin_ttl(), chrono::Duration::hours(24 * 365));
    }
}
