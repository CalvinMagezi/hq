use serde::{Deserialize, Serialize};

/// Dream engine configuration. `enabled` gates the memory-consolidation,
/// inbox-triage and outreach daemon tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamConfig {
    #[serde(default = "default_dream_enabled")]
    pub enabled: bool,
}

fn default_dream_enabled() -> bool {
    true
}

impl Default for DreamConfig {
    fn default() -> Self {
        Self {
            enabled: default_dream_enabled(),
        }
    }
}
