//! Messages between agent sessions, stored as task comments so the task thread
//! stays the one durable record. A message is delivered by typing it into the
//! recipient's pane, and `delivered_at` records that it was.

use super::*;

/// Kind of a comment written by one agent session to another.
pub const KIND_MESSAGE: &str = "message";

const COLUMNS: &str = "id, task_id, author, body, created_at, kind, sender_session_id, \
                       to_session_id, reply_to, delivered_at";

/// A message from `from_session` to `to_session` on `task_id`. The author is the
/// sending session.
pub fn add_message(
    conn: &Connection,
    task_id: &str,
    from_session: &str,
    to_session: &str,
    body: &str,
    reply_to: Option<i64>,
) -> Result<TaskComment> {
    conn.execute(
        "INSERT INTO task_comments \
         (task_id, author, body, kind, sender_session_id, to_session_id, reply_to) \
         VALUES (?1, ?2, ?3, ?4, ?2, ?5, ?6)",
        params![task_id, from_session, body, KIND_MESSAGE, to_session, reply_to],
    )?;
    let id = conn.last_insert_rowid();
    changed();
    Ok(conn.query_row(
        &format!("SELECT {COLUMNS} FROM task_comments WHERE id = ?1"),
        params![id],
        row_to_comment,
    )?)
}

/// Takes the oldest message nobody has delivered to `to_session` yet. The claim
/// is one conditional write, so two sweeps cannot both deliver it. Give it back
/// with `release_message` if typing it failed.
pub fn claim_next_message(conn: &Connection, to_session: &str) -> Result<Option<TaskComment>> {
    let next: Option<i64> = conn
        .query_row(
            "SELECT id FROM task_comments \
             WHERE to_session_id = ?1 AND kind = ?2 AND delivered_at IS NULL \
             ORDER BY id LIMIT 1",
            params![to_session, KIND_MESSAGE],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = next else { return Ok(None) };
    let claimed = conn.execute(
        "UPDATE task_comments SET delivered_at = datetime('now') \
         WHERE id = ?1 AND delivered_at IS NULL",
        params![id],
    )?;
    if claimed == 0 {
        return Ok(None);
    }
    Ok(Some(conn.query_row(
        &format!("SELECT {COLUMNS} FROM task_comments WHERE id = ?1"),
        params![id],
        row_to_comment,
    )?))
}

/// Makes a claimed message deliverable again, after a failed attempt to type it.
pub fn release_message(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "UPDATE task_comments SET delivered_at = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// How many messages `from_session` has sent in the last `minutes`.
pub fn messages_sent_since(conn: &Connection, from_session: &str, minutes: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM task_comments \
         WHERE sender_session_id = ?1 AND kind = ?2 \
           AND created_at >= datetime('now', ?3)",
        params![from_session, KIND_MESSAGE, format!("-{minutes} minutes")],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{make, setup as test_db};
    use crate::Database;
    use super::*;

    fn setup() -> (Database, String) {
        let (db, initiative) = test_db();
        let task = make(&db, "tk-1", &initiative, None).unwrap();
        (db, task.id)
    }

    #[test]
    fn a_message_is_a_comment_authored_by_its_sender() {
        let (db, task) = setup();
        let m = db
            .with_conn(|c| add_message(c, &task, "hs-a", "hs-b", "hello", None))
            .unwrap();
        assert_eq!((m.kind.as_str(), m.author.as_str()), ("message", "hs-a"));
        assert_eq!(m.sender_session_id.as_deref(), Some("hs-a"));
        assert_eq!(m.to_session_id.as_deref(), Some("hs-b"));
        assert!(m.delivered_at.is_none());
        let thread = db.with_conn(|c| list_comments(c, &task)).unwrap();
        assert_eq!(thread.len(), 1, "it shows up in the task thread");
    }

    #[test]
    fn a_plain_comment_has_no_recipient_and_is_never_claimed() {
        let (db, task) = setup();
        db.with_conn(|c| add_comment(c, &task, "calvin", "note", None)).unwrap();
        let comment = db.with_conn(|c| list_comments(c, &task)).unwrap().remove(0);
        assert_eq!(comment.kind, "comment");
        assert!(comment.to_session_id.is_none());
        assert!(db.with_conn(|c| claim_next_message(c, "hs-b")).unwrap().is_none());
    }

    #[test]
    fn messages_are_claimed_once_in_order_and_only_by_their_recipient() {
        let (db, task) = setup();
        db.with_conn(|c| {
            add_message(c, &task, "hs-a", "hs-b", "first", None)?;
            add_message(c, &task, "hs-a", "hs-b", "second", None)?;
            add_message(c, &task, "hs-a", "hs-c", "for c", None)
        })
        .unwrap();
        let claim = |who: &str| db.with_conn(|c| claim_next_message(c, who)).unwrap();
        assert_eq!(claim("hs-b").unwrap().body, "first");
        assert_eq!(claim("hs-b").unwrap().body, "second");
        assert!(claim("hs-b").is_none(), "each is delivered once");
        assert_eq!(claim("hs-c").unwrap().body, "for c");
        assert!(claim("hs-a").is_none(), "the sender is not a recipient");
    }

    #[test]
    fn a_released_message_is_delivered_again() {
        let (db, task) = setup();
        let sent = db
            .with_conn(|c| add_message(c, &task, "hs-a", "hs-b", "retry me", None))
            .unwrap();
        let first = db.with_conn(|c| claim_next_message(c, "hs-b")).unwrap().unwrap();
        db.with_conn(|c| release_message(c, first.id)).unwrap();
        let again = db.with_conn(|c| claim_next_message(c, "hs-b")).unwrap().unwrap();
        assert_eq!(again.id, sent.id);
    }

    #[test]
    fn a_senders_recent_messages_are_counted() {
        let (db, task) = setup();
        db.with_conn(|c| {
            add_message(c, &task, "hs-a", "hs-b", "1", None)?;
            add_message(c, &task, "hs-a", "hs-c", "2", None)?;
            add_message(c, &task, "hs-z", "hs-b", "other sender", None)
        })
        .unwrap();
        assert_eq!(db.with_conn(|c| messages_sent_since(c, "hs-a", 60)).unwrap(), 2);
        assert_eq!(db.with_conn(|c| messages_sent_since(c, "nobody", 60)).unwrap(), 0);
    }
}
