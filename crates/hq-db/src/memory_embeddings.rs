//! On-device vectors for memories: which memories still need one, storing one, and finding the
//! memories closest in meaning to a question.
//!
//! Only active memories from the owner are ever offered, the same rule as keyword recall, so a
//! held-back or shared-channel memory cannot come back through its meaning either.

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::search::{bytes_to_embedding, cosine_similarity, embedding_to_bytes};

/// Most vectors read for one question. A phone's active memories are capped well under this.
const SCAN_LIMIT: i64 = 20_000;

/// A memory still waiting for a vector made by `model`: its number and the words to embed.
pub fn missing(conn: &Connection, model: &str, limit: i64) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, COALESCE(NULLIF(m.summary, ''), m.content, '')
           FROM memories m
           LEFT JOIN memory_embeddings e ON e.memory_id = m.id AND e.model = ?1
          WHERE m.status = 'active' AND m.origin = 'owner' AND e.memory_id IS NULL
          ORDER BY m.id DESC
          LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![model, limit], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, s): &(i64, String)| !s.trim().is_empty())
        .collect())
}

/// Store (or replace) the vector for a memory.
pub fn store(conn: &Connection, memory_id: i64, model: &str, vector: &[f32]) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_embeddings (memory_id, model, vector) VALUES (?1, ?2, ?3)
         ON CONFLICT(memory_id) DO UPDATE SET model = excluded.model, vector = excluded.vector",
        params![memory_id, model, embedding_to_bytes(vector)],
    )?;
    Ok(())
}

/// How far the vectors have got: how many eligible memories have one from `model`, out of how
/// many are eligible.
pub fn progress(conn: &Connection, model: &str) -> Result<(i64, i64)> {
    let eligible: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active' AND origin = 'owner'",
        [],
        |r| r.get(0),
    )?;
    let done: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_embeddings e JOIN memories m ON m.id = e.memory_id
          WHERE e.model = ?1 AND m.status = 'active' AND m.origin = 'owner'",
        [model],
        |r| r.get(0),
    )?;
    Ok((done, eligible))
}

/// The active owner memories closest in meaning to `query`, best first, each with its cosine
/// similarity. Only those at or above `min_similarity` are returned.
pub fn nearest(
    conn: &Connection,
    query: &[f32],
    model: &str,
    min_similarity: f32,
    limit: usize,
) -> Result<Vec<(i64, f32)>> {
    let mut stmt = conn.prepare(
        "SELECT e.memory_id, e.vector
           FROM memory_embeddings e JOIN memories m ON m.id = e.memory_id
          WHERE e.model = ?1 AND m.status = 'active' AND m.origin = 'owner'
          LIMIT ?2",
    )?;
    let mut hits: Vec<(i64, f32)> = stmt
        .query_map(params![model, SCAN_LIMIT], |r| {
            let bytes: Vec<u8> = r.get(1)?;
            Ok((
                r.get::<_, i64>(0)?,
                cosine_similarity(query, &bytes_to_embedding(&bytes)),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    hits.retain(|(_, s)| *s >= min_similarity);
    hits.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
    hits.truncate(limit);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn db_with(rows: &[(&str, &str, &str)]) -> Database {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for (summary, status, origin) in rows {
                c.execute(
                    "INSERT INTO memories (source, harness, raw_text, summary, content, created_at, importance, status, origin)
                     VALUES ('t', 't', '', ?1, ?1, '2026-01-01T00:00:00Z', 0.5, ?2, ?3)",
                    params![summary, status, origin],
                )?;
            }
            Ok(())
        })
        .unwrap();
        db
    }

    #[test]
    fn only_active_owner_memories_wait_for_a_vector() {
        let db = db_with(&[
            ("one", "active", "owner"),
            ("two", "review", "owner"),
            ("three", "active", "unknown_channel"),
            ("four", "archived", "owner"),
        ]);
        let todo = db.with_conn(|c| missing(c, "m", 10)).unwrap();
        assert_eq!(
            todo.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
            ["one"]
        );
    }

    #[test]
    fn a_stored_vector_is_found_by_meaning_and_a_changed_model_needs_a_new_one() {
        let db = db_with(&[("near", "active", "owner"), ("far", "active", "owner")]);
        db.with_conn(|c| {
            store(c, 1, "m1", &[1.0, 0.0])?;
            store(c, 2, "m1", &[0.0, 1.0])?;
            Ok(())
        })
        .unwrap();
        let hits = db
            .with_conn(|c| nearest(c, &[0.9, 0.1], "m1", 0.5, 5))
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 1);
        assert_eq!(db.with_conn(|c| progress(c, "m1")).unwrap(), (2, 2));
        // A new model label means both are missing again.
        assert_eq!(db.with_conn(|c| missing(c, "m2", 10)).unwrap().len(), 2);
        assert_eq!(db.with_conn(|c| progress(c, "m2")).unwrap(), (0, 2));
    }

    #[test]
    fn a_held_back_or_shared_memory_is_not_found_by_meaning() {
        let db = db_with(&[
            ("held", "review", "owner"),
            ("shared", "active", "unknown_channel"),
        ]);
        db.with_conn(|c| {
            store(c, 1, "m", &[1.0, 0.0])?;
            store(c, 2, "m", &[1.0, 0.0])?;
            Ok(())
        })
        .unwrap();
        assert!(
            db.with_conn(|c| nearest(c, &[1.0, 0.0], "m", 0.1, 5))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn forgetting_a_memory_takes_its_vector_with_it() {
        let db = db_with(&[("gone soon", "active", "owner")]);
        db.with_conn(|c| store(c, 1, "m", &[1.0, 0.0])).unwrap();
        db.with_conn(|c| Ok(c.execute("DELETE FROM memories WHERE id = 1", [])?))
            .unwrap();
        let left: i64 = db
            .with_conn(|c| {
                Ok(c.query_row("SELECT COUNT(*) FROM memory_embeddings", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(left, 0);
    }
}
