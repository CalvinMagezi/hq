//! A limit on how many datagrams the session may turn away. The UDP socket is connected to the
//! hub, so the kernel already drops everything from another source. A sender that uses the hub's
//! own address and port gets through that filter, and every datagram it sends has to be read and
//! refused: at hundreds of thousands a second that is a whole core, and on a phone a battery.
//!
//! A healthy hub causes a few refusals a minute (a repeated timestamp, a packet for a session that
//! has gone). So refusals are charged against a bucket that refills slowly; when it runs dry the
//! socket is left alone for as long as the debt takes to pay off, and the kernel's buffer drops
//! what arrives meanwhile. Datagrams the session accepts cost nothing.

use std::time::{Duration, Instant};

/// Refusals the bucket refills per second. Far above anything a hub causes, far below a flood.
const REFUSALS_PER_SECOND: f64 = 2_000.0;
/// What the bucket holds: a few passes of the receive loop, so a short burst is not slowed.
const BURST: f64 = 512.0;
/// Reading resumes when the bucket holds a whole receive pass again. Stopping for the debt alone
/// would stop after every datagram, and the loop would wake as often as the flood sends.
const RESUME_AT: f64 = 128.0;

pub(crate) struct RefusalBudget {
    tokens: f64,
    updated: Instant,
    paused_until: Option<Instant>,
    /// How many times reading was stopped.
    pub(crate) pauses: u64,
}

impl RefusalBudget {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            tokens: BURST,
            updated: now,
            paused_until: None,
            pauses: 0,
        }
    }

    /// Whether the socket is being left alone at `now`.
    pub(crate) fn paused(&self, now: Instant) -> bool {
        self.paused_until.is_some_and(|until| now < until)
    }

    /// How long until reading may resume, if it is stopped.
    pub(crate) fn resumes_in(&self, now: Instant) -> Option<Duration> {
        self.paused_until
            .map(|until| until.saturating_duration_since(now))
            .filter(|left| !left.is_zero())
    }

    /// Charges `refused` datagrams turned away at `now`. Returns whether reading just stopped.
    pub(crate) fn charge(&mut self, now: Instant, refused: usize) -> bool {
        let elapsed = now.saturating_duration_since(self.updated).as_secs_f64();
        self.updated = now;
        self.tokens = (self.tokens + elapsed * REFUSALS_PER_SECOND).min(BURST);
        self.tokens -= refused as f64;
        if self.tokens >= 0.0 || self.paused(now) {
            return false;
        }
        let debt = Duration::from_secs_f64((RESUME_AT - self.tokens) / REFUSALS_PER_SECOND);
        self.paused_until = Some(now + debt);
        self.pauses += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    #[test]
    fn a_burst_inside_the_bucket_is_not_slowed() {
        let t0 = Instant::now();
        let mut budget = RefusalBudget::new(t0);
        assert!(!budget.charge(t0, BURST as usize));
        assert!(!budget.paused(t0));
        assert_eq!(budget.pauses, 0);
    }

    #[test]
    fn going_over_stops_reading_for_as_long_as_the_debt_takes_to_pay() {
        let t0 = Instant::now();
        let mut budget = RefusalBudget::new(t0);
        assert!(budget.charge(t0, BURST as usize + 200));
        assert!(budget.paused(t0));
        // 200 over and a whole pass of 128 to have again, at 2,000 a second: 164 ms.
        let left = budget.resumes_in(t0).unwrap();
        assert!(
            left > Duration::from_millis(160) && left <= Duration::from_millis(164),
            "{left:?}"
        );
        assert!(budget.paused(after(t0, 150)));
        assert!(!budget.paused(after(t0, 165)));
        assert!(budget.resumes_in(after(t0, 165)).is_none());
        assert_eq!(budget.pauses, 1);
    }

    #[test]
    fn accepted_datagrams_cost_nothing_and_the_bucket_refills() {
        let t0 = Instant::now();
        let mut budget = RefusalBudget::new(t0);
        for pass in 0..10_000 {
            assert!(!budget.charge(after(t0, pass), 0));
        }
        assert!(budget.charge(after(t0, 10_000), BURST as usize * 2));
        // A second later the bucket is whole again.
        assert!(!budget.paused(after(t0, 12_000)));
        assert!(!budget.charge(after(t0, 12_000), BURST as usize));
    }

    #[test]
    fn a_flood_is_held_to_the_refill_rate_whatever_it_sends() {
        let t0 = Instant::now();
        let mut budget = RefusalBudget::new(t0);
        let mut read = 0usize;
        // A read pass takes 128 datagrams whenever reading is allowed; the clock moves in 1 ms steps.
        for ms in 0..10_000u64 {
            let now = after(t0, ms);
            if !budget.paused(now) {
                read += 128;
                budget.charge(now, 128);
            }
        }
        let allowed = BURST as usize + (REFUSALS_PER_SECOND as usize) * 10;
        assert!(read <= allowed + 256, "{read} read, {allowed} allowed");
        assert!(
            read >= allowed - 2_000,
            "{read} read: it should not be starved either"
        );
        assert!(budget.pauses > 10);
    }

    #[test]
    fn charging_while_paused_does_not_count_a_second_pause() {
        let t0 = Instant::now();
        let mut budget = RefusalBudget::new(t0);
        assert!(budget.charge(t0, 2_000));
        assert!(!budget.charge(after(t0, 1), 10));
        assert_eq!(budget.pauses, 1);
    }
}
