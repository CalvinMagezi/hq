//! Where spend is heading, from HQ's own record of it. Pure: callers pass the events and the clock,
//! so every rule here is testable without a database. `copilot_burn` does the same job for a
//! provider that reports a running counter; this one works from incremental spend, which is what
//! the ledger has for every provider.

use chrono::{DateTime, Datelike, Duration, Utc};
use serde::Serialize;

use crate::copilot_burn::Confidence;

pub const WINDOWS_HOURS: [i64; 4] = [1, 6, 24, 168];
/// A weekday profile needs at least this many days of history; before that the rate is flat.
const SEASONALITY_MIN_DAYS: f64 = 14.0;
const LOW_CONFIDENCE_EVENTS: usize = 3;
const LOW_CONFIDENCE_HOURS: f64 = 6.0;
const HIGH_CONFIDENCE_HOURS: f64 = 72.0;
const HOURS_PER_DAY: f64 = 24.0;
const SECS_PER_HOUR: f64 = 3600.0;

/// Dollars spent at one moment. The ledger hands these over bucketed by hour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpendEvent {
    pub ts: DateTime<Utc>,
    pub usd: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WindowRate {
    pub window_hours: i64,
    pub usd: f64,
    pub per_hour: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Forecast {
    pub windows: Vec<WindowRate>,
    /// The rate the projection uses, dollars per hour. `None` with no history at all.
    pub rate_per_hour: Option<f64>,
    pub spent_this_period: f64,
    pub hours_to_reset: f64,
    /// Spend at the end of the period if the rate holds (weekday-shaped once there is enough history).
    pub projected_period_total: Option<f64>,
    pub limit_usd: Option<f64>,
    pub hours_to_limit: Option<f64>,
    pub projected_exhaustion_at: Option<DateTime<Utc>>,
    pub exhausts_before_reset: bool,
    pub confidence: Confidence,
    pub history_hours: f64,
    /// Why the projection is flat rather than weekday-shaped, when it is.
    pub seasonality_note: Option<String>,
}

fn hours(a: DateTime<Utc>, b: DateTime<Utc>) -> f64 {
    (b - a).num_seconds() as f64 / SECS_PER_HOUR
}

fn spend_since(events: &[SpendEvent], since: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    events
        .iter()
        .filter(|e| e.ts > since && e.ts <= now)
        .map(|e| e.usd)
        .sum()
}

/// Average spend per hour for each weekday (Monday first) over whole days of history, or `None`
/// when a weekday has never been seen.
fn weekday_hourly(events: &[SpendEvent], first: DateTime<Utc>, now: DateTime<Utc>) -> [Option<f64>; 7] {
    let mut spent = [0.0f64; 7];
    let mut days = [0u32; 7];
    let mut day = first.date_naive();
    while day < now.date_naive() {
        days[day.weekday().num_days_from_monday() as usize] += 1;
        day = day.succ_opt().unwrap_or(now.date_naive());
    }
    for e in events.iter().filter(|e| e.ts.date_naive() < now.date_naive()) {
        spent[e.ts.weekday().num_days_from_monday() as usize] += e.usd;
    }
    std::array::from_fn(|i| (days[i] > 0).then(|| spent[i] / f64::from(days[i]) / HOURS_PER_DAY))
}

/// Spend expected between `now` and `end` using each weekday's own hourly average.
fn weekday_projection(profile: &[Option<f64>; 7], fallback: f64, now: DateTime<Utc>, end: DateTime<Utc>) -> f64 {
    let mut total = 0.0;
    let mut cursor = now;
    while cursor < end {
        let next = (cursor.date_naive().succ_opt().map_or(end, |d| {
            d.and_hms_opt(0, 0, 0).map_or(end, |t| t.and_utc())
        }))
        .min(end);
        let rate = profile[cursor.weekday().num_days_from_monday() as usize].unwrap_or(fallback);
        total += rate * hours(cursor, next);
        cursor = next;
    }
    total
}

fn confidence(events: &[SpendEvent], history_hours: f64) -> Confidence {
    if events.len() < LOW_CONFIDENCE_EVENTS || history_hours < LOW_CONFIDENCE_HOURS {
        Confidence::Low
    } else if history_hours < HIGH_CONFIDENCE_HOURS {
        Confidence::Medium
    } else {
        Confidence::High
    }
}

/// `events` are spend increments in any order. `limit_usd` is the budget or balance being
/// consumed, measured from the same period as `spent_this_period`.
pub fn forecast(
    events: &[SpendEvent],
    spent_this_period: f64,
    period_end: DateTime<Utc>,
    limit_usd: Option<f64>,
    now: DateTime<Utc>,
) -> Forecast {
    let first = events.iter().map(|e| e.ts).min();
    let history_hours = first.map_or(0.0, |f| hours(f, now).max(0.0));
    let windows: Vec<WindowRate> = WINDOWS_HOURS
        .iter()
        .map(|w| {
            let usd = spend_since(events, now - Duration::hours(*w), now);
            // Only the part of the window that has history counts, so a day-old install is not
            // averaged over a week.
            let span = history_hours.min(*w as f64).max(f64::EPSILON);
            WindowRate {
                window_hours: *w,
                usd,
                per_hour: if history_hours <= 0.0 { 0.0 } else { usd / span },
            }
        })
        .collect();
    // A day is long enough to smooth a burst and short enough to follow a change.
    let rate = (history_hours > 0.0)
        .then(|| {
            windows
                .iter()
                .find(|w| w.window_hours == 24)
                .map_or(0.0, |w| w.per_hour)
        });
    let hours_to_reset = hours(now, period_end).max(0.0);
    let seasonal = first.filter(|f| hours(*f, now) / HOURS_PER_DAY >= SEASONALITY_MIN_DAYS);
    let seasonality_note = match (rate, seasonal) {
        (None, _) => None,
        (Some(_), Some(_)) => None,
        (Some(_), None) => Some(format!(
            "Needs {SEASONALITY_MIN_DAYS:.0} days of history to follow the weekly pattern; the projection assumes today's rate holds."
        )),
    };
    let projected_period_total = rate.map(|r| {
        let remaining = match seasonal {
            Some(f) => {
                let profile = weekday_hourly(events, f, now);
                weekday_projection(&profile, r, now, period_end)
            }
            None => r * hours_to_reset,
        };
        spent_this_period + remaining
    });
    let hours_to_limit = limit_usd.zip(rate).and_then(|(limit, r)| {
        (r > 0.0).then(|| ((limit - spent_this_period).max(0.0)) / r)
    });
    let exhausts_before_reset = hours_to_limit.is_some_and(|h| h < hours_to_reset);
    Forecast {
        windows,
        rate_per_hour: rate,
        spent_this_period,
        hours_to_reset,
        projected_period_total,
        limit_usd,
        hours_to_limit,
        projected_exhaustion_at: hours_to_limit
            .map(|h| now + Duration::seconds((h * SECS_PER_HOUR) as i64)),
        exhausts_before_reset,
        confidence: confidence(events, history_hours),
        history_hours,
        seasonality_note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// 2026-10-14 is a Wednesday.
    const NOW: &str = "2026-10-14T12:00:00Z";

    fn hourly(now: DateTime<Utc>, hours_back: i64, usd: f64) -> Vec<SpendEvent> {
        (0..hours_back)
            .map(|h| SpendEvent {
                ts: now - Duration::hours(h),
                usd,
            })
            .collect()
    }

    fn rate(f: &Forecast, hours: i64) -> f64 {
        f.windows.iter().find(|w| w.window_hours == hours).unwrap().per_hour
    }

    #[test]
    fn a_steady_rate_projects_the_period_total_and_the_hour_the_limit_is_hit() {
        let now = at(NOW);
        let events = hourly(now, 48, 0.5);
        let f = forecast(&events, 10.0, at("2026-11-01T00:00:00Z"), Some(20.0), now);
        assert!((rate(&f, 24) - 0.5).abs() < 1e-9);
        assert!((f.hours_to_limit.unwrap() - 20.0).abs() < 1e-9);
        assert!(f.exhausts_before_reset);
        let remaining_hours = hours(now, at("2026-11-01T00:00:00Z"));
        assert!((f.projected_period_total.unwrap() - (10.0 + 0.5 * remaining_hours)).abs() < 1e-6);
        assert_eq!(f.confidence, Confidence::Medium);
        assert!(f.seasonality_note.unwrap().contains("14 days"));
    }

    #[test]
    fn no_history_means_no_projection_and_low_confidence() {
        let now = at(NOW);
        let f = forecast(&[], 0.0, at("2026-11-01T00:00:00Z"), Some(5.0), now);
        assert!(f.rate_per_hour.is_none() && f.projected_period_total.is_none());
        assert!(f.hours_to_limit.is_none() && !f.exhausts_before_reset);
        assert_eq!(f.confidence, Confidence::Low);
    }

    #[test]
    fn a_young_history_is_averaged_over_its_own_length_not_the_whole_window() {
        let now = at(NOW);
        let f = forecast(&hourly(now, 3, 2.0), 6.0, at("2026-10-15T00:00:00Z"), None, now);
        // Three events over about two hours of history, not spread across 24 or 168 hours.
        assert!(rate(&f, 24) > 2.0, "{}", rate(&f, 24));
        assert_eq!(f.confidence, Confidence::Low);
    }

    #[test]
    fn zero_spend_never_exhausts() {
        let now = at(NOW);
        let events = vec![SpendEvent { ts: now - Duration::hours(30), usd: 0.0 }; 5];
        let f = forecast(&events, 0.0, at("2026-11-01T00:00:00Z"), Some(5.0), now);
        assert_eq!(f.rate_per_hour, Some(0.0));
        assert!(f.hours_to_limit.is_none() && !f.exhausts_before_reset);
    }

    #[test]
    fn with_two_weeks_of_history_the_projection_follows_the_weekday_pattern() {
        // Spend only on Thursdays: $24 each. Now is Wednesday noon, so the rest of this week holds
        // one Thursday, and a flat projection from today's zero rate would say nothing is coming.
        let now = at(NOW);
        let mut events = Vec::new();
        for day in 1..=20 {
            let d = now.date_naive() - chrono::Days::new(day);
            if d.weekday() == chrono::Weekday::Thu {
                events.push(SpendEvent { ts: d.and_hms_opt(10, 0, 0).unwrap().and_utc(), usd: 24.0 });
            }
        }
        events.push(SpendEvent { ts: now - Duration::days(20), usd: 0.0 });
        let end = at("2026-10-18T00:00:00Z");
        let f = forecast(&events, 0.0, end, None, now);
        assert!(f.seasonality_note.is_none());
        let projected = f.projected_period_total.unwrap();
        assert!((projected - 24.0).abs() < 1.0, "{projected}");
    }
}
