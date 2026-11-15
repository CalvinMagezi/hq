//! Agents with no balance API. HQ's own spend ledger is the only record of what they use.

use hq_core::quota::{LaneQuota, QuotaSource};

pub(crate) fn read(lane: &str) -> LaneQuota {
    LaneQuota::unknown(
        lane,
        QuotaSource::Ledger,
        "no balance API; spend is in HQ's own ledger",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::quota::Confidence;

    #[test]
    fn a_ledger_lane_is_unknown_not_zero() {
        let q = read("opencode");
        assert_eq!(q.confidence, Confidence::Unknown);
        assert!(q.windows.is_empty());
    }
}
