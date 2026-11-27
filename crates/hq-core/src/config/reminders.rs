use serde::{Deserialize, Serialize};

/// Reminders the owner set (`hq reminders`, the `remind_*` tools). The daemon task itself is
/// switched with `daemon.reminders`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemindersConfig {
    /// Local hours, `HH:MM-HH:MM`, in which a nudge's follow-up pings wait for the morning. The
    /// time the owner chose for a reminder always rings. Empty turns quiet hours off.
    #[serde(default = "default_quiet_hours")]
    pub quiet_hours: String,

    /// Local hour at which a nudge's next-day follow-ups ring.
    #[serde(default = "default_follow_up_hour")]
    pub follow_up_hour: u8,

    /// Most active reminders at once.
    #[serde(default = "default_max_active")]
    pub max_active: u32,

    /// Most reminders created in an hour, so a runaway session cannot flood the phone.
    #[serde(default = "default_max_per_hour")]
    pub max_per_hour: u32,
}

fn default_quiet_hours() -> String {
    "21:00-07:00".to_string()
}

fn default_follow_up_hour() -> u8 {
    9
}

fn default_max_active() -> u32 {
    200
}

fn default_max_per_hour() -> u32 {
    30
}

impl Default for RemindersConfig {
    fn default() -> Self {
        Self {
            quiet_hours: default_quiet_hours(),
            follow_up_hour: default_follow_up_hour(),
            max_active: default_max_active(),
            max_per_hour: default_max_per_hour(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_hold_follow_ups_overnight_and_cap_the_count() {
        let c = RemindersConfig::default();
        assert_eq!(c.quiet_hours, "21:00-07:00");
        assert_eq!(c.follow_up_hour, 9);
        assert_eq!((c.max_active, c.max_per_hour), (200, 30));
    }

    #[test]
    fn an_absent_section_takes_the_defaults() {
        let c: RemindersConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(c.follow_up_hour, 9);
        let c: RemindersConfig = serde_yaml::from_str("quiet_hours: ''").unwrap();
        assert!(c.quiet_hours.is_empty());
    }
}
