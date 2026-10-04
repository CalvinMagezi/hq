use serde::{Deserialize, Serialize};

/// Sub-agent spawning limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaborationConfig {
    /// Maximum sub-agent nesting depth (default 3). Prevents infinite recursion.
    #[serde(default = "default_max_subagent_depth")]
    pub max_subagent_depth: u32,

    /// Default timeout for synchronous sub-agent spawns, in seconds (default 300).
    #[serde(default = "default_subagent_timeout_secs")]
    pub subagent_timeout_secs: u64,

    /// When a background sub-agent settles, start a follow-up turn in the
    /// originating chat so HQ verifies the result without a user message.
    /// Off by default; run records and the event outbox are kept either way.
    #[serde(default)]
    pub supervision_followup: bool,

    /// Most automatic follow-up turns per chat per day (default 48).
    #[serde(default = "default_followups_per_day")]
    pub followups_per_chat_per_day: u32,
}

fn default_max_subagent_depth() -> u32 {
    3
}
fn default_subagent_timeout_secs() -> u64 {
    300
}
fn default_followups_per_day() -> u32 {
    48
}

impl Default for CollaborationConfig {
    fn default() -> Self {
        Self {
            max_subagent_depth: default_max_subagent_depth(),
            subagent_timeout_secs: default_subagent_timeout_secs(),
            supervision_followup: false,
            followups_per_chat_per_day: default_followups_per_day(),
        }
    }
}
