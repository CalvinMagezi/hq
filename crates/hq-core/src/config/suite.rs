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

    /// Scheduled snapshots of dashboard notes. Off by default.
    #[serde(default)]
    pub dashboard_refresh: DashboardRefreshConfig,
}

/// `suite.dashboard_refresh`: the daemon task `suite-dashboard-refresh` writes a PDF and a PNG
/// snapshot of each listed dashboard note into `Exports/<yyyy-mm>/` and logs one line. It sends
/// nothing anywhere; handing a snapshot to a chat stays an explicit act (`file_deliver`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardRefreshConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Minutes between runs. The daemon looks every few minutes and acts when this long has passed
    /// since the last run, so a value below 10 acts as 10.
    #[serde(default = "default_refresh_every_minutes")]
    pub every_minutes: u32,
    /// Vault-relative paths of the dashboard notes, such as `Notebooks/Sales dashboard.md`. At most
    /// 20 are used.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Fewest minutes between two runs, and most notes one run takes.
pub const DASHBOARD_REFRESH_MIN_MINUTES: u32 = 10;
pub const DASHBOARD_REFRESH_MAX_NOTES: usize = 20;

fn default_refresh_every_minutes() -> u32 {
    1440
}

impl Default for DashboardRefreshConfig {
    fn default() -> Self {
        DashboardRefreshConfig {
            enabled: false,
            every_minutes: default_refresh_every_minutes(),
            notes: Vec::new(),
        }
    }
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
            dashboard_refresh: DashboardRefreshConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dashboard_refresh_is_off_daily_and_lists_no_notes_by_default() {
        let c = SuiteConfig::default().dashboard_refresh;
        assert!(!c.enabled);
        assert_eq!(c.every_minutes, 1440);
        assert!(c.notes.is_empty());
        let absent: SuiteConfig = serde_yaml::from_str("governor: true").unwrap();
        assert_eq!(absent.dashboard_refresh, c);
    }

    #[test]
    fn the_section_reads_from_yaml_and_a_partial_one_keeps_the_other_defaults() {
        let c: SuiteConfig = serde_yaml::from_str(
            "dashboard_refresh:\n  enabled: true\n  notes: [\"Notebooks/Sales.md\"]\n",
        )
        .unwrap();
        assert!(c.dashboard_refresh.enabled);
        assert_eq!(c.dashboard_refresh.every_minutes, 1440);
        assert_eq!(c.dashboard_refresh.notes, ["Notebooks/Sales.md"]);
        assert!(c.governor, "the rest of the section is untouched");
    }
}
