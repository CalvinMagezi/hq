//! Batch embedding processor — finds notes in FTS5 that lack vector embeddings
//! and generates them, via OpenRouter when an API key is configured (the
//! default on hosts with no local Ollama, e.g. the VPS) or otherwise via
//! Ollama's local embedding API (nomic-embed-text).
//!
//! Embeddings are stored as raw f32 little-endian bytes in the `embeddings` table,
//! matching the format expected by `hq_db::search::bytes_to_embedding`.

use anyhow::Result;
use hq_db::Database;
use hq_memory::generate_embedding;
use hq_vault::VaultClient;
use tracing::{debug, info, warn};

/// Process a batch of notes that need vector embeddings.
///
/// Finds notes in `notes_fts` that have no entry in `embeddings` (up to `batch_size`),
/// generates embeddings — via OpenRouter if `openrouter_api_key` is `Some`, otherwise
/// via Ollama's local `/api/embeddings` endpoint — and stores the results as f32
/// little-endian blobs, labeled with whichever model actually produced them.
///
/// Returns the number of notes successfully embedded.
pub async fn process_embeddings(
    vault: &VaultClient,
    db: &Database,
    batch_size: usize,
    openrouter_api_key: Option<&str>,
) -> Result<usize> {
    // Notes in FTS5 but not yet embedded
    let pending_paths = db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT path FROM notes_fts \
             WHERE path NOT IN (SELECT note_path FROM embeddings) \
             LIMIT ?1",
        )?;
        let rows = stmt.query_map([batch_size], |row| row.get::<_, String>(0))?;
        let mut paths = Vec::new();
        for row in rows {
            paths.push(row?);
        }
        Ok(paths)
    })?;

    if pending_paths.is_empty() {
        debug!("no pending embeddings to process");
        return Ok(0);
    }

    info!(count = pending_paths.len(), "processing pending embeddings");
    let mut processed = 0usize;
    let embed_model = match openrouter_api_key {
        Some(_) => hq_memory::openrouter_embedding_model(),
        None => "nomic-embed-text".to_string(),
    };

    for note_path in &pending_paths {
        let note = match vault.read_note(note_path) {
            Ok(n) => n,
            Err(e) => {
                warn!(path = %note_path, error = %e, "skipping note — read failed");
                continue;
            }
        };

        // Build compact text for embedding: title + first 512 chars of body
        let body_preview: String = note.content.chars().take(512).collect();
        let embed_text = format!("{}\n\n{}", note.title, body_preview);

        let result = match openrouter_api_key {
            Some(key) => hq_memory::generate_embedding_openrouter(&embed_text, key).await,
            None => generate_embedding(&embed_text).await,
        };

        match result {
            Ok(embedding) if !embedding.is_empty() => {
                if let Err(e) = db.with_conn(|conn| {
                    hq_db::search::store_embedding(conn, note_path, &embedding, &embed_model)
                }) {
                    warn!(path = %note_path, error = %e, "failed to store embedding");
                    continue;
                }

                processed += 1;
                debug!(path = %note_path, dims = embedding.len(), "embedding stored");
            }
            Ok(_) => {
                warn!(path = %note_path, "embedding provider returned an empty vector");
            }
            Err(e) => {
                warn!(path = %note_path, error = %e, "embedding generation failed");
            }
        }
    }

    info!(
        processed,
        total = pending_paths.len(),
        "embedding batch complete"
    );
    Ok(processed)
}
