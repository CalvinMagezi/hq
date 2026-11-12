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
pub fn sanitize_fts_query(query: &str) -> String {
    join_terms(query, " ")
}

/// [`sanitize_fts_query`] with the terms joined by OR: any of them may match.
pub fn sanitize_fts_query_any(query: &str) -> String {
    join_terms(query, " OR ")
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

/// Remove everything derived from a note's text: its FTS row, embedding, sync state and cached
/// copy. Link rows stay, so a move can keep them; use `remove_path_everywhere` for a delete.
pub fn remove_note(conn: &Connection, path: &str) -> Result<()> {
    super::reconcile::delete_content_rows(conn, path)?;
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
    keyword_search_in(conn, query, limit, None)
}

/// [`keyword_search`] inside one folder: only notes whose path starts with `prefix`
/// (`Notebooks/Projects/`), filtered before the limit so a folder's matches are never cut by others.
pub fn keyword_search_in(
    conn: &Connection,
    query: &str,
    limit: usize,
    prefix: Option<&str>,
) -> Result<Vec<SearchResult>> {
    keyword_search_filtered(
        conn,
        query,
        limit,
        &SearchFilter {
            prefix,
            ..SearchFilter::default()
        },
    )
}

/// What a search may be narrowed to. Every part is applied in the query, before the limit.
#[derive(Debug, Clone, Copy, Default)]
pub struct SearchFilter<'a> {
    /// Only notes whose path starts with this (`Notebooks/Projects/`).
    pub prefix: Option<&'a str>,
    /// Only notes whose tag column contains this text. The column is space-joined, so this is a
    /// superset of the notes that carry the whole tag: the caller confirms with the note's own
    /// front matter.
    pub tag: Option<&'a str>,
    /// Only notes with a record written by HQ. A superset too: the caller confirms the version on
    /// disk with `note_marks::mark`.
    pub written_by_hq: bool,
}

/// [`keyword_search`] narrowed by a [`SearchFilter`].
pub fn keyword_search_filtered(
    conn: &Connection,
    query: &str,
    limit: usize,
    filter: &SearchFilter<'_>,
) -> Result<Vec<SearchResult>> {
    let prefix = filter;
    let all_terms = join_terms(query, " ");
    if all_terms.is_empty() {
        return Ok(Vec::new());
    }
    let hits = match_query(conn, &all_terms, limit, prefix)?;
    // A sentence rarely has every word in one note, so fall back to any-term ranking.
    let any_terms = join_terms(query, " OR ");
    if hits.is_empty() && any_terms != all_terms {
        return match_query(conn, &any_terms, limit, prefix);
    }
    Ok(hits)
}

/// `prefix` as a LIKE pattern that matches only that literal start.
fn like_prefix(prefix: &str) -> String {
    let escaped = prefix
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("{escaped}%")
}

/// `text` as a LIKE pattern that matches it anywhere, taken literally.
fn like_contains(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

fn match_query(
    conn: &Connection,
    escaped: &str,
    limit: usize,
    filter: &SearchFilter<'_>,
) -> Result<Vec<SearchResult>> {
    let sql = format!(
        "SELECT path, title, snippet(notes_fts, 2, '<mark>', '</mark>', '...', 30) as snippet,
                tags, rank
         FROM notes_fts
         WHERE notes_fts MATCH ?1 AND (?3 IS NULL OR path LIKE ?3 ESCAPE '\\')
           AND (?4 IS NULL OR lower(tags) LIKE ?4 ESCAPE '\\')
           AND (?5 = 0 OR path IN ({}))
         ORDER BY rank
         LIMIT ?2",
        crate::provenance::HQ_WRITTEN_PATHS_SQL
    );
    let mut stmt = conn.prepare(&sql)?;
    let pattern = filter.prefix.map(like_prefix);
    let tag_pattern = filter.tag.map(|t| like_contains(&t.trim().to_lowercase()));

    let rows = stmt.query_map(
        rusqlite::params![
            escaped,
            limit,
            pattern,
            tag_pattern,
            i64::from(filter.written_by_hq)
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        },
    )?;

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
/// Walks the same way `sync_index` does: hidden folders, the trash and symlinks are skipped.
pub fn rebuild_index(conn: &Connection, notebooks_dir: &std::path::Path) -> Result<(usize, usize)> {
    // Clear existing FTS data
    conn.execute_batch("DELETE FROM notes_fts")?;

    if !notebooks_dir.exists() {
        return Ok((0, 0));
    }

    let mut indexed = 0usize;
    let mut errors = 0usize;

    for (path, rel_path) in super::reconcile::indexable_files(notebooks_dir) {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            errors += 1;
            continue;
        };
        let content = strip_frontmatter(&raw);
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let tags = extract_tags_from_frontmatter(&raw);
        if index_note(conn, &rel_path, &title, &content, &tags).is_ok() {
            indexed += 1;
        } else {
            errors += 1;
        }
    }

    Ok((indexed, errors))
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

#[cfg(test)]
mod folder_tests {
    use super::*;

    #[test]
    fn a_folder_search_keeps_only_that_folder_and_filters_before_the_limit() {
        let db = crate::Database::open_memory().unwrap();
        db.with_conn(|c| {
            for i in 0..5 {
                index_note(
                    c,
                    &format!("Notebooks/Other/n{i}.md"),
                    "battery",
                    "battery battery battery",
                    "",
                )?;
            }
            index_note(
                c,
                "Notebooks/Projects/Agent-HQ/plan.md",
                "plan",
                "the battery plan",
                "",
            )?;
            index_note(
                c,
                "Notebooks/Projects/Agent_HQx/odd.md",
                "odd",
                "battery",
                "",
            )?;
            let hits = keyword_search_in(c, "battery", 1, Some("Notebooks/Projects/Agent-HQ/"))?;
            let paths: Vec<_> = hits.iter().map(|h| h.note_path.as_str()).collect();
            assert_eq!(paths, ["Notebooks/Projects/Agent-HQ/plan.md"]);
            // `_` and `%` in a folder name are literal, not LIKE wildcards.
            let none = keyword_search_in(c, "battery", 10, Some("Notebooks/Projects/Agent_HQ/"))?;
            assert!(none.is_empty(), "{none:?}");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn tag_and_written_by_filters_apply_before_the_limit() {
        use crate::provenance::{Attribution, SourceKind, record};
        let db = crate::Database::open_memory().unwrap();
        db.with_conn(|c| {
            // Ten better-ranked notes that do not match the filters, one that does.
            for n in 0..10 {
                index_note(
                    c,
                    &format!("Notebooks/n{n}.md"),
                    "n",
                    "pelican pelican pelican",
                    "plain",
                )?;
            }
            index_note(c, "Notebooks/hq.md", "hq", "pelican", "work idea")?;
            let agent = Attribution {
                actor_id: Some("helper"),
                source_kind: Some(SourceKind::Agent),
                session_id: Some("s1"),
                task_id: None,
            };
            record(c, "Notebooks/hq.md", "h1", &agent)?;
            let hq = SearchFilter {
                written_by_hq: true,
                ..SearchFilter::default()
            };
            let hits = keyword_search_filtered(c, "pelican", 1, &hq)?;
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].note_path, "Notebooks/hq.md");
            let tagged = SearchFilter {
                tag: Some("Idea"),
                ..SearchFilter::default()
            };
            let hits = keyword_search_filtered(c, "pelican", 1, &tagged)?;
            assert_eq!(hits[0].note_path, "Notebooks/hq.md");
            let none = SearchFilter {
                tag: Some("zzz"),
                ..SearchFilter::default()
            };
            assert!(keyword_search_filtered(c, "pelican", 5, &none)?.is_empty());
            Ok(())
        })
        .unwrap();
    }
}
