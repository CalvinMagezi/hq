//! What the owner can take back: the text a note had before HQ changed it, who deleted a Trash
//! item, and the record of an item removed for good. Written by the runtime, never from a
//! note's own text or a tool argument.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// Longest earlier text kept for a note. A larger note keeps none, so Undo is not offered for it.
pub const MAX_PREV_BYTES: usize = 64 * 1024;

/// Who put a note in the Trash, as the Trash list says it.
pub const DELETED_BY_YOU: &str = "you";
pub const DELETED_BY_HQ: &str = "hq";

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The earlier text of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prev {
    pub text: String,
    pub text_hash: String,
    /// The hash of the HQ version this text came before, once that write is recorded.
    pub for_hash: Option<String>,
    pub saved_at: String,
}

/// Keep `text` as the earlier text of `path`, replacing what was kept. A text over the cap keeps
/// nothing (and drops the old one, which no longer describes the note). Returns whether it kept it.
pub fn keep(conn: &Connection, path: &str, text: &str) -> Result<bool> {
    if text.len() > MAX_PREV_BYTES {
        remove(conn, path)?;
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO note_prev (path, text, text_hash, saved_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET text = excluded.text, text_hash = excluded.text_hash,
           for_hash = NULL, saved_at = excluded.saved_at",
        params![path, text, crate::search::content_hash(text), now()],
    )?;
    Ok(true)
}

/// The write about to happen was checked against the kept text (it is the owner's version, or was
/// kept just now), so the record of that write may tie to it.
pub fn arm(conn: &Connection, path: &str) -> Result<()> {
    conn.execute(
        "UPDATE note_prev SET for_hash = NULL WHERE path = ?1",
        [path],
    )?;
    Ok(())
}

/// HQ's write of `path` produced the version hashing to `hash`. A kept text that was armed for this
/// write is tied to it. One that was not (the write did not go through a keeping tool, so the text
/// may be older than the owner's current version) is dropped, never served.
pub fn tie_or_drop(conn: &Connection, path: &str, hash: &str) -> Result<()> {
    let tied = conn.execute(
        "UPDATE note_prev SET for_hash = ?2 WHERE path = ?1 AND for_hash IS NULL",
        [path, hash],
    )?;
    if tied == 0 {
        remove(conn, path)?;
    }
    Ok(())
}

/// How long a kept text lives if the owner never decides.
pub const PREV_RETENTION_DAYS: i64 = 30;
/// Most bytes of kept text across all notes; the oldest go first.
pub const PREV_TOTAL_BYTES: i64 = 5 * 1024 * 1024;

/// The daily clean-up: drop kept texts past their retention, then the oldest until the table is
/// within its size bound. Returns how many rows went.
pub fn prune_prev(conn: &Connection, retention_days: i64, max_total_bytes: i64) -> Result<usize> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(retention_days)).to_rfc3339();
    let mut dropped = conn.execute("DELETE FROM note_prev WHERE saved_at < ?1", [cutoff])?;
    loop {
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(LENGTH(text)), 0) FROM note_prev",
            [],
            |r| r.get(0),
        )?;
        if total <= max_total_bytes {
            return Ok(dropped);
        }
        let gone = conn.execute(
            "DELETE FROM note_prev WHERE path = (SELECT path FROM note_prev ORDER BY saved_at, path LIMIT 1)",
            [],
        )?;
        if gone == 0 {
            return Ok(dropped);
        }
        dropped += gone;
    }
}

pub fn get(conn: &Connection, path: &str) -> Result<Option<Prev>> {
    Ok(conn
        .query_row(
            "SELECT text, text_hash, for_hash, saved_at FROM note_prev WHERE path = ?1",
            [path],
            |r| {
                Ok(Prev {
                    text: r.get(0)?,
                    text_hash: r.get(1)?,
                    for_hash: r.get(2)?,
                    saved_at: r.get(3)?,
                })
            },
        )
        .optional()?)
}

pub fn remove(conn: &Connection, path: &str) -> Result<()> {
    conn.execute("DELETE FROM note_prev WHERE path = ?1", [path])?;
    Ok(())
}

/// The note moved: its earlier text goes with it.
pub fn move_path(conn: &Connection, from: &str, to: &str) -> Result<()> {
    if from == to {
        return Ok(());
    }
    conn.execute("DELETE FROM note_prev WHERE path = ?1", [to])?;
    conn.execute("UPDATE note_prev SET path = ?2 WHERE path = ?1", [from, to])?;
    Ok(())
}

/// Note who put the Trash item there. Anything but `hq` is stored as the owner.
pub fn record_deleted_by(conn: &Connection, trash_path: &str, by_hq: bool) -> Result<()> {
    let by = if by_hq { DELETED_BY_HQ } else { DELETED_BY_YOU };
    conn.execute(
        "INSERT OR REPLACE INTO note_trash_meta (trash_path, deleted_by, deleted_at)
         VALUES (?1, ?2, ?3)",
        params![trash_path, by, now()],
    )?;
    Ok(())
}

/// `hq` or `you`; an item nobody recorded reads as `you`.
pub fn deleted_by(conn: &Connection, trash_path: &str) -> Result<&'static str> {
    let by: Option<String> = conn
        .query_row(
            "SELECT deleted_by FROM note_trash_meta WHERE trash_path = ?1",
            [trash_path],
            |r| r.get(0),
        )
        .optional()?;
    Ok(if by.as_deref() == Some(DELETED_BY_HQ) {
        DELETED_BY_HQ
    } else {
        DELETED_BY_YOU
    })
}

pub fn forget_trash_item(conn: &Connection, trash_path: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM note_trash_meta WHERE trash_path = ?1",
        [trash_path],
    )?;
    Ok(())
}

/// A note is gone for good: its records of who wrote it, when the owner opened it and its earlier
/// text go, and one line (path and time, no text) says it was removed. Only records of a deleted
/// note are touched, so a note since created under the same name keeps its own. Whether such a
/// note exists is the caller's to say from the file system (`live_note_exists`), not from the
/// records, because a live note may have none.
pub fn purge_records(conn: &Connection, path: &str, live_note_exists: bool) -> Result<()> {
    if !live_note_exists {
        conn.execute(
            "DELETE FROM note_provenance WHERE path = ?1 AND deleted_at IS NOT NULL",
            [path],
        )?;
        conn.execute("DELETE FROM note_seen WHERE path = ?1", [path])?;
        remove(conn, path)?;
    }
    conn.execute(
        "INSERT INTO note_purge_log (path, purged_at) VALUES (?1, ?2)",
        params![path, now()],
    )?;
    Ok(())
}

/// Drop the who-deleted rows older than `cutoff` (an RFC 3339 time), which the daemon's purge of
/// old Trash days has already removed the files for.
pub fn prune_trash_meta(conn: &Connection, cutoff: &str) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM note_trash_meta WHERE deleted_at < ?1",
        [cutoff],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::provenance::{Attribution, SourceKind, mark_deleted, record};

    fn db() -> Database {
        Database::open_memory().unwrap()
    }

    #[test]
    fn keeps_one_text_per_note_and_replaces_it() {
        db().with_conn(|c| {
            assert!(keep(c, "Notebooks/a.md", "one")?);
            assert!(keep(c, "Notebooks/a.md", "two")?);
            assert_eq!(get(c, "Notebooks/a.md")?.unwrap().text, "two");
            remove(c, "Notebooks/a.md")?;
            assert!(get(c, "Notebooks/a.md")?.is_none());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_text_over_the_cap_keeps_nothing_and_drops_the_old_one() {
        db().with_conn(|c| {
            keep(c, "Notebooks/a.md", "small")?;
            let big = "x".repeat(MAX_PREV_BYTES + 1);
            assert!(!keep(c, "Notebooks/a.md", &big)?);
            assert!(get(c, "Notebooks/a.md")?.is_none());
            assert!(keep(c, "Notebooks/a.md", &"x".repeat(MAX_PREV_BYTES))?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_earlier_text_follows_a_move() {
        db().with_conn(|c| {
            keep(c, "Notebooks/a.md", "old")?;
            crate::provenance::move_path(c, "Notebooks/a.md", "Notebooks/b.md")?;
            assert!(get(c, "Notebooks/a.md")?.is_none());
            assert_eq!(get(c, "Notebooks/b.md")?.unwrap().text, "old");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn who_deleted_defaults_to_you() {
        db().with_conn(|c| {
            assert_eq!(deleted_by(c, "_trash/2026-01-01/Notebooks/a.md")?, "you");
            record_deleted_by(c, "_trash/2026-01-01/Notebooks/a.md", true)?;
            assert_eq!(deleted_by(c, "_trash/2026-01-01/Notebooks/a.md")?, "hq");
            forget_trash_item(c, "_trash/2026-01-01/Notebooks/a.md")?;
            assert_eq!(deleted_by(c, "_trash/2026-01-01/Notebooks/a.md")?, "you");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn purging_removes_a_deleted_notes_records_and_leaves_a_live_ones() {
        db().with_conn(|c| {
            let who = Attribution {
                actor_id: Some("a"),
                source_kind: Some(SourceKind::Agent),
                session_id: Some("s"),
                task_id: None,
            };
            record(c, "Notebooks/gone.md", "h1", &who)?;
            mark_deleted(c, "Notebooks/gone.md")?;
            keep(c, "Notebooks/gone.md", "old")?;
            purge_records(c, "Notebooks/gone.md", false)?;
            let rows: i64 = c.query_row(
                "SELECT COUNT(*) FROM note_provenance WHERE path = 'Notebooks/gone.md'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(rows, 0);
            assert!(get(c, "Notebooks/gone.md")?.is_none());

            record(c, "Notebooks/back.md", "h2", &who)?;
            purge_records(c, "Notebooks/back.md", true)?;
            let rows: i64 = c.query_row(
                "SELECT COUNT(*) FROM note_provenance WHERE path = 'Notebooks/back.md'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(rows, 1, "a live note keeps its records");
            let logged: i64 =
                c.query_row("SELECT COUNT(*) FROM note_purge_log", [], |r| r.get(0))?;
            assert_eq!(logged, 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_kept_text_not_armed_for_the_write_is_dropped_not_tied() {
        db().with_conn(|c| {
            keep(c, "Notebooks/a.md", "old")?;
            tie_or_drop(c, "Notebooks/a.md", "h1")?;
            assert_eq!(
                get(c, "Notebooks/a.md")?.unwrap().for_hash.as_deref(),
                Some("h1")
            );
            // The next write did not go through a keeping tool: the text may be stale.
            tie_or_drop(c, "Notebooks/a.md", "h2")?;
            assert!(get(c, "Notebooks/a.md")?.is_none());
            keep(c, "Notebooks/a.md", "old")?;
            tie_or_drop(c, "Notebooks/a.md", "h1")?;
            arm(c, "Notebooks/a.md")?;
            tie_or_drop(c, "Notebooks/a.md", "h2")?;
            assert_eq!(
                get(c, "Notebooks/a.md")?.unwrap().for_hash.as_deref(),
                Some("h2")
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn old_texts_expire_and_the_table_stays_within_its_bound() {
        db().with_conn(|c| {
            keep(c, "Notebooks/old.md", "old")?;
            c.execute(
                "UPDATE note_prev SET saved_at = '2000-01-01T00:00:00+00:00'",
                [],
            )?;
            keep(c, "Notebooks/a.md", &"a".repeat(600))?;
            c.execute(
                "UPDATE note_prev SET saved_at = '2999-01-01T00:00:00+00:00' WHERE path = 'Notebooks/a.md'",
                [],
            )?;
            keep(c, "Notebooks/b.md", &"b".repeat(600))?;
            c.execute(
                "UPDATE note_prev SET saved_at = '2999-01-02T00:00:00+00:00' WHERE path = 'Notebooks/b.md'",
                [],
            )?;
            // The expired row goes by age, then the oldest of the rest by size.
            assert_eq!(prune_prev(c, PREV_RETENTION_DAYS, 1000)?, 2);
            assert!(get(c, "Notebooks/old.md")?.is_none());
            assert!(get(c, "Notebooks/a.md")?.is_none());
            assert!(get(c, "Notebooks/b.md")?.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_live_note_without_records_keeps_its_opened_mark() {
        db().with_conn(|c| {
            c.execute(
                "INSERT INTO note_seen (path, content_hash, seen_at) VALUES ('Notebooks/x.md', 'h', 't')",
                [],
            )?;
            purge_records(c, "Notebooks/x.md", true)?;
            let rows: i64 = c.query_row("SELECT COUNT(*) FROM note_seen", [], |r| r.get(0))?;
            assert_eq!(rows, 1);
            purge_records(c, "Notebooks/x.md", false)?;
            let rows: i64 = c.query_row("SELECT COUNT(*) FROM note_seen", [], |r| r.get(0))?;
            assert_eq!(rows, 0);
            Ok(())
        })
        .unwrap();
    }
}
