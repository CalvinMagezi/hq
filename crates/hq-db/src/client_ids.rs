//! Client ids for what a browser or phone makes (`sql/111_client_ids.sql`): the first send of an id
//! claims it, a repeat learns the item already made, so an outbox that resends never makes two.

use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};

/// How long an id is remembered; an outbox gives up on a message after a day.
pub const KEEP_DAYS: i64 = 7;
/// A claim still without its item after this long was cut off (a crash, a failed save): the next
/// send of that id makes the item instead of being taken for a repeat.
pub const STALE_CLAIM_SECS: i64 = 5 * 60;
/// Ids longer than this are not ids a client made; they are not remembered.
pub const MAX_ID_CHARS: usize = 128;

pub const KIND_MESSAGE: &str = "message";
pub const KIND_NOTE: &str = "note";
pub const KIND_TASK: &str = "task";

/// What a send's client id says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// The first time: make the item, then [`record`] its id.
    New,
    /// Seen before. `item_id` is empty while the first send is still making it.
    Seen { item_id: String },
}

/// Claims `client_id` for `kind`, or says what it already made. Old ids are dropped first.
pub fn claim(conn: &Connection, kind: &str, client_id: &str) -> Result<Claim> {
    let cutoff = (Utc::now() - Duration::days(KEEP_DAYS)).to_rfc3339();
    conn.execute(
        "DELETE FROM client_ids WHERE created_at < ?1",
        params![cutoff],
    )?;
    let stale = (Utc::now() - Duration::seconds(STALE_CLAIM_SECS)).to_rfc3339();
    conn.execute(
        "DELETE FROM client_ids WHERE item_id = '' AND created_at < ?1",
        params![stale],
    )?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO client_ids (kind, client_id, created_at) VALUES (?1, ?2, ?3)",
        params![kind, client_id, Utc::now().to_rfc3339()],
    )?;
    if inserted == 1 {
        return Ok(Claim::New);
    }
    let item_id: Option<String> = conn
        .query_row(
            "SELECT item_id FROM client_ids WHERE kind = ?1 AND client_id = ?2",
            params![kind, client_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Claim::Seen {
        item_id: item_id.unwrap_or_default(),
    })
}

/// Records the item a claimed id made.
pub fn record(conn: &Connection, kind: &str, client_id: &str, item_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE client_ids SET item_id = ?3 WHERE kind = ?1 AND client_id = ?2",
        params![kind, client_id, item_id],
    )?;
    Ok(())
}

/// Gives a claim back when the item could not be made, so the next send tries again.
pub fn release(conn: &Connection, kind: &str, client_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM client_ids WHERE kind = ?1 AND client_id = ?2 AND item_id = ''",
        params![kind, client_id],
    )?;
    Ok(())
}

/// A client id worth remembering: present, not blank, not absurdly long.
pub fn usable(client_id: Option<&str>) -> Option<&str> {
    client_id
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.chars().count() <= MAX_ID_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::migrations::run(&c).unwrap();
        c
    }

    #[test]
    fn the_first_send_claims_and_a_repeat_learns_what_was_made() {
        let c = conn();
        assert_eq!(claim(&c, KIND_NOTE, "abc").unwrap(), Claim::New);
        assert_eq!(
            claim(&c, KIND_NOTE, "abc").unwrap(),
            Claim::Seen {
                item_id: String::new()
            },
            "still being made"
        );
        record(&c, KIND_NOTE, "abc", "Notes/Title.md").unwrap();
        assert_eq!(
            claim(&c, KIND_NOTE, "abc").unwrap(),
            Claim::Seen {
                item_id: "Notes/Title.md".into()
            }
        );
        assert_eq!(
            claim(&c, KIND_TASK, "abc").unwrap(),
            Claim::New,
            "kinds are apart"
        );
    }

    #[test]
    fn a_released_claim_can_be_tried_again_and_old_ids_go() {
        let c = conn();
        assert_eq!(claim(&c, KIND_MESSAGE, "m1").unwrap(), Claim::New);
        release(&c, KIND_MESSAGE, "m1").unwrap();
        assert_eq!(claim(&c, KIND_MESSAGE, "m1").unwrap(), Claim::New);
        let old = (Utc::now() - Duration::days(KEEP_DAYS + 1)).to_rfc3339();
        c.execute(
            "UPDATE client_ids SET created_at = ?1, item_id = 'x'",
            params![old],
        )
        .unwrap();
        assert_eq!(claim(&c, KIND_MESSAGE, "m1").unwrap(), Claim::New);
    }

    #[test]
    fn a_claim_cut_off_before_its_item_was_made_is_tried_again_later() {
        let c = conn();
        assert_eq!(claim(&c, KIND_NOTE, "n1").unwrap(), Claim::New);
        let old = (Utc::now() - Duration::seconds(STALE_CLAIM_SECS + 1)).to_rfc3339();
        c.execute("UPDATE client_ids SET created_at = ?1", params![old])
            .unwrap();
        assert_eq!(claim(&c, KIND_NOTE, "n1").unwrap(), Claim::New);
    }

    #[test]
    fn only_real_ids_are_remembered() {
        assert_eq!(usable(Some(" a1 ")), Some("a1"));
        assert_eq!(usable(Some("  ")), None);
        assert_eq!(usable(None), None);
        assert_eq!(usable(Some(&"x".repeat(MAX_ID_CHARS + 1))), None);
    }
}
