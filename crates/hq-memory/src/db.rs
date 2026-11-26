//! SQLite database operations for the memory system.
//! Ported from vault-memory/src/db.ts.

use anyhow::Result;
use hq_db::Database;

use crate::types::{Connection as MemConnection, Consolidation, Memory, MemoryStats};

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
            "SELECT * FROM memories WHERE consolidated = 0 ORDER BY created_at DESC LIMIT ?1",
            &[&limit],
        )
    })
}

pub fn get_recent_memories(db: &Database, limit: i64) -> Result<Vec<Memory>> {
    db.with_conn(|conn| {
        query_memories(
            conn,
            "SELECT * FROM memories ORDER BY importance DESC, created_at DESC LIMIT ?1",
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

/// Store a new memory. Returns the inserted row ID.
pub fn store_memory(db: &Database, params: &StoreMemoryParams<'_>) -> Result<i64> {
    let now = chrono::Utc::now().to_rfc3339();
    let entities_json = serde_json::to_string(params.entities)?;
    let topics_json = serde_json::to_string(params.topics)?;

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO memories (source, harness, raw_text, summary, entities, topics, importance, content, created_at, user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?4, ?8, ?9)",
            rusqlite::params![params.source, params.harness, params.raw_text, params.summary, entities_json, topics_json, params.importance, now, params.user_id],
        )?;
        Ok(conn.last_insert_rowid())
    })
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

// ─── Decay & Pruning (Synaptic Homeostasis) ─────────────────────────

/// Decay importance of unconsolidated memories older than `min_age_days`.
/// Returns the number of memories updated.
pub fn decay_old_memories(db: &Database, decay_rate: f64, min_age_days: i64) -> Result<i64> {
    let cutoff = cutoff_iso(min_age_days);
    db.with_conn(|conn| {
        let changes = conn.execute(
            "UPDATE memories SET importance = MAX(0.01, importance - ?1) WHERE consolidated = 0 AND created_at < ?2",
            rusqlite::params![decay_rate, cutoff],
        )?;
        Ok(changes as i64)
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

fn cutoff_iso(days: i64) -> String {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
    cutoff.to_rfc3339()
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

    fn importance_of(db: &Database, id: i64) -> f64 {
        db.with_conn(|c| {
            Ok(
                c.query_row("SELECT importance FROM memories WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap()
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
    fn decay_lowers_only_old_unconsolidated_memories_and_never_below_the_floor() {
        let db = Database::open_memory().unwrap();
        let old = store(&db, "old", 0.5);
        let fresh = store(&db, "fresh", 0.5);
        let consolidated = store(&db, "consolidated", 0.5);
        let at_floor = store(&db, "at the floor", 0.01);
        for id in [old, consolidated, at_floor] {
            age(&db, id, 10);
        }
        age(&db, fresh, 1);
        store_consolidation(&db, &[consolidated], "x", &[]).unwrap();

        let changed = decay_old_memories(&db, 0.1, 7).unwrap();

        assert_eq!(changed, 2, "the old unconsolidated pair, not fresh or done");
        assert!((importance_of(&db, old) - 0.4).abs() < 1e-9);
        assert!((importance_of(&db, fresh) - 0.5).abs() < 1e-9);
        assert!((importance_of(&db, consolidated) - 0.5).abs() < 1e-9);
        assert!((importance_of(&db, at_floor) - 0.01).abs() < 1e-9);
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
