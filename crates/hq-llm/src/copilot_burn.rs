//! Burn rate and projections from stored Copilot credit samples. Pure: no I/O, no clock.

use crate::copilot_usage::CopilotQuota;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

pub const WINDOWS_HOURS: [i64; 3] = [1, 6, 24];
const LOW_CONFIDENCE_SAMPLES: usize = 3;
const LOW_CONFIDENCE_MINUTES: i64 = 30;
const HIGH_CONFIDENCE_HOURS: i64 = 6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BurnSample {
    pub ts: DateTime<Utc>,
    pub credits_used: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WindowBurn {
    pub window_hours: i64,
    /// Credits spent between the first and last usable sample in the window.
    pub credits_used: Option<f64>,
    pub per_hour: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BurnReport {
    pub windows: Vec<WindowBurn>,
    pub projected_at_reset: Option<f64>,
    pub hours_to_reset: Option<f64>,
    pub projected_exhaustion_at: Option<DateTime<Utc>>,
    /// Whether the balance runs out before the cycle resets at the current rate.
    pub exhausts_before_reset: bool,
    /// Reported separately: running out is not a hard stop when overage is allowed.
    pub overage_permitted: bool,
    pub sample_count: usize,
    pub confidence: Confidence,
}

fn hours_between(a: DateTime<Utc>, b: DateTime<Utc>) -> f64 {
    (b - a).num_seconds() as f64 / 3600.0
}

/// Samples inside the window, starting after the latest cycle reset (a drop in credits used).
fn usable(samples: &[BurnSample], since: DateTime<Utc>) -> &[BurnSample] {
    let start = samples.partition_point(|s| s.ts < since);
    let inside = &samples[start..];
    let cut = inside
        .windows(2)
        .rposition(|w| w[1].credits_used < w[0].credits_used)
        .map_or(0, |i| i + 1);
    &inside[cut..]
}

fn window_burn(samples: &[BurnSample], now: DateTime<Utc>, hours: i64) -> WindowBurn {
    let pts = usable(samples, now - Duration::hours(hours));
    let none = WindowBurn {
        window_hours: hours,
        credits_used: None,
        per_hour: None,
    };
    let (Some(first), Some(last)) = (pts.first(), pts.last()) else {
        return none;
    };
    let span = hours_between(first.ts, last.ts);
    if span <= 0.0 {
        return none;
    }
    let used = last.credits_used - first.credits_used;
    WindowBurn {
        window_hours: hours,
        credits_used: Some(used),
        per_hour: Some(used / span),
    }
}

fn confidence(samples: &[BurnSample]) -> Confidence {
    let span = match (samples.first(), samples.last()) {
        (Some(a), Some(b)) => b.ts - a.ts,
        _ => Duration::zero(),
    };
    if samples.len() < LOW_CONFIDENCE_SAMPLES || span < Duration::minutes(LOW_CONFIDENCE_MINUTES) {
        Confidence::Low
    } else if span < Duration::hours(HIGH_CONFIDENCE_HOURS) {
        Confidence::Medium
    } else {
        Confidence::High
    }
}

/// `samples` must be chronological. The live quota counts as a final point when it is newer.
pub fn compute_burn(
    samples: &[BurnSample],
    quota: &CopilotQuota,
    now: DateTime<Utc>,
) -> BurnReport {
    let mut points = samples.to_vec();
    if points.last().is_none_or(|s| s.ts < quota.fetched_at) {
        points.push(BurnSample {
            ts: quota.fetched_at,
            credits_used: quota.credits_used,
        });
    }
    let windows: Vec<WindowBurn> = WINDOWS_HOURS
        .iter()
        .map(|h| window_burn(&points, now, *h))
        .collect();
    // 6h first, then 24h, then 1h: the longer windows are steadier.
    let rate = [6, 24, 1]
        .iter()
        .filter_map(|h| windows.iter().find(|w| w.window_hours == *h)?.per_hour)
        .next();
    let hours_to_reset = quota.reset_at.map(|r| hours_between(now, r).max(0.0));
    let projected_at_reset = rate
        .zip(hours_to_reset)
        .map(|(r, h)| quota.credits_used + r.max(0.0) * h);
    let hours_left = rate
        .filter(|r| *r > 0.0)
        .map(|r| quota.remaining.max(0.0) / r);
    let projected_exhaustion_at = hours_left.map(|h| now + Duration::seconds((h * 3600.0) as i64));
    let exhausts_before_reset = hours_left
        .zip(hours_to_reset)
        .is_some_and(|(left, to_reset)| left < to_reset);
    BurnReport {
        windows,
        projected_at_reset,
        hours_to_reset,
        projected_exhaustion_at,
        exhausts_before_reset,
        overage_permitted: quota.overage_permitted,
        sample_count: points.len(),
        confidence: confidence(&points),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(min: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-30T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::minutes(min)
    }

    fn s(min: i64, used: f64) -> BurnSample {
        BurnSample {
            ts: t(min),
            credits_used: used,
        }
    }

    fn quota(used: f64, at_min: i64) -> CopilotQuota {
        CopilotQuota {
            login: None,
            plan: None,
            sku: None,
            token_based_billing: true,
            entitlement: 1000.0,
            credits_used: used,
            remaining: 1000.0 - used,
            percent_remaining: 0.0,
            overage_permitted: true,
            unlimited: false,
            has_quota: true,
            reset_at: Some(t(at_min) + Duration::hours(10)),
            fetched_at: t(at_min),
        }
    }

    fn rate(r: &BurnReport, hours: i64) -> Option<f64> {
        r.windows.iter().find(|w| w.window_hours == hours)?.per_hour
    }

    #[test]
    fn steady_burn_projects_to_reset_and_exhaustion() {
        let samples: Vec<_> = (0..=6)
            .map(|i| s(i * 60, 100.0 + 50.0 * i as f64))
            .collect();
        let r = compute_burn(&samples, &quota(400.0, 360), t(360));
        assert!((rate(&r, 6).unwrap() - 50.0).abs() < 1e-9);
        assert!((r.projected_at_reset.unwrap() - 900.0).abs() < 1e-6);
        assert!((r.hours_to_reset.unwrap() - 10.0).abs() < 1e-9);
        // 600 left at 50/h is 12h, after the 10h reset.
        assert!(!r.exhausts_before_reset && r.projected_exhaustion_at.is_some());
        assert_eq!(r.confidence, Confidence::High);
    }

    #[test]
    fn exhaustion_before_reset_is_flagged() {
        let samples = vec![s(0, 0.0), s(30, 100.0), s(60, 200.0)];
        let mut q = quota(200.0, 60);
        q.remaining = 300.0;
        let r = compute_burn(&samples, &q, t(60));
        assert!(r.exhausts_before_reset);
        assert!(r.overage_permitted);
    }

    #[test]
    fn a_cycle_reset_restarts_the_window_after_the_drop() {
        let samples = vec![s(0, 900.0), s(60, 990.0), s(120, 5.0), s(180, 35.0)];
        let r = compute_burn(&samples, &quota(35.0, 180), t(180));
        assert!((rate(&r, 6).unwrap() - 30.0).abs() < 1e-9);
        assert_eq!(r.windows[1].credits_used, Some(30.0));
    }

    #[test]
    fn sparse_history_has_low_confidence_and_no_rates() {
        let r = compute_burn(&[], &quota(10.0, 0), t(0));
        assert_eq!(r.confidence, Confidence::Low);
        assert!(r.windows.iter().all(|w| w.per_hour.is_none()));
        assert!(r.projected_at_reset.is_none() && r.projected_exhaustion_at.is_none());
        let r = compute_burn(&[s(0, 10.0), s(10, 12.0)], &quota(12.0, 10), t(10));
        assert_eq!(r.confidence, Confidence::Low);
        assert!(rate(&r, 1).is_some());
    }

    #[test]
    fn zero_rate_never_exhausts() {
        let samples = vec![s(0, 50.0), s(60, 50.0), s(120, 50.0)];
        let r = compute_burn(&samples, &quota(50.0, 120), t(120));
        assert_eq!(rate(&r, 1), Some(0.0));
        assert_eq!(r.projected_at_reset, Some(50.0));
        assert!(r.projected_exhaustion_at.is_none() && !r.exhausts_before_reset);
    }

    #[test]
    fn falls_back_to_shorter_window_when_six_hour_is_empty() {
        // Only the last hour has two samples; older sample is outside 6h.
        let samples = vec![s(0, 0.0), s(600, 100.0), s(630, 130.0)];
        let r = compute_burn(&samples, &quota(130.0, 630), t(630));
        assert!(rate(&r, 24).is_some());
        assert!((rate(&r, 1).unwrap() - 60.0).abs() < 1e-9);
    }
}
