//! SQLite database operations for the memory system.
//! Ported from vault-memory/src/db.ts.

use anyhow::Result;
use hq_db::Database;

use crate::types::{Connection as MemConnection, Consolidation, Memory, MemoryStats, Trace};

// ─── Row parsing ────────────────────────────────────────────────────

/// Parse a rusqlite Row into a Memory struct.
/// The row must have been selected with `SELECT * FROM memories`.
pub fn parse_memory_row(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let entities_json: String = row.get_or_default("entities", "[]".to_string());
    let topics_json: String = row.get_or_default("topics", "[]".to_string());

    Ok(Memory {
        id: row.get("id")?,
        source: row.get("source")?,
        harness: row.get_or_default("harness", String::new()),
        raw_text: row.get_or_default("raw_text", String::new()),
        summary: row.get_or_default("summary", String::new()),
        entities: serde_json::from_str(&entities_json).unwrap_or_default(),
        topics: serde_json::from_str(&topics_json).unwrap_or_default(),
        importance: row.get("importance")?,
        consolidated: row.get::<_, i64>("consolidated")? != 0,
        replay_count: row.get_or_default("replay_count", 0),
        created_at: row.get("created_at")?,
        last_accessed_at: row.get_or_default("last_accessed_at", None),
        access_count: row.get_or_default("access_count", 0),
        delta_summary: row.get_or_default("delta_summary", None),
    })
}

/// Helper trait to get a column value with a default if the column doesn't exist.
trait RowExt {
    fn get_or_default<T: rusqlite::types::FromSql>(&self, col: &str, default: T) -> T;
}

impl RowExt for rusqlite::Row<'_> {
    fn get_or_default<T: rusqlite::types::FromSql>(&self, col: &str, default: T) -> T {
        self.get(col).unwrap_or(default)
    }
}

// ─── Read helpers ───────────────────────────────────────────────────

pub fn get_unconsolidated_memories(db: &Database, limit: i64) -> Result<Vec<Memory>> {
    db.with_conn(|conn| {
        query_memories(
            conn,
            "SELECT * FROM memories WHERE consolidated = 0 AND status = 'active' AND origin = 'owner' ORDER BY created_at DESC LIMIT ?1",
            &[&limit],
        )
    })
}

pub fn get_recent_memories(db: &Database, limit: i64) -> Result<Vec<Memory>> {
    db.with_conn(|conn| {
        query_memories(
            conn,
            "SELECT * FROM memories WHERE status = 'active' AND origin = 'owner' ORDER BY importance DESC, created_at DESC LIMIT ?1",
            &[&limit],
        )
    })
}

pub fn get_consolidation_history(db: &Database, limit: i64) -> Result<Vec<Consolidation>> {
    db.with_conn(|conn| {
        let mut stmt =
            conn.prepare("SELECT * FROM consolidations ORDER BY created_at DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit], |row| {
            let memory_ids_json: String = row.get("memory_ids")?;
            let connections_json: String = row.get_or_default("connections", "[]".to_string());
            Ok(Consolidation {
                id: row.get("id")?,
                source_ids: serde_json::from_str(&memory_ids_json).unwrap_or_default(),
                insight: row.get("insight")?,
                connections: serde_json::from_str(&connections_json).unwrap_or_default(),
                parent_id: row.get_or_default("parent_id", None),
                created_at: row.get("created_at")?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    })
}

pub fn get_memory_stats(db: &Database) -> Result<MemoryStats> {
    db.with_conn(|conn| {
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))?;
        let unconsolidated: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE consolidated = 0",
            [],
            |r| r.get(0),
        )?;
        let consolidations: i64 =
            conn.query_row("SELECT COUNT(*) FROM consolidations", [], |r| r.get(0))?;
        Ok(MemoryStats {
            total,
            unconsolidated,
            consolidations,
        })
    })
}

// ─── Write helpers ──────────────────────────────────────────────────

/// Parameters for storing a new memory.
pub struct StoreMemoryParams<'a> {
    pub source: &'a str,
    pub harness: &'a str,
    pub raw_text: &'a str,
    pub summary: &'a str,
    pub entities: &'a [String],
    pub topics: &'a [String],
    pub importance: f64,
    pub user_id: Option<&'a str>,
}

/// Store a new memory as an ordinary chat summary from the owner. Returns the inserted row ID.
pub fn store_memory(db: &Database, params: &StoreMemoryParams<'_>) -> Result<i64> {
    store_memory_with(db, params, &Trace::default())
}

/// Store a new memory with a record of where it came from. Returns the inserted row ID.
pub fn store_memory_with(
    db: &Database,
    params: &StoreMemoryParams<'_>,
    trace: &Trace<'_>,
) -> Result<i64> {
    db.with_conn(|conn| insert_memory(conn, params, trace, &Extras::default()))
}

/// What a new memory carries beyond the summary: whether it is held back, and whether the owner
/// asked for it to be kept for good.
#[derive(Debug, Clone, Copy)]
pub struct Extras {
    /// `active`, or `review` when the owner should look before it is ever brought up.
    pub status: &'static str,
    pub pinned: bool,
}

impl Default for Extras {
    fn default() -> Self {
        Self {
            status: "active",
            pinned: false,
        }
    }
}

/// The outcome of [`store_or_reinforce`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stored {
    pub id: i64,
    /// The same words were already held, so that memory was reinforced and nothing was added.
    pub reinforced: bool,
}

/// Store a memory, unless these same words (case, punctuation and spacing aside) are already held
/// from the same kind of source: then that memory is seen once more instead of a copy being
/// added. The lookup and the write share one write lock, so two turns saying the same thing at
/// once cannot both insert.
pub fn store_or_reinforce(
    db: &Database,
    params: &StoreMemoryParams<'_>,
    trace: &Trace<'_>,
    extras: &Extras,
) -> Result<Stored> {
    let hash = crate::hash::content_hash(params.summary);
    db.with_conn(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<Stored> {
            let existing: Option<i64> = conn
                .query_row(
                    "SELECT id FROM memories
                     WHERE content_hash = ?1 AND origin = ?2 AND status IN ('active', 'review')
                     ORDER BY id LIMIT 1",
                    rusqlite::params![hash, trace.origin.as_str()],
                    |r| r.get(0),
                )
                .ok();
            if let Some(id) = existing {
                conn.execute(
                    "UPDATE memories
                     SET seen_count = seen_count + 1,
                         last_seen_at = ?2,
                         importance = MAX(importance, ?3),
                         pinned = MAX(pinned, ?4),
                         provenance = CASE WHEN ?5 = 'owner_said' THEN 'owner_said' ELSE provenance END
                     WHERE id = ?1",
                    rusqlite::params![
                        id,
                        chrono::Utc::now().to_rfc3339(),
                        params.importance,
                        extras.pinned,
                        trace.provenance.as_str()
                    ],
                )?;
                return Ok(Stored {
                    id,
                    reinforced: true,
                });
            }
            Ok(Stored {
                id: insert_memory(conn, params, trace, extras)?,
                reinforced: false,
            })
        })();
        match result {
            Ok(stored) => {
                conn.execute_batch("COMMIT")?;
                Ok(stored)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    })
}

fn insert_memory(
    conn: &rusqlite::Connection,
    params: &StoreMemoryParams<'_>,
    trace: &Trace<'_>,
    extras: &Extras,
) -> Result<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let entities_json = serde_json::to_string(params.entities)?;
    let topics_json = serde_json::to_string(params.topics)?;
    conn.execute(
        "INSERT INTO memories (source, harness, raw_text, summary, entities, topics, importance, content, created_at, user_id, origin, provenance, thread_id, content_hash, status, pinned, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?4, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?8)",
        rusqlite::params![
            params.source,
            params.harness,
            params.raw_text,
            params.summary,
            entities_json,
            topics_json,
            params.importance,
            now,
            params.user_id,
            trace.origin.as_str(),
            trace.provenance.as_str(),
            trace.thread_id,
            crate::hash::content_hash(params.summary),
            extras.status,
            extras.pinned
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Store a consolidation and mark source memories as consolidated.
pub fn store_consolidation(
    db: &Database,
    source_ids: &[i64],
    insight: &str,
    connections: &[MemConnection],
) -> Result<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let ids_json = serde_json::to_string(source_ids)?;
    let conns_json = serde_json::to_string(connections)?;

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO consolidations (memory_ids, insight, connections, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ids_json, insight, conns_json, now],
        )?;
        let consolidation_id = conn.last_insert_rowid();

        // Mark source memories as consolidated
        if !source_ids.is_empty() {
            let placeholders: String = source_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!("UPDATE memories SET consolidated = 1 WHERE id IN ({placeholders})");
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<Box<dyn rusqlite::types::ToSql>> = source_ids
                .iter()
                .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
                .collect();
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            stmt.execute(param_refs.as_slice())?;
        }

        Ok(consolidation_id)
    })
}

/// Store a chained consolidation (insight that extends/contradicts a previous insight).
pub fn store_consolidation_chained(
    db: &Database,
    source_ids: &[i64],
    insight: &str,
    connections: &[MemConnection],
    parent_id: i64,
) -> Result<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let ids_json = serde_json::to_string(source_ids)?;
    let conns_json = serde_json::to_string(connections)?;

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO consolidations (memory_ids, insight, connections, parent_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![ids_json, insight, conns_json, parent_id, now],
        )?;
        Ok(conn.last_insert_rowid())
    })
}

/// Bump access count and last_accessed_at for a memory (served to an agent).
pub fn touch_memory(db: &Database, id: i64) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE memories SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
            rusqlite::params![now, id],
        )?;
        Ok(())
    })
}

// ─── Internal helpers ───────────────────────────────────────────────

fn query_memories(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[&dyn rusqlite::types::ToSql],
) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, parse_memory_row)?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(db: &Database, summary: &str, importance: f64) -> i64 {
        store_memory(
            db,
            &StoreMemoryParams {
                source: "chat-turn",
                harness: "web",
                raw_text: "",
                summary,
                entities: &[],
                topics: &[],
                importance,
                user_id: None,
            },
        )
        .unwrap()
    }

    /// Backdate a row so age-based rules can be exercised without waiting.
    fn age(db: &Database, id: i64, days: i64) {
        let at = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        db.with_conn(|c| {
            c.execute(
                "UPDATE memories SET created_at = ?1 WHERE id = ?2",
                rusqlite::params![at, id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn mark(db: &Database, id: i64, status: &str, origin: &str) {
        db.with_conn(|c| {
            c.execute(
                "UPDATE memories SET status = ?1, origin = ?2 WHERE id = ?3",
                rusqlite::params![status, origin, id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    /// The always-on list and the consolidator read memories nobody asked about, so they must
    /// only ever see what the owner's own chats made and nobody has held back or retired.
    #[test]
    fn only_active_notes_from_the_owner_reach_the_always_on_list_and_the_consolidator() {
        let db = Database::open_memory().unwrap();
        let fine = store(&db, "an ordinary note", 0.5);
        let held = store(&db, "held back for review", 0.9);
        let shared = store(&db, "from a shared channel", 0.9);
        let outdated = store(&db, "no longer true", 0.9);
        let away = store(&db, "put away after a year", 0.9);
        mark(&db, held, "review", "owner");
        mark(&db, shared, "active", "unknown_channel");
        mark(&db, outdated, "superseded", "owner");
        mark(&db, away, "archived", "owner");

        let recent: Vec<i64> = get_recent_memories(&db, 10)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        let pending: Vec<i64> = get_unconsolidated_memories(&db, 10)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();

        assert_eq!(recent, vec![fine]);
        assert_eq!(pending, vec![fine]);
    }

    #[test]
    fn recent_memories_come_back_most_important_first_then_newest() {
        let db = Database::open_memory().unwrap();
        let older = store(&db, "older", 0.5);
        let newer = store(&db, "newer", 0.5);
        let top = store(&db, "top", 0.9);
        age(&db, older, 5);
        age(&db, newer, 1);
        age(&db, top, 30);

        let ids: Vec<i64> = get_recent_memories(&db, 10)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        // Importance wins over age, and equal importance falls back to the newest.
        assert_eq!(ids, vec![top, newer, older]);
    }

    #[test]
    fn recent_memories_respect_the_limit() {
        let db = Database::open_memory().unwrap();
        for i in 0..5 {
            store(&db, &format!("m{i}"), 0.5);
        }
        assert_eq!(get_recent_memories(&db, 3).unwrap().len(), 3);
    }

    #[test]
    fn unconsolidated_memories_exclude_consolidated_ones_and_list_newest_first() {
        let db = Database::open_memory().unwrap();
        let old = store(&db, "old", 0.5);
        let new = store(&db, "new", 0.5);
        let done = store(&db, "done", 0.5);
        age(&db, old, 4);
        age(&db, new, 1);
        store_consolidation(&db, &[done], "an insight", &[]).unwrap();

        let ids: Vec<i64> = get_unconsolidated_memories(&db, 10)
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec![new, old]);
    }

    #[test]
    fn a_consolidation_marks_its_sources_and_shows_in_the_stats() {
        let db = Database::open_memory().unwrap();
        let a = store(&db, "a", 0.5);
        let b = store(&db, "b", 0.5);
        store(&db, "c", 0.5);

        store_consolidation(&db, &[a, b], "a and b agree", &[]).unwrap();

        let stats = get_memory_stats(&db).unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.unconsolidated, 1);
        assert_eq!(stats.consolidations, 1);
        let history = get_consolidation_history(&db, 5).unwrap();
        assert_eq!(history[0].source_ids, vec![a, b]);
        assert_eq!(history[0].insight, "a and b agree");
    }

    #[test]
    fn touching_a_memory_counts_the_access_and_stamps_the_time() {
        let db = Database::open_memory().unwrap();
        let id = store(&db, "used", 0.5);
        assert_eq!(get_recent_memories(&db, 1).unwrap()[0].access_count, 0);

        touch_memory(&db, id).unwrap();
        touch_memory(&db, id).unwrap();

        let m = &get_recent_memories(&db, 1).unwrap()[0];
        assert_eq!(m.access_count, 2);
        assert!(m.last_accessed_at.is_some());
    }

    #[test]
    fn the_summary_is_also_kept_in_the_legacy_content_column() {
        let db = Database::open_memory().unwrap();
        let id = store(&db, "the summary", 0.5);
        let content: String = db
            .with_conn(|c| {
                Ok(
                    c.query_row("SELECT content FROM memories WHERE id = ?1", [id], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(content, "the summary");
    }
}
