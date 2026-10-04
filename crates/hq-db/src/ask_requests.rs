//! Questions an external MCP client put to HQ's chat agent (`hq_ask`). The
//! chat thread holds the conversation; a row here lets a later MCP request find
//! the outcome and keeps a retried `external_id` from posting twice.

use anyhow::{Result, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::Serialize;
use uuid::Uuid;

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_ANSWERED: &str = "answered";
pub const STATUS_FAILED: &str = "failed";

/// Why an ask that was waiting when the daemon restarted can never finish.
pub const RESTART_REASON: &str = "HQ restarted before the reply finished, so this question was not answered. Ask again with a new external_id.";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AskRow {
    pub ask_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub external_id: Option<String>,
    /// Which MCP key asked: `full` or `handoff`.
    pub scope: String,
    /// `read_only` or `full`.
    pub mode: String,
    /// Display label the client gave itself; not an identity.
    pub caller: String,
    /// Hash of what the question was, so a reused `external_id` with different
    /// content is caught rather than answered with the wrong ask.
    pub fingerprint: String,
    pub status: String,
    pub answer: Option<String>,
    pub answer_message_id: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub answered_at: Option<String>,
}

const COLUMNS: &str = "ask_id, thread_id, turn_id, external_id, scope, mode, caller, fingerprint, \
                       status, answer, answer_message_id, error, created_at, answered_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<AskRow> {
    Ok(AskRow {
        ask_id: row.get(0)?,
        thread_id: row.get(1)?,
        turn_id: row.get(2)?,
        external_id: row.get(3)?,
        scope: row.get(4)?,
        mode: row.get(5)?,
        caller: row.get(6)?,
        fingerprint: row.get(7)?,
        status: row.get(8)?,
        answer: row.get(9)?,
        answer_message_id: row.get(10)?,
        error: row.get(11)?,
        created_at: row.get(12)?,
        answered_at: row.get(13)?,
    })
}

/// Which thread the question goes into.
pub enum ThreadTarget<'a> {
    /// A thread that must already exist and be active.
    Existing(&'a str),
    /// A new web chat with this title.
    New { title: &'a str },
}

/// Most new asks one key scope may file in a day.
pub const ASKS_PER_DAY: i64 = 200;
/// Most asks of one key scope that may be waiting for an answer at once.
pub const MAX_PENDING_PER_SCOPE: i64 = 3;

pub struct NewAsk<'a> {
    pub thread: ThreadTarget<'a>,
    pub external_id: Option<&'a str>,
    pub scope: &'a str,
    pub mode: &'a str,
    pub caller: &'a str,
    pub fingerprint: &'a str,
}

/// What `open` did.
#[derive(Debug)]
pub struct OpenedAsk {
    pub row: AskRow,
    /// False when `external_id` matched an earlier ask, which is returned as it stands.
    pub created: bool,
    /// The ask made its own thread, which `discard_unstarted` removes again.
    pub new_thread: bool,
}

pub fn get(conn: &Connection, ask_id: &str) -> Result<Option<AskRow>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM hq_asks WHERE ask_id = ?1"),
        [ask_id],
        from_row,
    )
    .optional()
    .map_err(Into::into)
}

pub fn get_by_external(
    conn: &Connection,
    scope: &str,
    external_id: &str,
) -> Result<Option<AskRow>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM hq_asks WHERE scope = ?1 AND external_id = ?2"),
        params![scope, external_id],
        from_row,
    )
    .optional()
    .map_err(Into::into)
}

/// Finds the ask for `external_id` or files a new pending one, and makes its
/// thread when asked to, all under one write lock: two calls with the same key
/// end with one ask, one thread and one question.
pub fn open(conn: &Connection, new: &NewAsk<'_>) -> Result<OpenedAsk> {
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if let Some(external) = new.external_id
        && let Some(row) = get_by_external(&tx, new.scope, external)?
    {
        return Ok(OpenedAsk {
            row,
            created: false,
            new_thread: false,
        });
    }
    // Counted under the same write lock as the insert, so concurrent opens cannot all slip past.
    if count_pending(&tx, new.scope)? >= MAX_PENDING_PER_SCOPE {
        bail!(
            "{MAX_PENDING_PER_SCOPE} questions are already waiting for answers; collect one with hq_ask_result first"
        );
    }
    let since = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    let today: i64 = tx.query_row(
        "SELECT COUNT(*) FROM hq_asks WHERE scope = ?1 AND created_at > ?2",
        params![new.scope, since],
        |row| row.get(0),
    )?;
    if today >= ASKS_PER_DAY {
        bail!("this key has already asked {ASKS_PER_DAY} questions in the last 24 hours");
    }
    let (thread_id, new_thread) = match new.thread {
        ThreadTarget::Existing(id) => match crate::chat::get_thread(&tx, id)? {
            None => bail!("thread_id {id} does not exist"),
            Some(t) if t.status != "active" => bail!("thread_id {id} is {}, not active", t.status),
            Some(t) => (t.thread_id, false),
        },
        ThreadTarget::New { title } => (
            crate::chat::create_thread(&tx, title, "user", "user")?.thread_id,
            true,
        ),
    };
    let ask_id = format!("ask-{}", Uuid::new_v4().simple());
    tx.execute(
        "INSERT INTO hq_asks (ask_id, thread_id, external_id, scope, mode, caller, fingerprint, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            ask_id,
            thread_id,
            new.external_id,
            new.scope,
            new.mode,
            new.caller,
            new.fingerprint,
            Utc::now().to_rfc3339()
        ],
    )?;
    let row = get(&tx, &ask_id)?.ok_or_else(|| anyhow::anyhow!("ask {ask_id} vanished"))?;
    tx.commit()?;
    Ok(OpenedAsk {
        row,
        created: true,
        new_thread,
    })
}

/// Asks of this key scope still waiting for an answer.
pub fn count_pending(conn: &Connection, scope: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM hq_asks WHERE scope = ?1 AND status = 'pending'",
        [scope],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

/// Whether an ask created this thread and the owner has typed nothing in it. A thread the owner
/// opened and merely asked into is not ask-owned, and neither is one the owner has since joined.
pub fn thread_is_ask_owned(conn: &Connection, thread_id: &str) -> Result<bool> {
    Ok(thread_has_ask(conn, thread_id, None, None)? && thread_is_mcp_only(conn, thread_id)?)
}

/// Asks of this scope and mode still waiting for an answer.
pub fn count_pending_mode(conn: &Connection, scope: &str, mode: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM hq_asks WHERE scope = ?1 AND mode = ?2 AND status = 'pending'",
        [scope, mode],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

/// Whether every user message in the thread came from an MCP client, so
/// continuing it cannot replay anything the owner typed.
pub fn thread_is_mcp_only(conn: &Connection, thread_id: &str) -> Result<bool> {
    let messages = crate::chat::get_messages_page(conn, thread_id, 5000, None)?;
    Ok(messages
        .iter()
        .filter(|m| m.message.role == "user")
        .all(|m| {
            m.meta
                .as_ref()
                .and_then(|v| v.pointer("/source/kind"))
                .and_then(|k| k.as_str())
                == Some("mcp")
        }))
}

/// Whether some ask of this scope and mode owns the thread.
pub fn thread_has_ask(
    conn: &Connection,
    thread_id: &str,
    scope: Option<&str>,
    mode: Option<&str>,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM hq_asks WHERE thread_id = ?1
            AND (?2 IS NULL OR scope = ?2) AND (?3 IS NULL OR mode = ?3))",
        params![thread_id, scope, mode],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

/// Records the reply turn an ask started. Never touches status, so it is safe
/// to call after the turn already finished.
pub fn set_turn(conn: &Connection, ask_id: &str, turn_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE hq_asks SET turn_id = ?2 WHERE ask_id = ?1",
        params![ask_id, turn_id],
    )?;
    Ok(())
}

/// Ends a pending ask. Only the first call counts, so a turn that finishes
/// after a stop, or a restart sweep that races a finish, cannot rewrite the outcome.
pub fn settle(
    conn: &Connection,
    ask_id: &str,
    status: &str,
    answer: Option<(&str, &str)>,
    error: Option<&str>,
) -> Result<bool> {
    debug_assert!(status == STATUS_ANSWERED || status == STATUS_FAILED);
    let (text, message_id) = answer.unzip();
    let changed = conn.execute(
        "UPDATE hq_asks SET status = ?2, answer = ?3, answer_message_id = ?4, error = ?5, answered_at = ?6
          WHERE ask_id = ?1 AND status = 'pending'",
        params![ask_id, status, text, message_id, error, Utc::now().to_rfc3339()],
    )?;
    Ok(changed > 0)
}

/// Fails every ask still pending. At startup no reply is running, so each one
/// belongs to a turn the old process took with it.
pub fn fail_all_pending(conn: &Connection, reason: &str) -> Result<usize> {
    let changed = conn.execute(
        "UPDATE hq_asks SET status = 'failed', error = ?1, answered_at = ?2 WHERE status = 'pending'",
        params![reason, Utc::now().to_rfc3339()],
    )?;
    Ok(changed)
}

/// Removes an ask whose question never reached the thread, and the thread too
/// when the ask made it, so a failed start leaves nothing behind and the same
/// `external_id` can be tried again. Does nothing once a turn was recorded.
pub fn discard_unstarted(conn: &Connection, ask: &AskRow, new_thread: bool) -> Result<bool> {
    let removed = conn.execute(
        "DELETE FROM hq_asks WHERE ask_id = ?1 AND status = 'pending' AND turn_id IS NULL",
        [&ask.ask_id],
    )?;
    if removed > 0 && new_thread {
        crate::chat::archive_thread(conn, &ask.thread_id)?;
    }
    Ok(removed > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn new_ask<'a>(thread: ThreadTarget<'a>, external: Option<&'a str>) -> NewAsk<'a> {
        NewAsk {
            thread,
            external_id: external,
            scope: "full",
            mode: "read_only",
            caller: "claude-code",
            fingerprint: "fp",
        }
    }

    #[test]
    fn only_a_thread_an_ask_created_and_the_owner_never_typed_in_is_ask_owned() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let mine = crate::chat::create_thread(c, "mine", "user", "user")?.thread_id;
            crate::chat::add_message(c, &mine, "user", "my own question")?;
            open(c, &new_ask(ThreadTarget::Existing(&mine), None))?;
            assert!(thread_has_ask(c, &mine, None, None)? && !thread_is_ask_owned(c, &mine)?);
            let created = open(c, &new_ask(ThreadTarget::New { title: "q" }, None))?.row.thread_id;
            assert!(thread_is_ask_owned(c, &created)?);
            crate::chat::add_message(c, &created, "user", "I joined in")?;
            assert!(!thread_is_ask_owned(c, &created)?, "once the owner types, it is theirs");
            assert!(!thread_is_ask_owned(c, "no-such-thread")?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_same_external_id_returns_one_ask_and_one_thread() {
        let db = Database::open_memory().unwrap();
        let open = |ext| {
            db.with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, ext)))
                .unwrap()
        };
        let first = open(Some("k1"));
        let again = open(Some("k1"));
        assert!(first.created && first.new_thread);
        assert!(!again.created && !again.new_thread);
        assert_eq!(first.row.ask_id, again.row.ask_id);
        let threads = db
            .with_conn(|c| crate::chat::list_threads(c, 50, 0, Some("web")))
            .unwrap();
        assert_eq!(threads.len(), 1);
        let other = open(Some("k2"));
        assert_ne!(other.row.ask_id, first.row.ask_id);
        db.with_conn(|c| fail_all_pending(c, "test")).unwrap();
        assert!(
            open(None).created && open(None).created,
            "no key means no dedupe"
        );
    }

    #[test]
    fn external_ids_are_scoped_to_the_key_that_made_them() {
        let db = Database::open_memory().unwrap();
        let mut handoff = new_ask(ThreadTarget::New { title: "q" }, Some("k"));
        let full = db
            .with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, Some("k"))))
            .unwrap();
        handoff.scope = "handoff";
        let other = db.with_conn(|c| open(c, &handoff)).unwrap();
        assert!(full.created && other.created);
    }

    #[test]
    fn an_existing_thread_must_exist_and_be_active() {
        let db = Database::open_memory().unwrap();
        let err = db
            .with_conn(|c| open(c, &new_ask(ThreadTarget::Existing("nope"), None)))
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
        let thread = db
            .with_conn(|c| crate::chat::create_thread(c, "t", "user", "user"))
            .unwrap();
        let ok = db
            .with_conn(|c| open(c, &new_ask(ThreadTarget::Existing(&thread.thread_id), None)))
            .unwrap();
        assert!(!ok.new_thread && ok.row.thread_id == thread.thread_id);
        db.with_conn(|c| crate::chat::archive_thread(c, &thread.thread_id))
            .unwrap();
        let err = db
            .with_conn(|c| open(c, &new_ask(ThreadTarget::Existing(&thread.thread_id), None)))
            .unwrap_err();
        assert!(err.to_string().contains("not active"));
    }

    #[test]
    fn only_the_first_settle_counts() {
        let db = Database::open_memory().unwrap();
        let ask = db
            .with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, None)))
            .unwrap()
            .row;
        assert!(
            db.with_conn(|c| settle(c, &ask.ask_id, STATUS_ANSWERED, Some(("42", "m1")), None))
                .unwrap()
        );
        assert!(
            !db.with_conn(|c| settle(c, &ask.ask_id, STATUS_FAILED, None, Some("late")))
                .unwrap()
        );
        let row = db.with_conn(|c| get(c, &ask.ask_id)).unwrap().unwrap();
        assert_eq!(
            (row.status.as_str(), row.answer.as_deref()),
            (STATUS_ANSWERED, Some("42"))
        );
        assert_eq!(row.answer_message_id.as_deref(), Some("m1"));
        assert!(row.answered_at.is_some() && row.error.is_none());
    }

    #[test]
    fn restart_fails_pending_asks_and_leaves_settled_ones() {
        let db = Database::open_memory().unwrap();
        let mk = |ext| {
            db.with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, Some(ext))))
                .unwrap()
                .row
        };
        let (pending, done) = (mk("a"), mk("b"));
        db.with_conn(|c| settle(c, &done.ask_id, STATUS_ANSWERED, Some(("x", "m")), None))
            .unwrap();
        assert_eq!(
            db.with_conn(|c| fail_all_pending(c, RESTART_REASON))
                .unwrap(),
            1
        );
        let row = db.with_conn(|c| get(c, &pending.ask_id)).unwrap().unwrap();
        assert_eq!(
            (row.status.as_str(), row.error.as_deref()),
            (STATUS_FAILED, Some(RESTART_REASON))
        );
        let kept = db.with_conn(|c| get(c, &done.ask_id)).unwrap().unwrap();
        assert_eq!(kept.status, STATUS_ANSWERED);
        assert_eq!(
            db.with_conn(|c| fail_all_pending(c, RESTART_REASON))
                .unwrap(),
            0,
            "a second sweep is a no-op"
        );
    }

    #[test]
    fn a_failed_start_is_discarded_with_its_thread_but_a_started_turn_is_kept() {
        let db = Database::open_memory().unwrap();
        let opened = db
            .with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, Some("k"))))
            .unwrap();
        assert!(
            db.with_conn(|c| discard_unstarted(c, &opened.row, opened.new_thread))
                .unwrap()
        );
        assert!(
            db.with_conn(|c| get(c, &opened.row.ask_id))
                .unwrap()
                .is_none()
        );
        let thread = db
            .with_conn(|c| crate::chat::get_thread(c, &opened.row.thread_id))
            .unwrap()
            .unwrap();
        assert_eq!(thread.status, "archived");
        let retry = db
            .with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, Some("k"))))
            .unwrap();
        assert!(retry.created, "the key is free again");

        db.with_conn(|c| set_turn(c, &retry.row.ask_id, "turn-1"))
            .unwrap();
        assert!(
            !db.with_conn(|c| discard_unstarted(c, &retry.row, true))
                .unwrap()
        );
        assert!(
            db.with_conn(|c| get(c, &retry.row.ask_id))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn concurrent_opens_cannot_get_past_the_pending_cap() {
        let dir = std::env::temp_dir().join(format!("hq-asks-cap-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = std::sync::Arc::new(Database::open(&dir.join("vault.db")).unwrap());
        let attempts = MAX_PENDING_PER_SCOPE as usize * 4;
        let handles: Vec<_> = (0..attempts)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || {
                    db.with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, None)))
                        .is_ok()
                })
            })
            .collect();
        let opened = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(opened as i64, MAX_PENDING_PER_SCOPE);
        assert_eq!(
            db.with_conn(|c| count_pending(c, "full")).unwrap(),
            MAX_PENDING_PER_SCOPE
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_scope_has_a_daily_cap_on_new_asks() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for i in 0..ASKS_PER_DAY {
                c.execute(
                    "INSERT INTO hq_asks (ask_id, thread_id, scope, mode, caller, fingerprint, status, created_at)
                     VALUES (?1, 't', 'full', 'read_only', 'c', 'f', 'answered', ?2)",
                    params![format!("a{i}"), Utc::now().to_rfc3339()],
                )?;
            }
            Ok(())
        })
        .unwrap();
        let err = db
            .with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, None)))
            .unwrap_err();
        assert!(err.to_string().contains("last 24 hours"), "{err}");
        let mut other = new_ask(ThreadTarget::New { title: "q" }, None);
        other.scope = "handoff";
        assert!(
            db.with_conn(|c| open(c, &other)).is_ok(),
            "the other key is unaffected"
        );
        db.with_conn(|c| {
            c.execute(
                "UPDATE hq_asks SET created_at = '2020-01-01T00:00:00+00:00' WHERE scope = 'full'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            db.with_conn(|c| open(c, &new_ask(ThreadTarget::New { title: "q" }, None)))
                .is_ok()
        );
    }

    #[test]
    fn a_thread_is_mcp_only_until_a_person_types_in_it() {
        let db = Database::open_memory().unwrap();
        let thread = db
            .with_conn(|c| crate::chat::create_thread(c, "t", "user", "user"))
            .unwrap()
            .thread_id;
        assert!(
            db.with_conn(|c| thread_is_mcp_only(c, &thread)).unwrap(),
            "empty is fine"
        );
        let mcp = serde_json::json!({"source": {"kind": "mcp", "caller": "c"}});
        db.with_conn(|c| crate::chat::add_message_with_meta(c, &thread, "user", "q", Some(&mcp)))
            .unwrap();
        db.with_conn(|c| crate::chat::add_message(c, &thread, "assistant", "a"))
            .unwrap();
        assert!(
            db.with_conn(|c| thread_is_mcp_only(c, &thread)).unwrap(),
            "replies do not count"
        );
        db.with_conn(|c| crate::chat::add_message(c, &thread, "user", "mine"))
            .unwrap();
        assert!(!db.with_conn(|c| thread_is_mcp_only(c, &thread)).unwrap());
    }
}
