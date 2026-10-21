//! When the WireGuard timers next need to run. boringtun rounds its own timers to a second, so
//! the useful granularity is a second while something is happening and the keepalive interval
//! when nothing is. An idle tunnel wakes once per keepalive, not once a second, and one that is
//! waiting for a hub that does not answer backs off instead of ticking every second forever.

use std::time::{Duration, Instant};

const BUSY_WINDOW: Duration = Duration::from_secs(10);
const ACTIVE_TICK: Duration = Duration::from_secs(1);
/// WireGuard sends an empty packet this long after it received data and had nothing to send.
const PASSIVE_KEEPALIVE: Duration = Duration::from_secs(10);
/// With no keepalive nothing is due while idle, but a session that ages out has to be noticed.
const NO_KEEPALIVE_WAKE: Duration = Duration::from_secs(120);
const MIN_SLEEP: Duration = Duration::from_millis(200);
/// How long an unanswered handshake is checked every second, then every `SLOW_TICK` until
/// `SLOWEST_AFTER`, then every `SLOWEST_TICK`. A network change retries at once regardless.
const FAST_RETRY_FOR: Duration = Duration::from_secs(10);
/// A little over boringtun's 5 s retransmit time: it stamps a send slightly after the loop's own
/// clock, so a tick of exactly 5 s can land just before the retry is due and wait a whole tick more.
const SLOW_TICK: Duration = Duration::from_secs(6);
const SLOWEST_AFTER: Duration = Duration::from_secs(60);
const SLOWEST_TICK: Duration = Duration::from_secs(15);

pub struct Inputs {
    pub now: Instant,
    /// A session exists, or one did and has aged out (boringtun forgets it after 180 s). Idle
    /// ticking then follows the keepalive; before any session the loop ticks every second.
    pub had_session: bool,
    pub last_tx: Instant,
    pub last_rx: Instant,
    pub last_activity: Instant,
    pub keepalive: Option<Duration>,
    /// How long the oldest handshake initiation that nobody has answered has been waiting.
    pub handshake_unanswered: Option<Duration>,
}

fn retry_tick(waited: Duration) -> Duration {
    if waited < FAST_RETRY_FOR {
        ACTIVE_TICK
    } else if waited < SLOWEST_AFTER {
        SLOW_TICK
    } else {
        SLOWEST_TICK
    }
}

pub fn next_deadline(i: &Inputs) -> Instant {
    let busy = i.now.saturating_duration_since(i.last_activity) < BUSY_WINDOW;
    if busy {
        return i.now + ACTIVE_TICK;
    }
    // A handshake in flight needs its retry whether or not an older session still exists.
    if let Some(waited) = i.handshake_unanswered {
        return i.now + retry_tick(waited);
    }
    if !i.had_session {
        return i.now + ACTIVE_TICK;
    }
    let mut due = match i.keepalive {
        Some(k) => i.last_tx + k,
        None => i.now + NO_KEEPALIVE_WAKE,
    };
    if i.last_rx > i.last_tx {
        due = due.min(i.last_rx + PASSIVE_KEEPALIVE);
    }
    due.max(i.now + MIN_SLEEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(
        now: Instant,
        secs_since_tx: u64,
        secs_since_rx: u64,
        secs_since_activity: u64,
    ) -> Inputs {
        Inputs {
            now,
            had_session: true,
            last_tx: now - Duration::from_secs(secs_since_tx),
            last_rx: now - Duration::from_secs(secs_since_rx),
            last_activity: now - Duration::from_secs(secs_since_activity),
            keepalive: Some(Duration::from_secs(25)),
            handshake_unanswered: None,
        }
    }

    #[test]
    fn an_idle_tunnel_sleeps_until_the_keepalive_is_due() {
        let now = Instant::now() + Duration::from_secs(1000);
        let i = at(now, 5, 5, 300);
        assert_eq!(next_deadline(&i), i.last_tx + Duration::from_secs(25));
    }

    #[test]
    fn data_received_with_nothing_sent_wakes_for_the_passive_keepalive() {
        let now = Instant::now() + Duration::from_secs(1000);
        let i = at(now, 5, 1, 300);
        assert_eq!(next_deadline(&i), i.last_rx + Duration::from_secs(10));
    }

    #[test]
    fn a_busy_or_unestablished_tunnel_ticks_every_second() {
        let now = Instant::now() + Duration::from_secs(1000);
        assert_eq!(
            next_deadline(&at(now, 1, 1, 3)),
            now + Duration::from_secs(1)
        );
        let mut cold = at(now, 5, 5, 300);
        cold.had_session = false;
        assert_eq!(next_deadline(&cold), now + Duration::from_secs(1));
    }

    #[test]
    fn an_overdue_keepalive_still_sleeps_a_little() {
        let now = Instant::now() + Duration::from_secs(1000);
        let overdue = at(now, 90, 90, 300);
        assert_eq!(next_deadline(&overdue), now + Duration::from_millis(200));
    }

    #[test]
    fn a_tunnel_with_no_keepalive_that_is_left_alone_wakes_every_two_minutes_not_five_times_a_second()
     {
        let now = Instant::now() + Duration::from_secs(10_000);
        for idle in [5, 119, 121, 200, 600, 5_000] {
            let mut none = at(now, idle, idle, 5_000);
            none.keepalive = None;
            assert_eq!(
                next_deadline(&none),
                now + Duration::from_secs(120),
                "idle for {idle} s"
            );
        }
    }

    #[test]
    fn an_unanswered_handshake_is_retried_even_beside_an_older_session_and_backs_off() {
        let now = Instant::now() + Duration::from_secs(10_000);
        for (waited, tick) in [(1, 1), (9, 1), (10, 6), (59, 6), (60, 15), (3_000, 15)] {
            let mut i = at(now, 5, 5, 300);
            i.handshake_unanswered = Some(Duration::from_secs(waited));
            assert_eq!(
                next_deadline(&i),
                now + Duration::from_secs(tick),
                "waited {waited} s"
            );
            i.had_session = false;
            assert_eq!(
                next_deadline(&i),
                now + Duration::from_secs(tick),
                "waited {waited} s, never established"
            );
        }
    }

    #[test]
    fn the_next_wake_is_never_sooner_than_the_floor_and_never_later_than_the_keepalive() {
        let now = Instant::now() + Duration::from_secs(10_000);
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut roll = |bound: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % bound
        };
        for _ in 0..5_000 {
            let mut i = at(now, roll(1_000), roll(1_000), roll(1_000));
            i.had_session = roll(2) == 0;
            i.keepalive = (roll(3) != 0).then(|| Duration::from_secs(10 + roll(291)));
            i.handshake_unanswered = (roll(3) == 0).then(|| Duration::from_secs(roll(2_000)));
            let wait = next_deadline(&i).duration_since(now);
            assert!(wait >= MIN_SLEEP, "{wait:?}");
            assert!(wait <= Duration::from_secs(300), "{wait:?}");
        }
    }
}
