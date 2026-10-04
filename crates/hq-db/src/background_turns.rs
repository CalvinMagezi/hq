//! Registry of durable background turns. One row per relay turn that detached
//! past its ack window: routing fields (platform/chat/thread/identity) for the
//! completion callback, the prompt, lifecycle status, and the harness child
//! sessions that did the work. Timestamps are unix epoch seconds.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

pub const STATUS_RUNNING: &str = "running";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_INTERRUPTED: &str = "interrupted";
pub const STATUS_CANCELLED: &str = "cancelled";

pub const KIND_TURN: &str = "turn";
pub const KIND_WATCH: &str = "watch";

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundTurnRow {
    pub id: String,
    pub platform: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub identity: Option<String>,
    pub prompt: String,
    pub status: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub result_text: Option<String>,
    pub child_session_ids: Vec<String>,
    pub cancel_token: Option<String>,
    pub kind: String,
    pub watch_interval_secs: Option<i64>,
    pub watch_until: Option<i64>,
    pub watch_last_fired: Option<i64>,
    /// Who this turn's last `report_progress(blocked_on=...)` call said it was
    /// waiting on. `None` means either it has never reported a block, or a
    /// later non-blocked note / completion cleared it.
    pub blocked_on: Option<String>,
    pub blocked_detail: Option<String>,
}

const COLS: &str = "id, platform, chat_id, thread_id, identity, prompt, status, created_at, completed_at, result_text, child_session_ids, cancel_token, kind, watch_interval_secs, watch_until, watch_last_fired, blocked_on, blocked_detail";

fn row_to_turn(row: &rusqlite::Row) -> rusqlite::Result<BackgroundTurnRow> {
    let child_json: Option<String> = row.get(10)?;
    let child_session_ids = child_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    Ok(BackgroundTurnRow {
        id: row.get(0)?,
        platform: row.get(1)?,
        chat_id: row.get(2)?,
        thread_id: row.get(3)?,
        identity: row.get(4)?,
        prompt: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        completed_at: row.get(8)?,
        result_text: row.get(9)?,
        child_session_ids,
        cancel_token: row.get(11)?,
        kind: row.get(12)?,
        watch_interval_secs: row.get(13)?,
        watch_until: row.get(14)?,
        watch_last_fired: row.get(15)?,
        blocked_on: row.get(16)?,
        blocked_detail: row.get(17)?,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn insert(
    conn: &Connection,
    id: &str,
    platform: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    identity: Option<&str>,
    prompt: &str,
    created_at: i64,
    cancel_token: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO background_turns (id, platform, chat_id, thread_id, identity, prompt, created_at, child_session_ids, cancel_token)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '[]', ?8)",
        params![id, platform, chat_id, thread_id, identity, prompt, created_at, cancel_token],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<BackgroundTurnRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM background_turns WHERE id = ?1"),
            params![id],
            row_to_turn,
        )
        .optional()?)
}

/// Floor on a watch interval: each firing dispatches a full LLM agent turn
/// (session build + model call + tool calls), so anything tighter than a few
/// minutes guarantees firings overlap and starve real interactive messages
/// for the same chat's turn slot. A `/watch 2 ...` set up on 2026-07-28 ran
/// unnoticed for two weeks, generated ~7800 background-turn rows, and
/// intermittently stole the chat's busy slot from the operator's own messages;
/// see agent-hq-debt.md for the incident.
pub const MIN_WATCH_INTERVAL_MINS: i64 = 5;
pub const MAX_WATCH_INTERVAL_MINS: i64 = 24 * 60;

/// Expiry applied when none is given: 30 days, not forever. The same
/// incident above ran with no expiry and nobody remembered `/unwatch` for two
/// weeks; a self-terminating default means a forgotten watch dies on its own
/// instead of running indefinitely. An explicit expiry still overrides this
/// up to `MAX_WATCH_EXPIRY_HOURS` (1y).
pub const DEFAULT_WATCH_EXPIRY_HOURS: i64 = 24 * 30;
pub const MAX_WATCH_EXPIRY_HOURS: i64 = 24 * 365;

/// Characters of a turn id shown to users; `get_by_prefix` resolves it back.
pub const SHORT_REF_LEN: usize = 8;
/// Shortest prefix `get_by_prefix` accepts, so a stray short word cannot match.
pub const MIN_PREFIX_LEN: usize = 6;

/// The user-facing short form of a turn id.
pub fn short_ref(id: &str) -> &str {
    id.get(..SHORT_REF_LEN).unwrap_or(id)
}

/// Resolve a short ref or full id. `Ok(None)` when nothing matches; an error
/// when the prefix is too short or matches more than one turn.
pub fn get_by_prefix(conn: &Connection, prefix: &str) -> Result<Option<BackgroundTurnRow>> {
    if prefix.chars().count() < MIN_PREFIX_LEN {
        anyhow::bail!(
            "turn ref `{prefix}` is too short, give at least {MIN_PREFIX_LEN} characters"
        );
    }
    // substr, not LIKE, so `%` and `_` in user input are matched literally.
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM background_turns WHERE substr(id, 1, length(?1)) = ?1 LIMIT 2"
    ))?;
    let mut rows = stmt
        .query_map(params![prefix], row_to_turn)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() > 1 {
        anyhow::bail!("turn ref `{prefix}` matches more than one turn, give more characters");
    }
    Ok(rows.pop())
}

/// Insert a recurring watch turn. `watch_until` is a unix epoch expiry
/// (None = no expiry); `watch_last_fired` starts NULL so the watch is due
/// on the scheduler's first poll after the initial synchronous dispatch.
#[allow(clippy::too_many_arguments)]
pub fn insert_watch(
    conn: &Connection,
    id: &str,
    platform: &str,
    chat_id: &str,
    thread_id: Option<&str>,
    identity: Option<&str>,
    prompt: &str,
    created_at: i64,
    interval_secs: i64,
    watch_until: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO background_turns (id, platform, chat_id, thread_id, identity, prompt, created_at, child_session_ids, kind, watch_interval_secs, watch_until)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '[]', 'watch', ?8, ?9)",
        params![id, platform, chat_id, thread_id, identity, prompt, created_at, interval_secs, watch_until],
    )?;
    Ok(())
}

/// Running watches whose next firing is due at or before `now`
/// (watch_last_fired + watch_interval_secs <= now; a NULL watch_last_fired
/// means the watch has never fired and is treated as created_at-based).
pub fn list_due_watches(conn: &Connection, now: i64) -> Result<Vec<BackgroundTurnRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM background_turns
         WHERE kind = 'watch' AND status = 'running'
           AND COALESCE(watch_last_fired, created_at) + watch_interval_secs <= ?1
         ORDER BY created_at ASC"
    ))?;
    let rows = stmt
        .query_map(params![now], row_to_turn)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Record that a watch fired at `fired_at`, pushing the next due time out by
/// one interval. Marked BEFORE dispatch so a crash cannot double-fire.
pub fn mark_watch_fired(conn: &Connection, id: &str, fired_at: i64) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET watch_last_fired = ?1 WHERE id = ?2",
        params![fired_at, id],
    )?;
    Ok(())
}

/// Cache a watch's last-delivered result text on the watch row itself (kind
/// 'watch', not the per-firing 'turn' rows), so the next firing can compare
/// against it and skip delivery when nothing changed. Deliberately does not
/// touch `status` — `list_due_watches` filters on `status = 'running'` and
/// this must never make a live watch stop being picked up.
pub fn update_watch_result(conn: &Connection, id: &str, result_text: &str) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET result_text = ?1 WHERE id = ?2",
        params![result_text, id],
    )?;
    Ok(())
}

/// Cancel a watch (/unwatch). 'cancelled' is terminal: list_running and
/// list_due_watches never pick the row up again.
pub fn mark_cancelled(conn: &Connection, id: &str, completed_at: i64) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET status = 'cancelled', completed_at = ?1 WHERE id = ?2",
        params![completed_at, id],
    )?;
    Ok(())
}

/// Turns still running (status = 'running'), oldest first so reapers and
/// recovery scans see the longest-detached work first.
pub fn list_running(conn: &Connection) -> Result<Vec<BackgroundTurnRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM background_turns WHERE status = 'running' ORDER BY created_at ASC"
    ))?;
    let rows = stmt
        .query_map([], row_to_turn)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn mark_completed(
    conn: &Connection,
    id: &str,
    result_text: &str,
    completed_at: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET status = 'completed', completed_at = ?1, result_text = ?2 WHERE id = ?3",
        params![completed_at, result_text, id],
    )?;
    Ok(())
}

pub fn mark_failed(conn: &Connection, id: &str, error: &str, completed_at: i64) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET status = 'failed', completed_at = ?1, result_text = ?2 WHERE id = ?3",
        params![completed_at, error, id],
    )?;
    Ok(())
}

pub fn mark_interrupted(conn: &Connection, id: &str, completed_at: i64) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET status = 'interrupted', completed_at = ?1 WHERE id = ?2",
        params![completed_at, id],
    )?;
    Ok(())
}

/// Move a turn back to 'running' (used by `resume <id>` when re-dispatching an
/// interrupted turn). Clears completed_at so reapers see it as live work again.
pub fn mark_running(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET status = 'running', completed_at = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

/// Most recent interrupted turns for one chat, newest first — what a bare
/// `resume` lists so the user can pick an id.
pub fn list_recent_interrupted(
    conn: &Connection,
    platform: &str,
    chat_id: &str,
    limit: usize,
) -> Result<Vec<BackgroundTurnRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM background_turns
         WHERE status = 'interrupted' AND platform = ?1 AND chat_id = ?2
         ORDER BY COALESCE(completed_at, created_at) DESC
         LIMIT ?3"
    ))?;
    let rows = stmt
        .query_map(params![platform, chat_id, limit as i64], row_to_turn)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Append a harness session id to the turn's child list. Idempotent: a session
/// id already recorded is not added again.
pub fn attach_child_session(conn: &Connection, id: &str, session_id: &str) -> Result<()> {
    let mut children = get(conn, id)?
        .map(|t| t.child_session_ids)
        .unwrap_or_default();
    if !children.iter().any(|c| c == session_id) {
        children.push(session_id.to_string());
    }
    let json = serde_json::to_string(&children)?;
    conn.execute(
        "UPDATE background_turns SET child_session_ids = ?1 WHERE id = ?2",
        params![json, id],
    )?;
    Ok(())
}

/// Record that this turn's agent has stopped to wait on someone, from a
/// `report_progress(blocked_on=...)` call. Overwrites any previous block —
/// only the most recent one matters for the reconciler's FYI text.
pub fn set_blocked(conn: &Connection, id: &str, on: &str, detail: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET blocked_on = ?1, blocked_detail = ?2 WHERE id = ?3",
        params![on, detail, id],
    )?;
    Ok(())
}

/// Clear a recorded block: the agent resumed (ordinary progress, an
/// assumption-based resume, or completion all mean it is no longer waiting).
pub fn clear_blocked(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE background_turns SET blocked_on = NULL, blocked_detail = NULL WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::Database;

    fn seed(c: &Connection, id: &str) -> Result<()> {
        insert(
            c,
            id,
            "telegram",
            "chat-42",
            Some("thread-7"),
            Some("alex"),
            "refactor the relay ack window",
            1_700_000_000,
            Some("cancel-tok"),
        )
    }

    #[test]
    fn insert_get_roundtrip() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            let t = get(c, "bt-1")?.unwrap();
            assert_eq!(t.platform, "telegram");
            assert_eq!(t.chat_id, "chat-42");
            assert_eq!(t.thread_id.as_deref(), Some("thread-7"));
            assert_eq!(t.identity.as_deref(), Some("alex"));
            assert_eq!(t.status, STATUS_RUNNING);
            assert_eq!(t.created_at, 1_700_000_000);
            assert!(t.completed_at.is_none());
            assert!(t.result_text.is_none());
            assert!(t.child_session_ids.is_empty());
            assert_eq!(t.cancel_token.as_deref(), Some("cancel-tok"));
            assert!(get(c, "bt-missing")?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn get_by_prefix_resolves_unique_refs_only() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "abcdef12-0000")?;
            seed(c, "abcdef99-0000")?;
            assert_eq!(get_by_prefix(c, "abcdef12")?.unwrap().id, "abcdef12-0000");
            assert_eq!(
                get_by_prefix(c, "abcdef12-0000")?.unwrap().id,
                "abcdef12-0000"
            );
            assert!(
                get_by_prefix(c, "abcdef")
                    .unwrap_err()
                    .to_string()
                    .contains("more than one")
            );
            assert!(
                get_by_prefix(c, "abcde")
                    .unwrap_err()
                    .to_string()
                    .contains("too short")
            );
            assert!(get_by_prefix(c, "zzzzzz")?.is_none());
            assert!(get_by_prefix(c, "%%%%%%")?.is_none());
            assert_eq!(short_ref("abcdef12-0000"), "abcdef12");
            assert_eq!(short_ref("bt-1"), "bt-1");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn list_running_only_returns_running() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            seed(c, "bt-2")?;
            mark_completed(c, "bt-2", "done", 1_700_000_100)?;
            let running = list_running(c)?;
            assert_eq!(running.len(), 1);
            assert_eq!(running[0].id, "bt-1");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn mark_completed_sets_fields() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            mark_completed(c, "bt-1", "shipped it", 1_700_000_100)?;
            let t = get(c, "bt-1")?.unwrap();
            assert_eq!(t.status, STATUS_COMPLETED);
            assert_eq!(t.completed_at, Some(1_700_000_100));
            assert_eq!(t.result_text.as_deref(), Some("shipped it"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn mark_failed_and_interrupted_set_status() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            seed(c, "bt-2")?;
            mark_failed(c, "bt-1", "harness crashed", 1_700_000_100)?;
            mark_interrupted(c, "bt-2", 1_700_000_200)?;
            let t1 = get(c, "bt-1")?.unwrap();
            assert_eq!(t1.status, STATUS_FAILED);
            assert_eq!(t1.result_text.as_deref(), Some("harness crashed"));
            let t2 = get(c, "bt-2")?.unwrap();
            assert_eq!(t2.status, STATUS_INTERRUPTED);
            assert_eq!(t2.completed_at, Some(1_700_000_200));
            assert!(list_running(c)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn mark_running_reactivates_and_clears_completed_at() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            mark_interrupted(c, "bt-1", 1_700_000_100)?;
            mark_running(c, "bt-1")?;
            let t = get(c, "bt-1")?.unwrap();
            assert_eq!(t.status, STATUS_RUNNING);
            assert!(t.completed_at.is_none());
            assert_eq!(list_running(c)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn list_recent_interrupted_filters_by_chat_and_orders_newest_first() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            seed(c, "bt-2")?;
            seed(c, "bt-3")?;
            mark_interrupted(c, "bt-1", 1_700_000_100)?;
            mark_interrupted(c, "bt-2", 1_700_000_300)?;
            // bt-3 stays running: must not appear.
            // A different chat's interrupted turn must not leak in either.
            insert(
                c,
                "bt-other",
                "telegram",
                "chat-99",
                None,
                None,
                "unrelated",
                1_700_000_000,
                None,
            )?;
            mark_interrupted(c, "bt-other", 1_700_000_400)?;

            let rows = list_recent_interrupted(c, "telegram", "chat-42", 5)?;
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].id, "bt-2");
            assert_eq!(rows[1].id, "bt-1");

            let limited = list_recent_interrupted(c, "telegram", "chat-42", 1)?;
            assert_eq!(limited.len(), 1);
            assert_eq!(limited[0].id, "bt-2");

            assert!(list_recent_interrupted(c, "discord", "chat-42", 5)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    /// The reason this column pair exists: by the time the reconciler runs,
    /// the process that fired the transient `ProgressEvent` is long gone, so
    /// the block reason has to survive here or the reconciler has nothing to
    /// read.
    #[test]
    fn set_blocked_persists_who_and_why() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            set_blocked(c, "bt-1", "Alex", Some("which OpenRouter key to use"))?;
            let t = get(c, "bt-1")?.unwrap();
            assert_eq!(t.blocked_on.as_deref(), Some("Alex"));
            assert_eq!(
                t.blocked_detail.as_deref(),
                Some("which OpenRouter key to use")
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn clear_blocked_removes_both_fields() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            set_blocked(c, "bt-1", "hermes", None)?;
            clear_blocked(c, "bt-1")?;
            let t = get(c, "bt-1")?.unwrap();
            assert!(t.blocked_on.is_none());
            assert!(t.blocked_detail.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_fresh_turn_has_no_recorded_block() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            let t = get(c, "bt-1")?.unwrap();
            assert!(t.blocked_on.is_none());
            assert!(t.blocked_detail.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn attach_child_session_appends_without_duplicates() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            attach_child_session(c, "bt-1", "hs-1")?;
            attach_child_session(c, "bt-1", "hs-2")?;
            attach_child_session(c, "bt-1", "hs-1")?; // duplicate: no-op
            let t = get(c, "bt-1")?.unwrap();
            assert_eq!(t.child_session_ids, vec!["hs-1", "hs-2"]);
            Ok(())
        })
        .unwrap();
    }

    fn seed_watch(c: &Connection, id: &str, interval_secs: i64) -> Result<()> {
        insert_watch(
            c,
            id,
            "telegram",
            "chat-42",
            Some("thread-7"),
            Some("alex"),
            "check whether the review is done",
            1_700_000_000,
            interval_secs,
            None,
        )
    }

    #[test]
    fn plain_insert_maps_with_turn_kind_and_null_watch_fields() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed(c, "bt-1")?;
            let t = get(c, "bt-1")?.unwrap();
            assert_eq!(t.kind, KIND_TURN);
            assert!(t.watch_interval_secs.is_none());
            assert!(t.watch_until.is_none());
            assert!(t.watch_last_fired.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn list_due_watches_returns_due_only() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed_watch(c, "w-due", 60)?; // due at 1_700_000_060
            seed_watch(c, "w-later", 3600)?; // due at 1_700_003_600
            // A plain turn is never a due watch even when old.
            seed(c, "bt-1")?;

            let due = list_due_watches(c, 1_700_000_100)?;
            assert_eq!(due.len(), 1);
            assert_eq!(due[0].id, "w-due");
            assert_eq!(due[0].kind, KIND_WATCH);
            assert_eq!(due[0].watch_interval_secs, Some(60));

            let due_later = list_due_watches(c, 1_700_003_600)?;
            assert_eq!(due_later.len(), 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn mark_watch_fired_pushes_watch_out_of_due_set() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed_watch(c, "w-1", 60)?;
            assert_eq!(list_due_watches(c, 1_700_000_100)?.len(), 1);
            mark_watch_fired(c, "w-1", 1_700_000_100)?;
            let t = get(c, "w-1")?.unwrap();
            assert_eq!(t.watch_last_fired, Some(1_700_000_100));
            assert!(list_due_watches(c, 1_700_000_100)?.is_empty());
            // Due again one interval later.
            assert_eq!(list_due_watches(c, 1_700_000_160)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn mark_cancelled_removes_from_running_and_due() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            seed_watch(c, "w-1", 60)?;
            assert_eq!(list_running(c)?.len(), 1);
            mark_cancelled(c, "w-1", 1_700_000_050)?;
            let t = get(c, "w-1")?.unwrap();
            assert_eq!(t.status, STATUS_CANCELLED);
            assert_eq!(t.completed_at, Some(1_700_000_050));
            assert!(list_running(c)?.is_empty());
            assert!(list_due_watches(c, 1_700_000_100)?.is_empty());
            Ok(())
        })
        .unwrap();
    }
}
