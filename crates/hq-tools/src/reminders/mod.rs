//! Reminders the owner set: when each rings, how a repeat or a nudge advances, and the inbox item
//! each ping raises. No model is involved anywhere here. The rows live in `hq_db::reminders`.
//!
//! `schedule` is pure (every function takes `now`), `fire` is the pass the daemon runs.

pub mod fire;
pub mod schedule;
pub mod tools;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// The owner's local time as one line for the model's per-turn note, so "tomorrow at 4" resolves
/// against their clock and not the server's. Kept out of the cached system prompt.
pub fn clock_line(zone: Tz, now: DateTime<Utc>) -> String {
    format!(
        "It is {} for the owner ({}).",
        now.with_timezone(&zone).format("%A %-d %B %Y, %-I:%M %p"),
        zone.name()
    )
}

/// Saves the owner's zone the first time a phone reports one, so later turns from any surface
/// can resolve their times. An owner who already chose a zone is never overwritten.
pub fn seed_timezone(db: &hq_db::Database, zone: Tz) {
    if hq_db::owner_prefs::timezone(db).is_none() {
        let _ = hq_db::owner_prefs::set(db, hq_db::owner_prefs::OWNER_TIMEZONE, zone.name());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn the_clock_line_is_in_the_owners_zone() {
        let now = Utc.with_ymd_and_hms(2026, 11, 27, 13, 20, 0).unwrap();
        assert_eq!(
            clock_line(chrono_tz::Africa::Kampala, now),
            "It is Friday 27 November 2026, 4:20 PM for the owner (Africa/Kampala)."
        );
    }

    #[test]
    fn the_first_zone_is_kept_and_a_later_one_does_not_replace_it() {
        let db = hq_db::Database::open_memory().unwrap();
        seed_timezone(&db, chrono_tz::Africa::Kampala);
        seed_timezone(&db, chrono_tz::Europe::London);
        assert_eq!(
            hq_db::owner_prefs::timezone(&db),
            Some(chrono_tz::Africa::Kampala)
        );
    }
}
