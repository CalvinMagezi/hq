//! When a push may be sent: quiet hours, one non-urgent push an hour, a daily budget for the
//! non-urgent ones. An approval, a `notify_owner` message and a due reminder are urgent and are
//! never held back by the budget or the hourly fold; only quiet hours hold them, until the
//! quiet period ends. Everything held back or folded stays in the inbox.

use chrono::{DateTime, Duration, NaiveTime, Timelike, Utc};
use std::sync::Mutex;

/// Non-urgent pushes fold into one per this many minutes.
const FOLD_MINUTES: i64 = 60;

/// What to do with one push the dispatcher wants to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Send,
    /// Not now: leave it unclaimed so a later pass sends it (quiet hours).
    Hold,
    /// Never pushed: it is in the inbox, and was folded into an earlier push or over budget.
    Fold,
}

pub(crate) struct Gate {
    /// Quiet window as minutes after midnight UTC: start and end. `start > end` wraps midnight.
    quiet: Option<(u32, u32)>,
    daily_budget: u32,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    last_non_urgent: Option<DateTime<Utc>>,
    day: Option<chrono::NaiveDate>,
    sent_today: u32,
}

impl Gate {
    pub(crate) fn new(quiet_hours_utc: Option<&str>, daily_budget: u32) -> Self {
        Self {
            quiet: quiet_hours_utc.and_then(parse_window),
            daily_budget,
            state: Mutex::new(State::default()),
        }
    }

    /// How far back the dispatcher must look so an item raised at the start of the quiet
    /// period is still found when it ends.
    pub(crate) fn extra_lookback(&self) -> Duration {
        match self.quiet {
            Some((start, end)) => {
                let minutes = (i64::from(end) - i64::from(start)).rem_euclid(24 * 60);
                Duration::minutes(minutes)
            }
            None => Duration::zero(),
        }
    }

    pub(crate) fn decide(&self, now: DateTime<Utc>, urgent: bool) -> Decision {
        if self.in_quiet_hours(now) {
            return Decision::Hold;
        }
        if urgent {
            return Decision::Send;
        }
        let mut state = self.state.lock().expect("push gate lock poisoned");
        if state.day != Some(now.date_naive()) {
            state.day = Some(now.date_naive());
            state.sent_today = 0;
        }
        let folded = state
            .last_non_urgent
            .is_some_and(|at| now - at < Duration::minutes(FOLD_MINUTES));
        if folded || state.sent_today >= self.daily_budget {
            return Decision::Fold;
        }
        state.last_non_urgent = Some(now);
        state.sent_today += 1;
        Decision::Send
    }

    fn in_quiet_hours(&self, now: DateTime<Utc>) -> bool {
        let Some((start, end)) = self.quiet else {
            return false;
        };
        let minute = now.hour() * 60 + now.minute();
        if start <= end {
            (start..end).contains(&minute)
        } else {
            minute >= start || minute < end
        }
    }
}

/// `"22:00-07:00"` (UTC) as minutes after midnight, or `None` when it is not that.
fn parse_window(raw: &str) -> Option<(u32, u32)> {
    let (a, b) = raw.split_once('-')?;
    let minutes = |t: &str| {
        let t = NaiveTime::parse_from_str(t.trim(), "%H:%M").ok()?;
        Some(t.hour() * 60 + t.minute())
    };
    let (start, end) = (minutes(a)?, minutes(b)?);
    (start != end).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2030, 1, 1, h, m, 0).unwrap()
    }

    #[test]
    fn a_burst_of_ten_non_urgent_items_is_one_push() {
        let gate = Gate::new(None, 5);
        let sent = (0..10)
            .filter(|i| gate.decide(at(10, *i), false) == Decision::Send)
            .count();
        assert_eq!(sent, 1);
        assert_eq!(
            gate.decide(at(11, 5), false),
            Decision::Send,
            "an hour later"
        );
    }

    #[test]
    fn the_daily_budget_caps_non_urgent_pushes_but_not_urgent_ones() {
        let gate = Gate::new(None, 2);
        let sent = (0..6)
            .filter(|h| gate.decide(at(8 + h * 2, 0), false) == Decision::Send)
            .count();
        assert_eq!(sent, 2);
        assert_eq!(gate.decide(at(23, 0), true), Decision::Send);
    }

    #[test]
    fn an_urgent_approval_goes_at_once_outside_quiet_hours_and_waits_inside_them() {
        let gate = Gate::new(Some("22:00-07:00"), 5);
        assert_eq!(gate.decide(at(12, 0), true), Decision::Send);
        assert_eq!(gate.decide(at(23, 30), true), Decision::Hold);
        assert_eq!(gate.decide(at(3, 0), true), Decision::Hold);
        assert_eq!(gate.decide(at(7, 0), true), Decision::Send);
        assert_eq!(gate.extra_lookback(), Duration::minutes(9 * 60));
    }

    #[test]
    fn a_bad_window_means_no_quiet_hours() {
        for raw in ["", "22:00", "25:00-07:00", "08:00-08:00", "x-y"] {
            assert_eq!(
                Gate::new(Some(raw), 5).decide(at(3, 0), true),
                Decision::Send,
                "{raw}"
            );
        }
    }
}
