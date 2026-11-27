//! Making the on-device vector for each memory, a few at a time.
//!
//! Nothing here fetches a model. The caller says whether the machine may embed
//! (`HqConfig::embeds_locally()`, not on data saver) and [`crate::local_embed::is_ready`] says
//! whether the model is already there; with either answer "no" nothing is embedded and recall
//! stays on keywords. A memory without a vector is simply not found by meaning yet.

use std::future::Future;

use anyhow::Result;
use hq_db::Database;
use tracing::{debug, warn};

/// Memories embedded in one pass, so a first run on a big vault is spread over many cycles.
pub const BATCH: i64 = 200;

/// How a pass went.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pass {
    pub embedded: usize,
    pub failed: usize,
    /// Eligible memories that have a vector from this model, and how many are eligible.
    pub done: i64,
    pub eligible: i64,
}

/// Embed up to `batch` memories that have no vector from `model` yet, with `embed`. A memory
/// that fails is left for the next pass; one failure does not stop the others.
pub async fn embed_pending<F, Fut>(db: &Database, model: &str, batch: i64, embed: F) -> Result<Pass>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Vec<f32>>>,
{
    let todo = db.with_conn(|c| hq_db::memory_embeddings::missing(c, model, batch))?;
    let mut pass = Pass::default();
    for (id, summary) in todo {
        match embed(summary).await {
            Ok(vector) if !vector.is_empty() => {
                db.with_conn(|c| hq_db::memory_embeddings::store(c, id, model, &vector))?;
                pass.embedded += 1;
            }
            Ok(_) => pass.failed += 1,
            Err(e) => {
                // The model is probably unavailable: stop here and try the rest next cycle.
                warn!(error = %e, "memory embedding failed");
                pass.failed += 1;
                break;
            }
        }
    }
    (pass.done, pass.eligible) = db.with_conn(|c| hq_db::memory_embeddings::progress(c, model))?;
    debug!(?pass, "memory embedding pass");
    Ok(pass)
}

/// One pass with the on-device model, or nothing when the model is not already on the machine.
pub async fn embed_pending_local(db: &Database) -> Result<Option<Pass>> {
    if !crate::local_embed::is_ready() {
        return Ok(None);
    }
    embed_pending(db, crate::LOCAL_EMBEDDING_MODEL, BATCH, |text| async move {
        crate::generate_embedding_local(&text).await
    })
    .await
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_with(summaries: &[&str]) -> Database {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for s in summaries {
                c.execute(
                    "INSERT INTO memories (source, harness, raw_text, summary, content, created_at, importance)
                     VALUES ('t', 't', '', ?1, ?1, '2026-01-01T00:00:00Z', 0.5)",
                    [s],
                )?;
            }
            Ok(())
        })
        .unwrap();
        db
    }

    #[tokio::test]
    async fn a_pass_embeds_what_is_missing_and_a_second_pass_finds_nothing_to_do() {
        let db = db_with(&["alpha", "beta", "gamma"]);
        let embed = |t: String| async move { Ok(vec![t.len() as f32, 1.0]) };
        let first = embed_pending(&db, "m", 2, embed).await.unwrap();
        assert_eq!((first.embedded, first.done, first.eligible), (2, 2, 3));
        let second = embed_pending(&db, "m", 2, embed).await.unwrap();
        assert_eq!((second.embedded, second.done), (1, 3));
        let third = embed_pending(&db, "m", 2, embed).await.unwrap();
        assert_eq!(third.embedded, 0);
    }

    #[tokio::test]
    async fn a_failing_model_stops_the_pass_and_loses_nothing() {
        let db = db_with(&["alpha", "beta"]);
        let down = |_t: String| async move { anyhow::bail!("model unavailable") };
        let pass = embed_pending(&db, "m", 10, down).await.unwrap();
        assert_eq!(
            (pass.embedded, pass.failed, pass.done, pass.eligible),
            (0, 1, 0, 2)
        );
        let ok = |_t: String| async move { Ok(vec![1.0, 0.0]) };
        assert_eq!(embed_pending(&db, "m", 10, ok).await.unwrap().embedded, 2);
    }
}
