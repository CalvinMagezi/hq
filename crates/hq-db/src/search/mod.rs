//! Full search client — FTS5 keyword search, vector embeddings, cosine similarity,
//! hybrid search, and graph links.
//!
//! Port of the TypeScript `SearchClient` from `packages/vault-client/src/search.ts`.

mod fts;
mod graph;
mod semantic;
#[cfg(test)]
mod tests;

pub use fts::*;
pub use graph::*;
pub use semantic::*;

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Connection;

use hq_core::types::{MatchType, SearchResult, SearchStats};

/// Extract notebook name from a relative note path (e.g. "Notebooks/Projects/foo.md" -> "Projects").
fn notebook_from_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 1 {
        parts[1].to_string()
    } else {
        "Unknown".to_string()
    }
}

/// Extract a title from a file path (basename without .md extension).
fn title_from_path(path: &str) -> String {
    let basename = path.rsplit('/').next().unwrap_or(path);
    basename.strip_suffix(".md").unwrap_or(basename).to_string()
}

// ─── Hybrid Search ───────────────────────────────────────────────────────────

/// Hybrid search: run both keyword and semantic search, normalize scores to [0,1],
/// merge with keyword * 0.4 + semantic * 0.6 weighting.
pub fn hybrid_search(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let keyword_results = keyword_search(conn, query, limit * 2)?;

    let query_embedding = match query_embedding {
        Some(emb) => emb,
        None => {
            // No embedding available — return keyword-only results
            let mut results = keyword_results;
            results.truncate(limit);
            return Ok(results);
        }
    };

    let semantic_results = semantic_search(conn, query_embedding, limit * 2)?;

    // Find max scores for normalization (floor at 1.0 to avoid division by zero)
    let max_keyword = keyword_results
        .iter()
        .map(|r| r.relevance)
        .fold(1.0f64, f64::max);
    let max_semantic = semantic_results
        .iter()
        .map(|r| r.relevance)
        .fold(1.0f64, f64::max);

    // Merge into a map keyed by note_path
    let mut merged: HashMap<String, SearchResult> = HashMap::new();

    for r in keyword_results {
        let normalized = r.relevance / max_keyword;
        merged.insert(
            r.note_path.clone(),
            SearchResult {
                relevance: normalized * 0.4,
                match_type: MatchType::Hybrid,
                ..r
            },
        );
    }

    for r in semantic_results {
        let normalized = r.relevance / max_semantic;
        if let Some(existing) = merged.get_mut(&r.note_path) {
            existing.relevance += normalized * 0.6;
        } else {
            merged.insert(
                r.note_path.clone(),
                SearchResult {
                    relevance: normalized * 0.6,
                    match_type: MatchType::Hybrid,
                    ..r
                },
            );
        }
    }

    let mut results: Vec<SearchResult> = merged.into_values().collect();
    results.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    Ok(results)
}

// ─── Stats ───────────────────────────────────────────────────────────────────

/// Get search index statistics: FTS count + embedding count.
pub fn get_stats(conn: &Connection) -> Result<SearchStats> {
    let fts_count: usize =
        conn.query_row("SELECT COUNT(*) FROM notes_fts", [], |row| row.get(0))?;
    let embedding_count: usize =
        conn.query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))?;
    Ok(SearchStats {
        fts_count,
        embedding_count,
    })
}
