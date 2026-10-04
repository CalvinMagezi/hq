//! Similarity graph links between notes.

use anyhow::Result;
use rusqlite::Connection;

// ─── Graph Links ─────────────────────────────────────────────────────────────

/// Record a graph link between two notes (UPSERT).
pub fn add_graph_link(
    conn: &Connection,
    source: &str,
    target: &str,
    score: f64,
    link_type: &str,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT OR REPLACE INTO graph_links (source_path, target_path, score, link_type, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![source, target, score, link_type, now],
    )?;
    Ok(())
}

/// Notes related to `path` via a similarity-based graph link, in either
/// direction, ordered by score descending. Restricted to `suggested`/
/// `applied` link types: `wikilink` rows store an absolute filesystem path as
/// `source_path` and a bare link title (not a path) as `target_path`, so they
/// never match `path`'s vault-relative form and would silently contribute
/// nothing anyway.
pub fn get_related_paths(conn: &Connection, path: &str, limit: usize) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT CASE WHEN source_path = ?1 THEN target_path ELSE source_path END AS other
         FROM graph_links
         WHERE (source_path = ?1 OR target_path = ?1)
           AND link_type IN ('suggested', 'applied')
         ORDER BY score DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![path, limit as i64], |row| {
        row.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}
