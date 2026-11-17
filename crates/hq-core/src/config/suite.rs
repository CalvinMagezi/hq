//! `suite:`: the HQ Suite's storage settings (`docs/suite/README.md`).

use serde::{Deserialize, Serialize};

/// Days a set-aside original is kept before the budget may reclaim it. Matches the vault's trash.
const DEFAULT_COLD_WINDOW_DAYS: u32 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuiteConfig {
    /// Run the storage governor: remove pictures nothing links to, mark notes whose original left
    /// the trash, and keep to the budget. Turn it off to stop all of that.
    #[serde(default = "default_governor")]
    pub governor: bool,

    /// What the suite may hold, in MB (pictures, temporary files and set-aside originals). Unset
    /// means no limit: the governor then only tidies, it never makes room.
    #[serde(default)]
    pub budget_mb: Option<u64>,

    /// Days an original set aside by a shrink stays before the budget may take it.
    #[serde(default = "default_cold_window_days")]
    pub cold_window_days: u32,
}

fn default_governor() -> bool {
    true
}

fn default_cold_window_days() -> u32 {
    DEFAULT_COLD_WINDOW_DAYS
}

impl Default for SuiteConfig {
    fn default() -> Self {
        SuiteConfig {
            governor: default_governor(),
            budget_mb: None,
            cold_window_days: default_cold_window_days(),
        }
    }
}
