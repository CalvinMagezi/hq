//! FTS5 index: note indexing, full and incremental sync, keyword and tag queries.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use hq_core::types::{MatchType, SearchResult};

use super::{notebook_from_path, title_from_path};

// ─── FTS5 Query Sanitization ─────────────────────────────────────────────────

/// Turn free text into a safe FTS5 query: each alphanumeric run becomes a quoted
/// phrase (so `AND`/`OR`/`NOT`/`NEAR` are plain words), joined by implicit AND.
/// Text with no searchable characters yields an empty string.
pub(super) fn sanitize_fts_query(query: &str) -> String {
    join_terms(query, " ")
}

fn join_terms(query: &str, joiner: &str) -> String {
    query
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(joiner)
}

// ─── Index Operations ────────────────────────────────────────────────────────

/// Index a note into FTS5 for full-text search.
/// Performs DELETE + INSERT (FTS5 doesn't support UPSERT).
pub fn index_note(
    conn: &Connection,
    path: &str,
    title: &str,
    content: &str,
    tags: &str,
) -> Result<()> {
    conn.execute("DELETE FROM notes_fts WHERE path = ?1", [path])?;
    conn.execute(
        "INSERT INTO notes_fts (path, title, content, tags) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![path, title, content, tags],
    )?;
    Ok(())
}

/// Remove a note from both FTS index and embeddings table.
pub fn remove_note(conn: &Connection, path: &str) -> Result<()> {
    conn.execute("DELETE FROM notes_fts WHERE path = ?1", [path])?;
    conn.execute("DELETE FROM embeddings WHERE note_path = ?1", [path])?;
    Ok(())
}

/// Get total number of FTS-indexed notes.
pub fn indexed_count(conn: &Connection) -> Result<usize> {
    let count: usize = conn.query_row("SELECT COUNT(*) FROM notes_fts", [], |row| row.get(0))?;
    Ok(count)
}

// ─── Keyword Search ──────────────────────────────────────────────────────────

/// Full-text keyword search using FTS5 MATCH with snippets.
/// Returns results sorted by FTS5 rank (most relevant first).
pub fn keyword_search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let all_terms = join_terms(query, " ");
    if all_terms.is_empty() {
        return Ok(Vec::new());
    }
    let hits = match_query(conn, &all_terms, limit)?;
    // A sentence rarely has every word in one note, so fall back to any-term ranking.
    let any_terms = join_terms(query, " OR ");
    if hits.is_empty() && any_terms != all_terms {
        return match_query(conn, &any_terms, limit);
    }
    Ok(hits)
}

fn match_query(conn: &Connection, escaped: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let mut stmt = conn.prepare(
        "SELECT path, title, snippet(notes_fts, 2, '<mark>', '</mark>', '...', 30) as snippet,
                tags, rank
         FROM notes_fts
         WHERE notes_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(rusqlite::params![escaped, limit], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, f64>(4)?,
        ))
    })?;

    let mut results = Vec::new();
    for row in rows {
        let (path, title, snippet, tags_str, rank) = row?;
        let notebook = notebook_from_path(&path);
        let snippet_clean = snippet.replace("<mark>", "").replace("</mark>", "");
        let tags: Vec<String> = tags_str
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        results.push(SearchResult {
            note_path: path,
            title,
            notebook,
            snippet: snippet_clean,
            tags,
            relevance: -rank, // FTS5 rank is negative; negate so higher = better
            match_type: MatchType::Keyword,
        });
    }

    Ok(results)
}

// ─── Recent Notes ────────────────────────────────────────────────────────────

/// The most recently-touched notes, newest first — bypasses FTS matching
/// entirely. A query with no useful keyword returns nothing from
/// `keyword_search`/`hybrid_search` no matter how the (empty) result set is
/// sorted afterward, so "what have we been working on" needs a mode that
/// reads recency directly from `vault_cache` instead.
pub fn recent_notes(conn: &Connection, limit: usize) -> Result<Vec<SearchResult>> {
    let mut stmt = conn.prepare(
        "SELECT vc.path, vc.title, vc.content_preview, COALESCE(f.tags, '')
         FROM (SELECT path, title, content_preview FROM vault_cache ORDER BY mtime DESC LIMIT ?1) vc
         LEFT JOIN notes_fts f ON f.path = vc.path",
    )?;
    let rows = stmt.query_map([limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    let mut results = Vec::new();
    for row in rows {
        let (path, title, preview, tags_str) = row?;
        let tags: Vec<String> = tags_str
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        results.push(SearchResult {
            title: title.unwrap_or_else(|| title_from_path(&path)),
            notebook: notebook_from_path(&path),
            snippet: preview.unwrap_or_default(),
            tags,
            relevance: 0.0,
            match_type: MatchType::Recent,
            note_path: path,
        });
    }
    Ok(results)
}

// ─── Rebuild Index ───────────────────────────────────────────────────────────

/// Rebuild the full-text index by scanning all `.md` files under `notebooks_dir`.
/// Returns (indexed_count, error_count).
///
/// The caller is responsible for parsing frontmatter and calling `index_note`
/// for each file; this function provides the batch scaffolding.
pub fn rebuild_index(conn: &Connection, notebooks_dir: &std::path::Path) -> Result<(usize, usize)> {
    use std::fs;

    // Clear existing FTS data
    conn.execute_batch("DELETE FROM notes_fts")?;

    if !notebooks_dir.exists() {
        return Ok((0, 0));
    }

    let mut indexed = 0usize;
    let mut errors = 0usize;

    fn scan_dir(
        conn: &Connection,
        dir: &std::path::Path,
        vault_path: &std::path::Path,
        indexed: &mut usize,
        errors: &mut usize,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_dir(conn, &path, vault_path, indexed, errors);
            } else if let Some(ext) = path.extension()
                && ext == "md"
            {
                if let Some(name) = path.file_name()
                    && name == "_meta.md"
                {
                    continue;
                }
                match fs::read_to_string(&path) {
                    Ok(raw) => {
                        // Simple frontmatter extraction: skip YAML block between --- delimiters
                        let content = strip_frontmatter(&raw);
                        let title = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_string();

                        // Extract tags from frontmatter (simple parsing)
                        let tags = extract_tags_from_frontmatter(&raw);

                        let rel_path = path
                            .strip_prefix(vault_path)
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| path.to_string_lossy().to_string());

                        if index_note(conn, &rel_path, &title, &content, &tags).is_ok() {
                            *indexed += 1;
                        } else {
                            *errors += 1;
                        }
                    }
                    Err(_) => {
                        *errors += 1;
                    }
                }
            }
        }
    }

    // vault_path is the parent of Notebooks
    let vault_path = notebooks_dir.parent().unwrap_or(notebooks_dir);

    scan_dir(conn, notebooks_dir, vault_path, &mut indexed, &mut errors);

    Ok((indexed, errors))
}

/// Incrementally update the FTS index, skipping unchanged files.
///
/// Compares each `.md` file's content hash against `sync_state`. Files with a
/// matching hash are skipped entirely. Changed or new files are re-indexed and
/// their hash stored. FTS entries for deleted files are removed.
///
/// Returns `(indexed, removed, unchanged)` counts.
pub fn sync_index_incremental(
    conn: &Connection,
    notebooks_dir: &std::path::Path,
) -> Result<(usize, usize, usize)> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Load known hashes from sync_state
    let mut known_hashes: HashMap<String, String> = {
        let mut stmt = conn.prepare("SELECT path, content_hash FROM sync_state")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut m = HashMap::new();
        for row in rows {
            let (p, h) = row?;
            m.insert(p, h);
        }
        m
    };

    if !notebooks_dir.exists() {
        return Ok((0, 0, 0));
    }

    let vault_path = notebooks_dir.parent().unwrap_or(notebooks_dir);
    let mut indexed = 0usize;
    let mut unchanged = 0usize;
    let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    fn hash_content(s: &str) -> String {
        let mut h = DefaultHasher::new();
        s.hash(&mut h);
        format!("{:x}", h.finish())
    }

    fn walk(
        conn: &Connection,
        dir: &std::path::Path,
        vault_path: &std::path::Path,
        known: &mut HashMap<String, String>,
        seen: &mut std::collections::HashSet<String>,
        indexed: &mut usize,
        unchanged: &mut usize,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(conn, &path, vault_path, known, seen, indexed, unchanged);
                continue;
            }
            let Some(ext) = path.extension() else {
                continue;
            };
            if ext != "md" {
                continue;
            }
            if path.file_name().map(|n| n == "_meta.md").unwrap_or(false) {
                continue;
            }

            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let hash = hash_content(&raw);

            let rel_path = path
                .strip_prefix(vault_path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| path.to_string_lossy().to_string());

            seen.insert(rel_path.clone());

            if known.get(&rel_path).map(|h| h == &hash).unwrap_or(false) {
                *unchanged += 1;
                continue;
            }

            // Index the changed/new file
            let content = strip_frontmatter(&raw);
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let tags = extract_tags_from_frontmatter(&raw);

            if index_note(conn, &rel_path, &title, &content, &tags).is_ok() {
                let now = chrono::Utc::now().timestamp();
                let _ = conn.execute(
                    "INSERT INTO sync_state (path, content_hash, modified_at, synced_at)
                     VALUES (?1, ?2, ?3, ?3)
                     ON CONFLICT(path) DO UPDATE SET content_hash = excluded.content_hash, synced_at = excluded.synced_at",
                    rusqlite::params![rel_path, hash, now],
                );
                // Populate vault_cache with token count so context engine can skip counting.
                let token_count = ((raw.len() * 2).div_ceil(7)) as i64;
                if let Err(e) = crate::vault_cache::write_note_cached(
                    conn,
                    &rel_path,
                    &raw,
                    &title,
                    None,
                    Some(token_count),
                ) {
                    tracing::debug!(path = %rel_path, error = %e, "vault_cache write failed during FTS sync");
                }
                known.insert(rel_path, hash);
                *indexed += 1;
            }
        }
    }

    walk(
        conn,
        notebooks_dir,
        vault_path,
        &mut known_hashes,
        &mut seen_paths,
        &mut indexed,
        &mut unchanged,
    );

    // Remove FTS + sync_state entries for deleted files
    let stale: Vec<String> = known_hashes
        .keys()
        .filter(|p| !seen_paths.contains(*p))
        .cloned()
        .collect();
    let removed = stale.len();
    for path in &stale {
        let _ = conn.execute("DELETE FROM notes_fts WHERE path = ?1", [path.as_str()]);
        let _ = conn.execute("DELETE FROM sync_state WHERE path = ?1", [path.as_str()]);
    }

    Ok((indexed, removed, unchanged))
}

/// Strip YAML frontmatter from markdown content. Delegates to `hq_core::frontmatter_utils`.
pub(super) fn strip_frontmatter(raw: &str) -> String {
    hq_core::frontmatter_utils::strip_frontmatter(raw).to_string()
}

/// Extract space-separated tags from YAML frontmatter. Delegates to `hq_core::frontmatter_utils`.
pub(super) fn extract_tags_from_frontmatter(raw: &str) -> String {
    hq_core::frontmatter_utils::extract_tags_from_frontmatter(raw).join(" ")
}

// ─── Tag Queries ─────────────────────────────────────────────────────────────

/// Get tag counts across all indexed notes.
pub fn get_all_tags(conn: &Connection) -> Result<HashMap<String, usize>> {
    let mut stmt = conn.prepare("SELECT tags FROM notes_fts WHERE tags != ''")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

    let mut counts: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let tags_str = row?;
        for tag in tags_str.split_whitespace() {
            if !tag.is_empty() {
                *counts.entry(tag.to_string()).or_insert(0) += 1;
            }
        }
    }
    Ok(counts)
}

/// Get note paths for a specific tag from the FTS index.
pub fn get_tagged_note_paths(conn: &Connection, tag: &str) -> Result<Vec<String>> {
    let escaped = sanitize_fts_query(tag);
    if escaped.is_empty() {
        return Ok(Vec::new());
    }

    // Use FTS5 column filter syntax so MATCH only searches the `tags` column,
    // not all columns (which would cause false positives on title/body matches).
    let fts_query = format!("tags:({escaped})");
    let mut stmt = conn.prepare("SELECT path FROM notes_fts WHERE notes_fts MATCH ?1 LIMIT 100")?;
    let rows = stmt.query_map([&fts_query], |row| row.get::<_, String>(0))?;

    let mut paths = Vec::new();
    for row in rows {
        paths.push(row?);
    }
    Ok(paths)
}
