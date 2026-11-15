//! How fast a window is being used, from its own reset boundary. Pure: no clock, no I/O.

use chrono::{DateTime, Duration, Months, Utc};
use hq_core::quota::{QuotaWindow, WindowKind};
use serde::Serialize;

const FIVE_HOUR_WINDOW_HOURS: i64 = 5;
const WEEK_WINDOW_DAYS: i64 = 7;
/// Below this share of the window gone, a projection is noise: one early request would read as
/// a runaway. The share is raised to this before dividing.
const MIN_ELAPSED_FRACTION: f64 = 0.05;
/// A projection past this is shown as this, so a first-minute reading stays readable.
const PROJECTED_CAP_PERCENT: f64 = 999.0;
const PERCENT: f64 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaceStatus {
    OnPace,
    OverPace,
    OverPaceHard,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Pace {
    /// Used percent at the reset if use goes on at the rate so far.
    pub projected_percent: f64,
    /// How much of the window had gone when it was read.
    pub elapsed_percent: f64,
    pub status: PaceStatus,
}

/// When a window began, from its reset. Monthly, credit and dollar windows are the calendar
/// month before the reset (every reader reports those on a monthly cycle).
pub fn window_start(kind: WindowKind, resets_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match kind {
        WindowKind::FiveHour => Some(resets_at - Duration::hours(FIVE_HOUR_WINDOW_HOURS)),
        WindowKind::Weekly => Some(resets_at - Duration::days(WEEK_WINDOW_DAYS)),
        WindowKind::Monthly | WindowKind::Credits | WindowKind::Dollars => {
            resets_at.checked_sub_months(Months::new(1))
        }
    }
}

/// The pace of `window` as read at `read_at`. `None` when the reading lacks a used percentage
/// or a reset, or the window had already reset: nothing can be said, and nothing is guessed.
pub fn pace(
    window: &QuotaWindow,
    read_at: DateTime<Utc>,
    warn_percent: f64,
    block_percent: f64,
) -> Option<Pace> {
    let used = window.used_percent?;
    let resets_at = window.resets_at?;
    let start = window_start(window.kind, resets_at)?;
    let length = (resets_at - start).num_seconds();
    if length <= 0 || read_at >= resets_at {
        return None;
    }
    let gone = (read_at - start).num_seconds().clamp(0, length) as f64 / length as f64;
    let projected = (used / gone.max(MIN_ELAPSED_FRACTION)).min(PROJECTED_CAP_PERCENT);
    let status = if projected >= block_percent {
        PaceStatus::OverPaceHard
    } else if projected >= warn_percent {
        PaceStatus::OverPace
    } else {
        PaceStatus::OnPace
    };
    Some(Pace {
        projected_percent: projected,
        elapsed_percent: gone * PERCENT,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const WARN: f64 = 80.0;
    const BLOCK: f64 = 95.0;

    fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    fn window(kind: WindowKind, used: Option<f64>, resets: Option<DateTime<Utc>>) -> QuotaWindow {
        QuotaWindow {
            label: "w".into(),
            kind,
            used_percent: used,
            remaining: None,
            resets_at: resets,
        }
    }

    #[test]
    fn five_hour_window_projects_from_the_share_gone() {
        let reset = at(2026, 11, 15, 10);
        let w = window(WindowKind::FiveHour, Some(40.0), Some(reset));
        // 5h window read 2.5h in: half gone, so 40 used projects to 80, the warn line.
        let p = pace(&w, reset - Duration::minutes(150), WARN, BLOCK).unwrap();
        assert!((p.projected_percent - 80.0).abs() < 1e-6);
        assert_eq!(p.status, PaceStatus::OverPace);
        let calm = window(WindowKind::FiveHour, Some(30.0), Some(reset));
        let p = pace(&calm, reset - Duration::minutes(150), WARN, BLOCK).unwrap();
        assert_eq!(p.status, PaceStatus::OnPace);
    }

    #[test]
    fn weekly_window_far_ahead_of_pace_is_hard() {
        let reset = at(2026, 11, 20, 0);
        let w = window(WindowKind::Weekly, Some(60.0), Some(reset));
        // Two days of seven gone, 60 used.
        let p = pace(&w, reset - Duration::days(5), WARN, BLOCK).unwrap();
        assert_eq!(p.status, PaceStatus::OverPaceHard);
        assert!(p.projected_percent > BLOCK);
    }

    #[test]
    fn monthly_start_is_the_calendar_month_before_the_reset() {
        let reset = at(2026, 12, 1, 0);
        assert_eq!(
            window_start(WindowKind::Monthly, reset),
            Some(at(2026, 11, 1, 0))
        );
        // A 28-day February is not treated as 30 days.
        assert_eq!(
            window_start(WindowKind::Dollars, at(2027, 3, 1, 0)),
            Some(at(2027, 2, 1, 0))
        );
        let w = window(WindowKind::Monthly, Some(50.0), Some(reset));
        // Half of November gone (15 of 30 days), 50 used: lands at 100 percent.
        let p = pace(&w, at(2026, 11, 16, 0), WARN, BLOCK).unwrap();
        assert!((p.projected_percent - 100.0).abs() < 1e-6);
        assert_eq!(p.status, PaceStatus::OverPaceHard);
    }

    #[test]
    fn early_readings_do_not_explode() {
        let reset = at(2026, 11, 20, 0);
        let w = window(WindowKind::Weekly, Some(3.0), Some(reset));
        let p = pace(
            &w,
            reset - Duration::days(7) + Duration::minutes(1),
            WARN,
            BLOCK,
        )
        .unwrap();
        assert!((p.projected_percent - 60.0).abs() < 1e-6);
        assert_eq!(p.status, PaceStatus::OnPace);
    }

    #[test]
    fn missing_or_passed_inputs_give_no_pace() {
        let reset = at(2026, 11, 20, 0);
        let now = reset - Duration::days(1);
        assert!(
            pace(
                &window(WindowKind::Weekly, None, Some(reset)),
                now,
                WARN,
                BLOCK
            )
            .is_none()
        );
        assert!(
            pace(
                &window(WindowKind::Weekly, Some(10.0), None),
                now,
                WARN,
                BLOCK
            )
            .is_none()
        );
        let passed = window(WindowKind::Weekly, Some(10.0), Some(reset));
        assert!(pace(&passed, reset, WARN, BLOCK).is_none());
    }
}
