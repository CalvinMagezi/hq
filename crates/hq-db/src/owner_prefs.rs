//! Choices the owner makes in the web app that HQ's own background work reads (`sql/137_owner_prefs.sql`).
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

/// Every key that may be stored.
pub const KNOWN: [&str; 2] = [CHAT_AUTO_TITLE, HELPERS_ASK_LEVEL];

/// Whether `value` is one this key accepts.
pub fn valid(key: &str, value: &str) -> bool {
    match key {
        CHAT_AUTO_TITLE => matches!(value, "true" | "false"),
        HELPERS_ASK_LEVEL => matches!(value, "nothing" | "loses_work" | "everything"),
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
    fn chat_titles_are_on_until_turned_off() {
        let db = Database::open_memory().unwrap();
        assert!(auto_title_on(&db));
        set(&db, CHAT_AUTO_TITLE, "false").unwrap();
        assert!(!auto_title_on(&db));
        set(&db, CHAT_AUTO_TITLE, "true").unwrap();
        assert!(auto_title_on(&db));
    }
}
