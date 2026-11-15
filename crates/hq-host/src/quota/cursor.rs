//! Cursor: the CLI names the subscription tier but exposes no usage figure.

use chrono::Utc;
use hq_core::quota::{Confidence, LaneQuota, QuotaSource};

use super::{LOCAL_CMD_TIMEOUT, OUTPUT_CAP_BYTES, run_capture};

const LANE: &str = "cursor";
const TIER_KEY: &str = "Subscription Tier";

pub(crate) fn parse_tier(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix(TIER_KEY))
        .map(|rest| rest.trim().to_string())
        .find(|tier| !tier.is_empty())
}

pub(crate) fn read() -> LaneQuota {
    let argv = ["cursor-agent", "about"].map(String::from).to_vec();
    let tier = run_capture(&argv, None, LOCAL_CMD_TIMEOUT, OUTPUT_CAP_BYTES)
        .ok()
        .and_then(|o| parse_tier(&o.stdout));
    LaneQuota {
        lane: LANE.into(),
        account: tier,
        windows: Vec::new(),
        source: QuotaSource::Cli,
        observed_at: Utc::now(),
        confidence: Confidence::Unknown,
        reason: Some("Cursor shows no usage figure in its CLI; check the vendor dashboard".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tier_is_read_from_about() {
        let text = "cursor-agent\nVersion  2026.11.01\nSubscription Tier  Pro\nModel  auto\n";
        assert_eq!(parse_tier(text).as_deref(), Some("Pro"));
        assert_eq!(parse_tier("nothing here"), None);
    }
}
