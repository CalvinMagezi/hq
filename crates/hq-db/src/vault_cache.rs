//! Read-through / write-through cache for vault notes in SQLite.
//!
//! Functions here are intentionally standalone (take `&Connection`) so callers
//! that already have a DB handle can use them without coupling vault I/O to hq-db.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

/// Write a note into vault_cache after a filesystem write succeeds.
///
/// Stores the full content, mtime (seconds since Unix epoch), a fast hash,
/// title, JSON frontmatter, and pre-computed token count.
pub fn write_note_cached(
    conn: &Connection,
    rel_path: &str,
    content: &str,
    title: &str,
    metadata_json: Option<&str>,
    token_count: Option<i64>,
) -> Result<()> {
    // Truncate the preview on a UTF-8 char boundary — slicing by raw byte index
    // panics when byte 512 lands inside a multi-byte character.
    fn preview_512(s: &str) -> &str {
        &s[..s.floor_char_boundary(512)]
    }
    let mtime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let hash = fast_hash(content);

    conn.execute(
        "INSERT INTO vault_cache (path, mtime, hash, title, metadata, content_preview, content, token_count, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
         ON CONFLICT(path) DO UPDATE SET
           mtime = excluded.mtime,
           hash = excluded.hash,
           title = excluded.title,
           metadata = excluded.metadata,
           content_preview = excluded.content_preview,
           content = excluded.content,
           token_count = excluded.token_count,
           updated_at = datetime('now')",
        rusqlite::params![
            rel_path,
            mtime,
            hash,
            title,
            metadata_json,
            preview_512(content),
            content,
            token_count,
        ],
    )?;
    Ok(())
}

/// Last cache-write time (epoch seconds) for a note, or `None` if it was
/// never written through the cache.
pub fn get_mtime(conn: &Connection, path: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT mtime FROM vault_cache WHERE path = ?1",
        [path],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn fast_hash(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../sql/010_vault_cache.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/018_vault_cache_token_count.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/019_vault_cache_content.sql"))
            .unwrap();
        conn
    }

    #[test]
    fn get_mtime_none_for_unknown_path() {
        let conn = setup_test_db();
        assert_eq!(get_mtime(&conn, "Notebooks/missing.md").unwrap(), None);
    }

    #[test]
    fn get_mtime_some_after_write() {
        let conn = setup_test_db();
        write_note_cached(&conn, "Notebooks/foo.md", "content", "Foo", None, None).unwrap();
        assert!(get_mtime(&conn, "Notebooks/foo.md").unwrap().is_some());
    }
}
