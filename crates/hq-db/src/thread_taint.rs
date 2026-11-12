//! Taint that outlives a turn: the first untrusted source a chat thread read.
//!
//! A session's taint flag dies with the session, but a chat thread runs many sessions. The
//! mark lives on the thread row so a later turn starts tainted with the same source.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// The source recorded when the mark cannot be read or written, so the policy fails closed.
pub const UNREADABLE_SOURCE: &str =
    "an earlier turn of this chat (its taint record was unreadable)";

/// The source that tainted the thread, if any. A missing thread is clean.
pub fn source(conn: &Connection, thread_id: &str) -> Result<Option<String>> {
    let found: Option<Option<String>> = conn
        .query_row(
            "SELECT taint_source FROM chat_threads WHERE thread_id = ?1",
            params![thread_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.flatten())
}

/// Record `source` unless the thread is already tainted: the first source is kept.
pub fn mark(conn: &Connection, thread_id: &str, source: &str) -> Result<()> {
    conn.execute(
        "UPDATE chat_threads SET taint_source = ?2 WHERE thread_id = ?1 AND taint_source IS NULL",
        params![thread_id, source],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::create_thread;

    fn setup() -> (Connection, String, String) {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        let a = create_thread(&conn, "a", "user", "user").unwrap().thread_id;
        let b = create_thread(&conn, "b", "user", "user").unwrap().thread_id;
        (conn, a, b)
    }

    #[test]
    fn mark_taints_only_its_thread_and_keeps_the_first_source() {
        let (conn, a, b) = setup();
        assert_eq!(source(&conn, &a).unwrap(), None);
        mark(&conn, &a, "calendar_list_events").unwrap();
        mark(&conn, &a, "web_fetch").unwrap();
        assert_eq!(
            source(&conn, &a).unwrap().as_deref(),
            Some("calendar_list_events")
        );
        assert_eq!(source(&conn, &b).unwrap(), None);
    }

    #[test]
    fn unknown_thread_is_clean() {
        let (conn, _, _) = setup();
        assert_eq!(source(&conn, "missing").unwrap(), None);
    }
}
