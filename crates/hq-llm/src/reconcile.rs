//! Compares what HQ's ledger says it spent with what a provider says it billed. Pure: the caller
//! supplies both figures, so the drift rule is testable without a network.

use serde::Serialize;

/// Drift beyond this share of the provider's figure is reported as a mismatch.
pub const DRIFT_TOLERANCE_PCT: f64 = 10.0;
/// Below this, both figures are noise and no verdict is worth giving.
const MIN_MEANINGFUL_USD: f64 = 0.01;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Aligned,
    /// HQ recorded less than the provider billed: calls are missing or priced too low.
    LedgerLow,
    /// HQ recorded more than the provider billed: prices are too high or calls are double counted.
    LedgerHigh,
    /// The provider gave no figure for this window.
    ProviderUnknown,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WindowDrift {
    pub window: String,
    pub ledger_usd: f64,
    pub provider_usd: Option<f64>,
    /// Signed: positive means the ledger is above the provider.
    pub drift_pct: Option<f64>,
    /// Calls in the window whose cost HQ could not price, so the ledger is a lower bound.
    pub unpriced_calls: i64,
    pub verdict: Verdict,
}

pub fn compare(
    window: &str,
    ledger_usd: f64,
    unpriced_calls: i64,
    provider_usd: Option<f64>,
) -> WindowDrift {
    let (drift_pct, verdict) = match provider_usd {
        None => (None, Verdict::ProviderUnknown),
        Some(p) if p < MIN_MEANINGFUL_USD && ledger_usd < MIN_MEANINGFUL_USD => {
            (Some(0.0), Verdict::Aligned)
        }
        Some(p) if p < MIN_MEANINGFUL_USD => (None, Verdict::LedgerHigh),
        Some(p) => {
            let pct = (ledger_usd - p) / p * 100.0;
            let verdict = if pct.abs() <= DRIFT_TOLERANCE_PCT {
                Verdict::Aligned
            } else if pct < 0.0 {
                Verdict::LedgerLow
            } else {
                Verdict::LedgerHigh
            };
            (Some(pct), verdict)
        }
    };
    WindowDrift {
        window: window.to_string(),
        ledger_usd,
        provider_usd,
        drift_pct,
        unpriced_calls,
        verdict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_drift_is_aligned_and_large_drift_names_its_direction() {
        assert_eq!(compare("day", 1.05, 0, Some(1.0)).verdict, Verdict::Aligned);
        assert_eq!(
            compare("day", 0.5, 0, Some(1.0)).verdict,
            Verdict::LedgerLow
        );
        assert_eq!(
            compare("day", 2.0, 0, Some(1.0)).verdict,
            Verdict::LedgerHigh
        );
        let d = compare("day", 0.5, 0, Some(1.0));
        assert_eq!(d.drift_pct, Some(-50.0));
    }

    #[test]
    fn no_provider_figure_is_unknown_not_aligned() {
        assert_eq!(
            compare("day", 1.0, 0, None).verdict,
            Verdict::ProviderUnknown
        );
    }

    #[test]
    fn two_negligible_figures_agree_and_spend_against_a_zero_bill_is_flagged() {
        assert_eq!(compare("day", 0.0, 0, Some(0.0)).verdict, Verdict::Aligned);
        assert_eq!(
            compare("day", 3.0, 0, Some(0.0)).verdict,
            Verdict::LedgerHigh
        );
    }
}
