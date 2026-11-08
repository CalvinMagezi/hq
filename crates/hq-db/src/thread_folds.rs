//! Fold blocks over a chat thread's older messages (`docs/plans/context-fold.md`).
//!
//! The originals stay in `chat_messages`. A block names the first and last message it stands
//! for, and the model is sent the block instead of them. Blocks are append-only: a later fold
//! covers only messages after the newest block, and a tier-2 block absorbs older blocks by
//! marking them superseded rather than editing them. An edit or regenerate that deletes
//! messages deletes the blocks that reach them (`drop_reaching`, called by
//! `chat::delete_messages_from`), and a block it absorbed comes back.

use anyhow::{Result, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::chat::{ChatMessage, StoredMessage};

/// One fold block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fold {
    pub id: String,
    pub thread_id: String,
    /// 1: summarizes messages. 2: distills older blocks.
    pub tier: i64,
    pub first_msg_id: String,
    pub last_msg_id: String,
    pub message_count: i64,
    pub summary: String,
    pub source_tokens: i64,
    pub summary_tokens: i64,
    /// The range held content from someone other than the owner (an MCP caller).
    pub tainted: bool,
    pub model: String,
    pub superseded_by: Option<String>,
    pub created_at: String,
}

impl Fold {
    /// The short reference the model sees and passes back to restore the block.
    pub fn short_ref(&self) -> String {
        format!("f:{}", short(&self.id))
    }
}

/// What a new block needs. The id, time and supersession are filled in on insert.
#[derive(Debug, Clone)]
pub struct NewFold<'a> {
    pub thread_id: &'a str,
    pub tier: i64,
    pub first_msg_id: &'a str,
    pub last_msg_id: &'a str,
    pub message_count: i64,
    pub summary: &'a str,
    pub source_tokens: i64,
    pub summary_tokens: i64,
    pub tainted: bool,
    pub model: &'a str,
}

/// The first 8 characters of an id, the form refs use in context.
pub fn short(id: &str) -> &str {
    let end = id.char_indices().nth(8).map_or(id.len(), |(i, _)| i);
    &id[..end]
}

/// The short reference of a message, as fold blocks and restored ranges print it.
pub fn message_ref(message_id: &str) -> String {
    format!("m:{}", short(message_id))
}

const FOLD_COLUMNS: &str = "f.id, f.thread_id, f.tier, f.first_msg_id, f.last_msg_id, \
     f.message_count, f.summary, f.source_tokens, f.summary_tokens, f.tainted, f.model, \
     f.superseded_by, f.created_at";

fn fold_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Fold> {
    Ok(Fold {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        tier: row.get(2)?,
        first_msg_id: row.get(3)?,
        last_msg_id: row.get(4)?,
        message_count: row.get(5)?,
        summary: row.get(6)?,
        source_tokens: row.get(7)?,
        summary_tokens: row.get(8)?,
        tainted: row.get::<_, i64>(9)? != 0,
        model: row.get(10)?,
        superseded_by: row.get(11)?,
        created_at: row.get(12)?,
    })
}

/// The blocks the model is sent for a thread, oldest range first. A block whose first message
/// is gone is left out (it cannot be placed).
pub fn active(conn: &Connection, thread_id: &str) -> Result<Vec<Fold>> {
    let sql = format!(
        "SELECT {FOLD_COLUMNS} FROM thread_folds f
           JOIN chat_messages m ON m.message_id = f.first_msg_id AND m.thread_id = f.thread_id
          WHERE f.thread_id = ?1 AND f.superseded_by IS NULL
          ORDER BY m.created_at ASC, m.rowid ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![thread_id], fold_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// One block by id.
pub fn get(conn: &Connection, id: &str) -> Result<Option<Fold>> {
    let sql = format!("SELECT {FOLD_COLUMNS} FROM thread_folds f WHERE f.id = ?1");
    conn.query_row(&sql, params![id], fold_from_row)
        .optional()
        .map_err(Into::into)
}

/// A block by the short ref the model saw (`f:1a2b3c4d`, or the bare prefix), in one thread or
/// any. Superseded blocks still resolve, so a ref quoted in an older summary keeps working.
pub fn find_by_ref(
    conn: &Connection,
    thread_id: Option<&str>,
    reference: &str,
) -> Result<Option<Fold>> {
    let prefix = reference.trim().trim_start_matches("f:");
    if prefix.len() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Ok(None);
    }
    let sql = format!(
        "SELECT {FOLD_COLUMNS} FROM thread_folds f
          WHERE substr(f.id, 1, length(?1)) = ?1 AND (?2 IS NULL OR f.thread_id = ?2)
          ORDER BY f.created_at DESC LIMIT 1"
    );
    conn.query_row(&sql, params![prefix, thread_id], fold_from_row)
        .optional()
        .map_err(Into::into)
}

/// Where a message sits in its thread's order.
fn position(conn: &Connection, thread_id: &str, message_id: &str) -> Result<Option<(String, i64)>> {
    conn.query_row(
        "SELECT created_at, rowid FROM chat_messages WHERE thread_id = ?1 AND message_id = ?2",
        params![thread_id, message_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

fn stored_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    let meta: Option<String> = row.get(5)?;
    Ok(StoredMessage {
        message: ChatMessage {
            message_id: row.get(0)?,
            thread_id: row.get(1)?,
            role: row.get(2)?,
            content: row.get(3)?,
            created_at: row.get(4)?,
        },
        meta: meta.and_then(|m| serde_json::from_str(&m).ok()),
    })
}

/// The messages after `after` (all of them when `None`), oldest first, at most `limit` of the
/// newest. An `after` that is not in the thread returns nothing.
pub fn messages_after(
    conn: &Connection,
    thread_id: &str,
    after: Option<&str>,
    limit: i64,
) -> Result<Vec<StoredMessage>> {
    let cursor = match after {
        Some(id) => match position(conn, thread_id, id)? {
            Some(pos) => Some(pos),
            None => return Ok(Vec::new()),
        },
        None => None,
    };
    let (created, rowid) = cursor.unzip();
    let mut stmt = conn.prepare(
        "SELECT message_id, thread_id, role, content, created_at, meta FROM (
            SELECT rowid AS rid, message_id, thread_id, role, content, created_at, meta
              FROM chat_messages
             WHERE thread_id = ?1 AND (?2 IS NULL OR (created_at, rowid) > (?2, ?3))
             ORDER BY created_at DESC, rowid DESC
             LIMIT ?4
         ) ORDER BY created_at ASC, rid ASC",
    )?;
    let rows = stmt.query_map(params![thread_id, created, rowid, limit], stored_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// The messages from `first` to `last` inclusive, oldest first, at most `limit` from the start.
pub fn messages_between(
    conn: &Connection,
    thread_id: &str,
    first: &str,
    last: &str,
    limit: i64,
) -> Result<Vec<StoredMessage>> {
    let (Some((c1, r1)), Some((c2, r2))) = (
        position(conn, thread_id, first)?,
        position(conn, thread_id, last)?,
    ) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(
        "SELECT message_id, thread_id, role, content, created_at, meta
           FROM chat_messages
          WHERE thread_id = ?1 AND (created_at, rowid) >= (?2, ?3) AND (created_at, rowid) <= (?4, ?5)
          ORDER BY created_at ASC, rowid ASC
          LIMIT ?6",
    )?;
    let rows = stmt.query_map(params![thread_id, c1, r1, c2, r2, limit], stored_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Saves a tier-1 block. Refused when its range starts at or before the end of the newest
/// active block, so two folds racing on one thread cannot both land.
pub fn insert(conn: &Connection, new: &NewFold<'_>) -> Result<Fold> {
    let tx = conn.unchecked_transaction()?;
    if let Some(newest) = active(&tx, new.thread_id)?.last() {
        let (Some(start), Some(end)) = (
            position(&tx, new.thread_id, new.first_msg_id)?,
            position(&tx, new.thread_id, &newest.last_msg_id)?,
        ) else {
            bail!("fold range is not in the thread");
        };
        if start <= end {
            bail!("fold range overlaps block {}", newest.id);
        }
    }
    let fold = write(&tx, new)?;
    tx.commit()?;
    Ok(fold)
}

/// Saves a tier-2 block that absorbs `absorbed` (all active blocks of the thread). They are
/// marked superseded by it, never deleted, so their refs keep resolving.
pub fn insert_distilled(conn: &Connection, new: &NewFold<'_>, absorbed: &[String]) -> Result<Fold> {
    let tx = conn.unchecked_transaction()?;
    let live: Vec<String> = active(&tx, new.thread_id)?
        .into_iter()
        .map(|f| f.id)
        .collect();
    if absorbed.is_empty() || absorbed.iter().any(|id| !live.contains(id)) {
        bail!("a block to distill is no longer active");
    }
    let fold = write(&tx, new)?;
    for id in absorbed {
        tx.execute(
            "UPDATE thread_folds SET superseded_by = ?1 WHERE id = ?2",
            params![fold.id, id],
        )?;
    }
    tx.commit()?;
    Ok(fold)
}

fn write(conn: &Connection, new: &NewFold<'_>) -> Result<Fold> {
    // The summary call takes seconds; an edit in that time may have deleted part of the range.
    let (Some(start), Some(end)) = (
        position(conn, new.thread_id, new.first_msg_id)?,
        position(conn, new.thread_id, new.last_msg_id)?,
    ) else {
        bail!("a message in the fold range is gone");
    };
    if start > end {
        bail!("the fold range ends before it starts");
    }
    let fold = Fold {
        id: Uuid::new_v4().to_string(),
        thread_id: new.thread_id.to_string(),
        tier: new.tier,
        first_msg_id: new.first_msg_id.to_string(),
        last_msg_id: new.last_msg_id.to_string(),
        message_count: new.message_count,
        summary: new.summary.to_string(),
        source_tokens: new.source_tokens,
        summary_tokens: new.summary_tokens,
        tainted: new.tainted,
        model: new.model.to_string(),
        superseded_by: None,
        created_at: Utc::now().to_rfc3339(),
    };
    conn.execute(
        "INSERT INTO thread_folds (id, thread_id, tier, first_msg_id, last_msg_id, message_count,
             summary, source_tokens, summary_tokens, tainted, model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            fold.id,
            fold.thread_id,
            fold.tier,
            fold.first_msg_id,
            fold.last_msg_id,
            fold.message_count,
            fold.summary,
            fold.source_tokens,
            fold.summary_tokens,
            i64::from(fold.tainted),
            fold.model,
            fold.created_at,
        ],
    )?;
    Ok(fold)
}

/// Deletes every block whose range reaches `from` or later; call it before those messages are
/// deleted. A block that a deleted block had absorbed becomes active again, since its own range
/// is still intact. Returns how many blocks went.
pub fn drop_reaching(conn: &Connection, thread_id: &str, from: &str) -> Result<usize> {
    let Some(cut) = position(conn, thread_id, from)? else {
        return Ok(0);
    };
    let mut stmt = conn.prepare(
        "SELECT f.id FROM thread_folds f
           LEFT JOIN chat_messages m ON m.message_id = f.last_msg_id AND m.thread_id = f.thread_id
          WHERE f.thread_id = ?1 AND (m.message_id IS NULL OR (m.created_at, m.rowid) >= (?2, ?3))",
    )?;
    let doomed: Vec<String> = stmt
        .query_map(params![thread_id, cut.0, cut.1], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in &doomed {
        conn.execute(
            "UPDATE thread_folds SET superseded_by = NULL WHERE superseded_by = ?1",
            params![id],
        )?;
        conn.execute("DELETE FROM thread_folds WHERE id = ?1", params![id])?;
    }
    Ok(doomed.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{add_message, create_thread};

    fn setup() -> (Connection, String, Vec<String>) {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        let t = create_thread(&conn, "t", "user", "user").unwrap().thread_id;
        let ids = (0..10)
            .map(|i| {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                add_message(&conn, &t, role, &format!("message {i}"))
                    .unwrap()
                    .message_id
            })
            .collect();
        (conn, t, ids)
    }

    fn new<'a>(t: &'a str, first: &'a str, last: &'a str) -> NewFold<'a> {
        NewFold {
            thread_id: t,
            tier: 1,
            first_msg_id: first,
            last_msg_id: last,
            message_count: 2,
            summary: "summary",
            source_tokens: 100,
            summary_tokens: 10,
            tainted: false,
            model: "test",
        }
    }

    #[test]
    fn blocks_are_ordered_and_the_tail_starts_after_the_newest() {
        let (conn, t, ids) = setup();
        insert(&conn, &new(&t, &ids[0], &ids[3])).unwrap();
        insert(&conn, &new(&t, &ids[4], &ids[5])).unwrap();
        let blocks = active(&conn, &t).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].first_msg_id, ids[0]);
        let tail = messages_after(&conn, &t, Some(&blocks[1].last_msg_id), 100).unwrap();
        assert_eq!(tail.len(), 4);
        assert_eq!(tail[0].message.message_id, ids[6]);
    }

    #[test]
    fn an_overlapping_fold_is_refused() {
        let (conn, t, ids) = setup();
        insert(&conn, &new(&t, &ids[0], &ids[3])).unwrap();
        assert!(insert(&conn, &new(&t, &ids[2], &ids[5])).is_err());
        assert!(insert(&conn, &new(&t, &ids[3], &ids[5])).is_err());
        assert_eq!(active(&conn, &t).unwrap().len(), 1);
    }

    #[test]
    fn a_fold_over_messages_an_edit_deleted_is_refused() {
        let (conn, t, ids) = setup();
        // An edit lands while the summary call runs: messages 3 onward are gone.
        crate::chat::delete_messages_from(&conn, &t, &ids[3]).unwrap();
        assert!(insert(&conn, &new(&t, &ids[0], &ids[3])).is_err());
        assert!(insert(&conn, &new(&t, &ids[2], &ids[0])).is_err());
        assert!(active(&conn, &t).unwrap().is_empty());
    }

    #[test]
    fn distilling_supersedes_and_an_edit_brings_absorbed_blocks_back() {
        let (conn, t, ids) = setup();
        let a = insert(&conn, &new(&t, &ids[0], &ids[1])).unwrap();
        let b = insert(&conn, &new(&t, &ids[2], &ids[5])).unwrap();
        let mut d = new(&t, &ids[0], &ids[5]);
        d.tier = 2;
        let big = insert_distilled(&conn, &d, &[a.id.clone(), b.id.clone()]).unwrap();
        let live = active(&conn, &t).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, big.id);
        assert_eq!(
            get(&conn, &a.id).unwrap().unwrap().superseded_by,
            Some(big.id.clone())
        );

        // Editing message 4 deletes the tier-2 block and block b, which reach it; a returns.
        assert_eq!(drop_reaching(&conn, &t, &ids[4]).unwrap(), 2);
        let live = active(&conn, &t).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, a.id);
        assert_eq!(live[0].superseded_by, None);
    }

    #[test]
    fn delete_messages_from_drops_the_blocks_it_cuts() {
        let (conn, t, ids) = setup();
        insert(&conn, &new(&t, &ids[0], &ids[3])).unwrap();
        insert(&conn, &new(&t, &ids[4], &ids[7])).unwrap();
        crate::chat::delete_messages_from(&conn, &t, &ids[6]).unwrap();
        let live = active(&conn, &t).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].last_msg_id, ids[3]);
    }

    #[test]
    fn refs_resolve_by_prefix_and_reject_junk() {
        let (conn, t, ids) = setup();
        let f = insert(&conn, &new(&t, &ids[0], &ids[1])).unwrap();
        let found = find_by_ref(&conn, Some(&t), &f.short_ref())
            .unwrap()
            .unwrap();
        assert_eq!(found.id, f.id);
        assert!(
            find_by_ref(&conn, Some("other"), &f.short_ref())
                .unwrap()
                .is_none()
        );
        assert!(find_by_ref(&conn, None, "f:%").unwrap().is_none());
        assert!(find_by_ref(&conn, None, "ab").unwrap().is_none());
    }

    #[test]
    fn a_range_reads_back_inclusive_and_bounded() {
        let (conn, t, ids) = setup();
        let all = messages_between(&conn, &t, &ids[2], &ids[5], 100).unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[3].message.message_id, ids[5]);
        assert_eq!(
            messages_between(&conn, &t, &ids[2], &ids[5], 2)
                .unwrap()
                .len(),
            2
        );
        assert!(
            messages_between(&conn, &t, "nope", &ids[5], 9)
                .unwrap()
                .is_empty()
        );
    }
}
