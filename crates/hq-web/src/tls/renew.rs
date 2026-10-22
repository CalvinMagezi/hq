//! When to ask the CA for a certificate, and what to do when it says no. Pure functions of
//! the clock, so the schedule is tested without waiting.

use std::time::Duration;

/// Renew once this fraction of a certificate's life has passed. A fraction, not "30 days
/// before the end": CAs are shortening lifetimes, and the same rule has to work for 90 days,
/// 45 days and 6.
const RENEW_NUMERATOR: i64 = 2;
const RENEW_DENOMINATOR: i64 = 3;
/// The loop wakes at least this often, so a clock change or a replaced file is noticed.
pub(crate) const MAX_SLEEP: Duration = Duration::from_secs(6 * 60 * 60);
/// After a failed order: 15 minutes, then longer. The CA allows 5 failed validations per name
/// per hour, so the first retry must not come sooner than the limit refills.
const BACKOFF: [Duration; 6] = [
    Duration::from_secs(15 * 60),
    Duration::from_secs(30 * 60),
    Duration::from_secs(60 * 60),
    Duration::from_secs(2 * 60 * 60),
    Duration::from_secs(4 * 60 * 60),
    Duration::from_secs(6 * 60 * 60),
];
/// A rate limit that names no time is waited out this long.
const RATE_LIMIT_DEFAULT_WAIT: Duration = Duration::from_secs(6 * 60 * 60);
const RATE_LIMIT_MIN_WAIT: Duration = Duration::from_secs(60);
/// CAs' own limits refill within a week (Let's Encrypt: 7 days); longer is a mistake.
const RATE_LIMIT_MAX_WAIT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The moment a certificate valid from `not_before` to `not_after` is due for renewal.
pub(crate) fn renew_at(not_before: i64, not_after: i64) -> i64 {
    let life = (not_after - not_before).max(0);
    not_before + life * RENEW_NUMERATOR / RENEW_DENOMINATOR
}

/// What the loop does next.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// Order a certificate now.
    Now,
    /// Nothing to do for this long.
    Wait(Duration),
}

/// `cert` is the installed certificate's validity, if there is one; `not_before_retry` is the
/// earliest time a new attempt is allowed after a failure or a rate limit.
pub(crate) fn decide(now: i64, cert: Option<(i64, i64)>, not_before_retry: Option<i64>) -> Next {
    let due = match cert {
        None => now,
        Some((from, to)) => renew_at(from, to),
    };
    let at = due.max(not_before_retry.unwrap_or(i64::MIN));
    if at <= now {
        return Next::Now;
    }
    let wait = Duration::from_secs(u64::try_from(at - now).unwrap_or(0));
    Next::Wait(wait.min(MAX_SLEEP))
}

/// How long to wait after the `failures`-th failure in a row (the first is 1).
pub(crate) fn backoff(failures: u32) -> Duration {
    let i = usize::try_from(failures.saturating_sub(1)).unwrap_or(usize::MAX);
    BACKOFF[i.min(BACKOFF.len() - 1)]
}

/// How long a rate limit holds us, from the CA's `Retry-After` when it sent one.
pub(crate) fn rate_limit_wait(retry_after: Option<Duration>) -> Duration {
    retry_after
        .unwrap_or(RATE_LIMIT_DEFAULT_WAIT)
        .clamp(RATE_LIMIT_MIN_WAIT, RATE_LIMIT_MAX_WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 60 * 60;

    #[test]
    fn renewal_is_due_two_thirds_through_whatever_the_lifetime() {
        assert_eq!(renew_at(0, 90 * DAY), 60 * DAY);
        assert_eq!(renew_at(1000, 1000 + 45 * DAY), 1000 + 30 * DAY);
        assert_eq!(renew_at(0, 6 * DAY), 4 * DAY);
        assert_eq!(renew_at(500, 500), 500);
        assert_eq!(
            renew_at(900, 100),
            900,
            "a window that ends before it starts is due at once"
        );
    }

    #[test]
    fn with_no_certificate_the_first_order_is_now_unless_a_failure_holds_it() {
        assert_eq!(decide(100, None, None), Next::Now);
        assert_eq!(decide(100, None, Some(100)), Next::Now);
        assert_eq!(
            decide(100, None, Some(160)),
            Next::Wait(Duration::from_secs(60))
        );
    }

    #[test]
    fn a_healthy_certificate_sleeps_until_it_is_due_and_no_longer_than_the_cap() {
        let cert = Some((0, 90 * DAY));
        assert_eq!(decide(DAY, cert, None), Next::Wait(MAX_SLEEP));
        assert_eq!(
            decide(60 * DAY - 120, cert, None),
            Next::Wait(Duration::from_secs(120))
        );
        assert_eq!(decide(60 * DAY, cert, None), Next::Now);
        assert_eq!(decide(80 * DAY, cert, None), Next::Now);
    }

    #[test]
    fn a_due_certificate_still_waits_out_a_rate_limit_and_keeps_serving() {
        let cert = Some((0, 90 * DAY));
        let held_until = 70 * DAY;
        assert_eq!(
            decide(61 * DAY, cert, Some(held_until)),
            Next::Wait(MAX_SLEEP)
        );
        assert_eq!(decide(70 * DAY, cert, Some(held_until)), Next::Now);
    }

    #[test]
    fn failures_back_off_and_stop_growing() {
        assert_eq!(backoff(1), Duration::from_secs(15 * 60));
        assert_eq!(backoff(2), Duration::from_secs(30 * 60));
        assert_eq!(backoff(6), Duration::from_secs(6 * 60 * 60));
        assert_eq!(backoff(500), Duration::from_secs(6 * 60 * 60));
        assert!(
            backoff(1) >= Duration::from_secs(12 * 60),
            "above the refill of 5 failures an hour"
        );
    }

    #[test]
    fn a_rate_limit_honours_retry_after_within_sane_bounds() {
        assert_eq!(rate_limit_wait(None), RATE_LIMIT_DEFAULT_WAIT);
        assert_eq!(
            rate_limit_wait(Some(Duration::from_secs(3 * 60 * 60))),
            Duration::from_secs(3 * 60 * 60)
        );
        assert_eq!(rate_limit_wait(Some(Duration::ZERO)), RATE_LIMIT_MIN_WAIT);
        assert_eq!(
            rate_limit_wait(Some(Duration::from_secs(365 * 24 * 60 * 60))),
            RATE_LIMIT_MAX_WAIT
        );
    }
}
