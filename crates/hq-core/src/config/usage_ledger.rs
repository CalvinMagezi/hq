use serde::{Deserialize, Serialize};

/// Reports (`hq usage`, budgets, forecasts) read raw ledger rows for up to this many days, so
/// retention never goes below it.
pub const MIN_RETAIN_RAW_DAYS: i64 = 35;

/// Housekeeping for the LLM spend ledger. Old rows are folded into daily totals, never lost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageLedgerConfig {
    /// Days of per-call rows to keep before they are folded into the daily rollup.
    #[serde(default = "default_retain_raw_days")]
    pub retain_raw_days: i64,
}

fn default_retain_raw_days() -> i64 {
    90
}

impl UsageLedgerConfig {
    pub fn effective_retain_days(&self) -> i64 {
        self.retain_raw_days.max(MIN_RETAIN_RAW_DAYS)
    }
}

impl Default for UsageLedgerConfig {
    fn default() -> Self {
        Self {
            retain_raw_days: default_retain_raw_days(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_defaults_to_ninety_days_and_never_drops_below_the_report_window() {
        assert_eq!(UsageLedgerConfig::default().effective_retain_days(), 90);
        let tiny: UsageLedgerConfig = serde_yaml::from_str("retain_raw_days: 3").unwrap();
        assert_eq!(tiny.effective_retain_days(), MIN_RETAIN_RAW_DAYS);
        let empty: UsageLedgerConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty.retain_raw_days, 90);
    }
}
