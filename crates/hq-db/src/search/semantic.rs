//! Embeddings: storage, cosine similarity, semantic and similar-note search.

use anyhow::Result;
use rusqlite::Connection;

use hq_core::types::{MatchType, SearchResult};

use super::{notebook_from_path, title_from_path};

// ─── Cosine Similarity (pure Rust, no external crate) ────────────────────────

/// Compute cosine similarity between two vectors.
/// Returns 0.0 if either vector has zero magnitude.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

/// Batch cosine similarity: `matrix` is a flattened row-major matrix of embeddings,
/// each row has `dim` elements. Returns one similarity score per row.
pub fn batch_cosine_similarity(query: &[f32], matrix: &[f32], dim: usize) -> Vec<f32> {
    matrix
        .chunks(dim)
        .map(|row| cosine_similarity(query, row))
        .collect()
}

// ─── Embedding Serialization Helpers ─────────────────────────────────────────

/// Serialize `&[f32]` to bytes (little-endian, matching JS Float32Array layout).
pub(super) fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize bytes back to `Vec<f32>`.
pub fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    let (chunks, _remainder) = bytes.as_chunks::<4>();
    chunks
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

// ─── Embedding Storage ───────────────────────────────────────────────────────

/// Store (or replace) an embedding for a note, without recording which version of the note it
/// was made from. Prefer `store_embedding_of`.
pub fn store_embedding(
    conn: &Connection,
    path: &str,
    embedding: &[f32],
    model: &str,
) -> Result<()> {
    store_embedding_of(conn, path, embedding, model, None)
}

/// Store (or replace) an embedding for a note, recording the hash of the note it was made from
/// (`content_hash` of the raw file). `pending_embeddings` compares it with the note's current
/// hash to find embeddings that an edit has made stale.
pub fn store_embedding_of(
    conn: &Connection,
    path: &str,
    embedding: &[f32],
    model: &str,
    note_hash: Option<&str>,
) -> Result<()> {
    let blob = embedding_to_bytes(embedding);
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (note_path, embedding, model, embedded_at, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![path, blob, model, now, note_hash],
    )?;
    Ok(())
}

/// Why a note needs an embedding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedReason {
    /// It has none.
    Missing,
    /// It was edited after its embedding was made.
    Edited,
    /// Its embedding came from a different model than the one in use now.
    OtherModel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEmbedding {
    pub path: String,
    pub reason: EmbedReason,
}

/// SQL condition for an embedding made from an older version of its note. `e` is the embedding,
/// `s` the note's `sync_state` row. With a recorded hash it is a comparison. Without one (made
/// before the column existed) the note counts as edited only if its file changed after the
/// embedding was made, so upgrading does not embed everything again.
const STALE_EMBEDDING: &str = "(
    (e.content_hash IS NOT NULL AND s.content_hash IS NOT NULL AND e.content_hash != s.content_hash)
    OR (e.content_hash IS NULL AND s.modified_at IS NOT NULL AND s.modified_at * 1000 > e.embedded_at)
)";

/// Notes that need an embedding made, new notes first, then edited ones, then notes whose
/// embedding came from another model. At most `limit`.
pub fn pending_embeddings(
    conn: &Connection,
    model: &str,
    limit: usize,
) -> Result<Vec<PendingEmbedding>> {
    let sql = format!(
        "SELECT f.path,
                CASE WHEN e.note_path IS NULL THEN 0
                     WHEN e.model != ?1 THEN 2
                     ELSE 1 END AS reason
           FROM notes_fts f
           LEFT JOIN embeddings e ON e.note_path = f.path
           LEFT JOIN sync_state s ON s.path = f.path
          WHERE e.note_path IS NULL OR e.model != ?1 OR {STALE_EMBEDDING}
          ORDER BY (e.note_path IS NULL) DESC, (e.model != ?1) ASC, f.path
          LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![model, limit as i64], |row| {
        let reason = match row.get::<_, i64>(1)? {
            0 => EmbedReason::Missing,
            2 => EmbedReason::OtherModel,
            _ => EmbedReason::Edited,
        };
        Ok(PendingEmbedding {
            path: row.get(0)?,
            reason,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Embeddings made from an older version of their note, whatever model made them.
pub fn stale_embedding_count(conn: &Connection) -> Result<usize> {
    let sql = format!(
        "SELECT COUNT(*)
           FROM embeddings e
           JOIN sync_state s ON s.path = e.note_path
          WHERE {STALE_EMBEDDING}"
    );
    Ok(conn.query_row(&sql, [], |row| row.get(0))?)
}

/// Retrieve the stored embedding for a note. Returns `None` if not found.
pub fn get_embedding(conn: &Connection, path: &str) -> Result<Option<Vec<f32>>> {
    let mut stmt = conn.prepare("SELECT embedding FROM embeddings WHERE note_path = ?1")?;
    let result = stmt.query_row([path], |row| {
        let blob: Vec<u8> = row.get(0)?;
        Ok(blob)
    });
    match result {
        Ok(blob) => Ok(Some(bytes_to_embedding(&blob))),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// ─── Semantic Search ─────────────────────────────────────────────────────────

/// Semantic search: load ALL stored embeddings, compute cosine similarity against
/// the query embedding, return the top `limit` results sorted by similarity desc.
pub fn semantic_search(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<SearchResult>> {
    // Cap the number of loaded embeddings to avoid OOM on very large vaults.
    // 10 000 embeddings at 1536-dim f32 ≈ 60 MB, which is safe for in-memory scoring.
    let mut stmt = conn.prepare("SELECT note_path, embedding FROM embeddings LIMIT 10000")?;
    let rows: Vec<(String, Vec<u8>)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let dim = query_embedding.len();

    // Pack all embeddings into a contiguous matrix for batch processing
    let mut matrix = vec![0.0f32; rows.len() * dim];
    for (i, (_path, blob)) in rows.iter().enumerate() {
        let vec = bytes_to_embedding(blob);
        let copy_len = dim.min(vec.len());
        matrix[i * dim..i * dim + copy_len].copy_from_slice(&vec[..copy_len]);
    }

    let scores = batch_cosine_similarity(query_embedding, &matrix, dim);

    // Pair paths with scores, sort descending
    let mut scored: Vec<(usize, f32)> = scores.iter().copied().enumerate().collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    let mut results = Vec::with_capacity(scored.len());
    for (idx, score) in scored {
        let path = &rows[idx].0;

        // Look up title and tags from FTS table
        let fts_info: Option<(String, String)> = conn
            .prepare("SELECT title, tags FROM notes_fts WHERE path = ?1")?
            .query_row([path.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .ok();

        let (title, tags) = match fts_info {
            Some((t, tg)) => (
                t,
                tg.split_whitespace()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect(),
            ),
            None => (title_from_path(path), Vec::new()),
        };

        results.push(SearchResult {
            note_path: path.clone(),
            title,
            notebook: notebook_from_path(path),
            snippet: String::new(),
            tags,
            relevance: score as f64,
            match_type: MatchType::Semantic,
        });
    }

    Ok(results)
}

// ─── Similar Notes ───────────────────────────────────────────────────────────

/// Find notes most similar to a given note using stored embeddings.
/// Only returns results above the `threshold` cosine similarity.
pub fn find_similar_notes(
    conn: &Connection,
    path: &str,
    limit: usize,
    threshold: f32,
) -> Result<Vec<SearchResult>> {
    let source_emb = match get_embedding(conn, path)? {
        Some(e) => e,
        None => return Ok(Vec::new()),
    };

    // Cap loaded embeddings to avoid OOM on large vaults (see semantic_search for rationale).
    let mut stmt = conn
        .prepare("SELECT note_path, embedding FROM embeddings WHERE note_path != ?1 LIMIT 10000")?;
    let rows: Vec<(String, Vec<u8>)> = stmt
        .query_map([path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let dim = source_emb.len();

    // Pack into matrix
    let mut matrix = vec![0.0f32; rows.len() * dim];
    for (i, (_path, blob)) in rows.iter().enumerate() {
        let vec = bytes_to_embedding(blob);
        let copy_len = dim.min(vec.len());
        matrix[i * dim..i * dim + copy_len].copy_from_slice(&vec[..copy_len]);
    }

    let scores = batch_cosine_similarity(&source_emb, &matrix, dim);

    // Filter by threshold, sort descending, truncate
    let mut scored: Vec<(usize, f32)> = scores
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, s)| *s >= threshold)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);

    let mut results = Vec::with_capacity(scored.len());
    for (idx, score) in scored {
        let note_path = &rows[idx].0;

        let fts_info: Option<(String, String)> = conn
            .prepare("SELECT title, tags FROM notes_fts WHERE path = ?1")?
            .query_row([note_path.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .ok();

        let (title, tags) = match fts_info {
            Some((t, tg)) => (
                t,
                tg.split_whitespace()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect(),
            ),
            None => (title_from_path(note_path), Vec::new()),
        };

        results.push(SearchResult {
            note_path: note_path.clone(),
            title,
            notebook: notebook_from_path(note_path),
            snippet: String::new(),
            tags,
            relevance: score as f64,
            match_type: MatchType::Semantic,
        });
    }

    Ok(results)
}
