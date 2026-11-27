//! Choices the owner makes in the web app that HQ's own background work reads (`sql/139_owner_prefs.sql`).
//!
//! A pref overrides the config file's value for the same thing, so the owner can change it without editing
//! files. Only keys listed in [`KNOWN`] can be stored, and each has its own check on the value, so nothing
//! else can be written through the web route.

use crate::Database;
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};

/// Whether HQ writes a short title for a chat after its first reply (`true` or `false`).
pub const CHAT_AUTO_TITLE: &str = "chat.auto_title";
/// Which helper requests reach the owner: `nothing`, `loses_work` or `everything`.
pub const HELPERS_ASK_LEVEL: &str = "helpers.ask_level";

/// How many focus lanes may be active at once, 1 to 5 (`crate::focus_lanes`).
pub const FOCUS_MAX_LANES: &str = "focus.max_lanes";

/// The IANA zone the owner's times mean (`Africa/Kampala`). Reminders and daily nudges read it.
pub const OWNER_TIMEZONE: &str = "owner.timezone";

/// Every key that may be stored.
pub const KNOWN: [&str; 4] = [
    CHAT_AUTO_TITLE,
    HELPERS_ASK_LEVEL,
    FOCUS_MAX_LANES,
    OWNER_TIMEZONE,
];

/// Whether `value` is one this key accepts.
pub fn valid(key: &str, value: &str) -> bool {
    match key {
        CHAT_AUTO_TITLE => matches!(value, "true" | "false"),
        HELPERS_ASK_LEVEL => matches!(value, "nothing" | "loses_work" | "everything"),
        FOCUS_MAX_LANES => matches!(value, "1" | "2" | "3" | "4" | "5"),
        OWNER_TIMEZONE => value.parse::<chrono_tz::Tz>().is_ok(),
        _ => false,
    }
}

/// The stored value, or None when the owner never set it.
pub fn get(db: &Database, key: &str) -> Result<Option<String>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT value FROM owner_prefs WHERE key = ?1",
            params![key],
            |r| r.get::<_, String>(0),
        )
        .optional()?)
    })
}

/// Stores a value. A key or value this module does not know is refused.
pub fn set(db: &Database, key: &str, value: &str) -> Result<()> {
    if !valid(key, value) {
        bail!("'{value}' is not a choice for {key}");
    }
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO owner_prefs (key, value, updated_at) VALUES (?1, ?2, datetime('now')) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value],
        )?;
        Ok(())
    })
}

/// The owner's chosen zone, when they set one and it still parses.
pub fn timezone(db: &Database) -> Option<chrono_tz::Tz> {
    get(db, OWNER_TIMEZONE)
        .ok()
        .flatten()
        .and_then(|z| z.parse().ok())
}

/// The owner's wall clock at `now`: the hour and the date in their zone when they set one, and on
/// this machine's clock when they did not.
pub fn wall_clock(db: &Database, now: chrono::DateTime<chrono::Utc>) -> (u32, chrono::NaiveDate) {
    use chrono::Timelike;
    match timezone(db) {
        Some(tz) => {
            let local = now.with_timezone(&tz);
            (local.hour(), local.date_naive())
        }
        None => {
            let local = now.with_timezone(&chrono::Local);
            (local.hour(), local.date_naive())
        }
    }
}

/// Whether HQ names chats after their first reply. On unless the owner turned it off.
pub fn auto_title_on(db: &Database) -> bool {
    !matches!(get(db, CHAT_AUTO_TITLE), Ok(Some(v)) if v == "false")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_is_stored_replaced_and_read_back() {
        let db = Database::open_memory().unwrap();
        assert_eq!(get(&db, HELPERS_ASK_LEVEL).unwrap(), None);
        set(&db, HELPERS_ASK_LEVEL, "loses_work").unwrap();
        set(&db, HELPERS_ASK_LEVEL, "everything").unwrap();
        assert_eq!(
            get(&db, HELPERS_ASK_LEVEL).unwrap().as_deref(),
            Some("everything")
        );
    }

    #[test]
    fn only_known_keys_and_their_own_values_are_accepted() {
        let db = Database::open_memory().unwrap();
        assert!(set(&db, "something.else", "true").is_err());
        assert!(set(&db, CHAT_AUTO_TITLE, "maybe").is_err());
        assert!(set(&db, HELPERS_ASK_LEVEL, "always").is_err());
        assert_eq!(get(&db, "something.else").unwrap(), None);
    }

    #[test]
    fn the_owner_timezone_must_be_a_real_zone() {
        let db = Database::open_memory().unwrap();
        assert!(timezone(&db).is_none());
        assert!(set(&db, OWNER_TIMEZONE, "Mars/Olympus").is_err());
        set(&db, OWNER_TIMEZONE, "Africa/Kampala").unwrap();
        assert_eq!(timezone(&db), Some(chrono_tz::Africa::Kampala));
    }

    #[test]
    fn the_wall_clock_follows_the_owners_zone_not_the_servers() {
        use chrono::TimeZone;
        let db = Database::open_memory().unwrap();
        set(&db, OWNER_TIMEZONE, "Africa/Kampala").unwrap();
        // 22:00 UTC is 01:00 the next day in Kampala: the middle of the night.
        let late = chrono::Utc
            .with_ymd_and_hms(2026, 11, 26, 22, 0, 0)
            .unwrap();
        let (hour, date) = wall_clock(&db, late);
        assert_eq!(hour, 1);
        assert_eq!(date.to_string(), "2026-11-27");
        // 06:00 UTC is 09:00 there, inside waking hours.
        let morning = chrono::Utc.with_ymd_and_hms(2026, 11, 26, 6, 0, 0).unwrap();
        assert_eq!(wall_clock(&db, morning).0, 9);
    }

    #[test]
    fn chat_titles_are_on_until_turned_off() {
        let db = Database::open_memory().unwrap();
        assert!(auto_title_on(&db));
        set(&db, CHAT_AUTO_TITLE, "false").unwrap();
        assert!(!auto_title_on(&db));
        set(&db, CHAT_AUTO_TITLE, "true").unwrap();
        assert!(auto_title_on(&db));
    }
}
