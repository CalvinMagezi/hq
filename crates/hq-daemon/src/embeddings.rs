//! Batch embedding processor: finds notes whose vector embedding is missing, out of date or from
//! another model, and makes it, via OpenRouter when an API key is configured (the default on
//! hosts with no local Ollama, e.g. the VPS) or otherwise via Ollama's local embedding API
//! (nomic-embed-text).
//!
//! Embeddings are stored as raw f32 little-endian bytes in the `embeddings` table, matching the
//! format expected by `hq_db::search::bytes_to_embedding`, together with the hash of the note
//! they were made from. An edit changes that hash, which is how an edited note is told apart
//! from an unchanged one.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use hq_core::config::EmbeddingProvider;
use hq_db::Database;
use hq_db::search::{
    NOTEBOOKS_DIR, PathOutcome, content_hash, pending_embeddings, reconcile_path,
    remove_path_everywhere, store_embedding_of,
};
use hq_memory::generate_embedding;
use hq_vault::VaultClient;
use tracing::{debug, info, warn};

/// Stop a batch after this many failures in a row. A provider that is down or not configured
/// fails every note, and there is no point asking it twenty times.
const MAX_CONSECUTIVE_FAILURES: usize = 3;

/// Runs of the embedder are serialized, so the scheduled task and the background worker never
/// embed the same note twice at once.
static RUN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How the embedding for a piece of text is made. Boxed so a test can stand in for the network.
pub type Embedder =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<Vec<f32>>> + Send>> + Send + Sync>;

/// The embedder to use and the model name its vectors are labeled with. `Local` never falls
/// back to a remote provider: the owner chose to keep note text on this machine.
pub fn embedder_for(
    provider: EmbeddingProvider,
    openrouter_api_key: Option<&str>,
) -> (Embedder, String) {
    match (provider, openrouter_api_key) {
        (EmbeddingProvider::Local, _) => {
            let embedder: Embedder = Arc::new(|text| {
                Box::pin(async move { hq_memory::generate_embedding_local(&text).await })
            });
            (embedder, hq_memory::LOCAL_EMBEDDING_MODEL.to_string())
        }
        (EmbeddingProvider::Remote, Some(key)) => {
            let key = key.to_string();
            let embedder: Embedder = Arc::new(move |text| {
                let key = key.clone();
                Box::pin(async move { hq_memory::generate_embedding_openrouter(&text, &key).await })
            });
            (embedder, hq_memory::openrouter_embedding_model())
        }
        (EmbeddingProvider::Remote, None) => {
            let embedder: Embedder =
                Arc::new(|text| Box::pin(async move { generate_embedding(&text).await }));
            (embedder, "nomic-embed-text".to_string())
        }
    }
}

/// What one batch did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EmbedReport {
    pub embedded: usize,
    /// The embedder returned an error or nothing.
    pub failed: usize,
    /// The note could not be read, or no longer exists and was cleaned out of the index.
    pub unreadable: usize,
    /// The batch was stopped early because the embedder failed several times in a row.
    pub gave_up: bool,
    /// The latest error from the embedder, for health output.
    pub last_error: Option<String>,
}

impl EmbedReport {
    /// Whether the batch found anything to do at all.
    pub fn idle(&self) -> bool {
        self.embedded == 0 && self.failed == 0 && self.unreadable == 0
    }
}

/// Embed up to `batch_size` notes that need it: new notes first, then edited ones, then notes
/// embedded with a model other than `model`.
///
/// `skip` holds notes that could not be read. They are passed over rather than retried on every
/// batch, so a few unreadable files cannot occupy the whole queue.
pub async fn embed_pending(
    vault: &VaultClient,
    db: &Database,
    model: &str,
    embedder: &Embedder,
    batch_size: usize,
    skip: &mut HashSet<String>,
) -> Result<EmbedReport> {
    let _running = RUN_LOCK.lock().await;
    let mut report = EmbedReport::default();

    let wanted = batch_size + skip.len();
    let model_owned = model.to_string();
    let pending = db.with_conn(move |conn| pending_embeddings(conn, &model_owned, wanted))?;
    let pending: Vec<_> = pending
        .into_iter()
        .filter(|p| !skip.contains(&p.path))
        .take(batch_size)
        .collect();
    if pending.is_empty() {
        debug!("no pending embeddings to process");
        return Ok(report);
    }
    info!(count = pending.len(), "processing pending embeddings");

    let mut consecutive_failures = 0;
    for item in &pending {
        // Read the raw file first: its hash is what gets recorded, so an edit that lands while
        // the provider is working makes the embedding look stale and it is made again.
        let raw = match std::fs::read_to_string(vault.vault_path().join(&item.path)) {
            Ok(raw) => raw,
            Err(e) => {
                report.unreadable += 1;
                forget_if_gone(vault, db, &item.path, &e, skip);
                continue;
            }
        };
        let hash = content_hash(&raw);
        let note = match vault.read_note(&item.path) {
            Ok(note) => note,
            Err(e) => {
                warn!(path = %item.path, error = %e, "skipping note, read failed");
                report.unreadable += 1;
                skip.insert(item.path.clone());
                continue;
            }
        };

        // Compact text for embedding: title + first 512 chars of body.
        let body_preview: String = note.content.chars().take(512).collect();
        let text = format!("{}\n\n{}", note.title, body_preview);

        match embedder(text).await {
            Ok(embedding) if !embedding.is_empty() => {
                let (path, model) = (item.path.clone(), model.to_string());
                let dims = embedding.len();
                if let Err(e) = db.with_conn(move |conn| {
                    store_embedding_of(conn, &path, &embedding, &model, Some(&hash))
                }) {
                    warn!(path = %item.path, error = %e, "failed to store embedding");
                    report.failed += 1;
                    continue;
                }
                consecutive_failures = 0;
                report.embedded += 1;
                debug!(path = %item.path, dims, reason = ?item.reason, "embedding stored");
            }
            Ok(_) => {
                warn!(path = %item.path, "embedding provider returned an empty vector");
                report.failed += 1;
                consecutive_failures += 1;
                report.last_error = Some("the embedding provider returned an empty vector".into());
            }
            Err(e) => {
                warn!(path = %item.path, error = %e, "embedding generation failed");
                report.failed += 1;
                consecutive_failures += 1;
                report.last_error = Some(e.to_string());
            }
        }
        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            warn!(
                failures = consecutive_failures,
                "embedding provider keeps failing; stopping this batch"
            );
            report.gave_up = true;
            break;
        }
    }

    info!(
        embedded = report.embedded,
        failed = report.failed,
        unreadable = report.unreadable,
        total = pending.len(),
        "embedding batch complete"
    );
    Ok(report)
}

/// A note that cannot be read is either gone (the file was deleted and the index still lists it:
/// clean it out) or unreadable (leave it alone and stop asking about it).
fn forget_if_gone(
    vault: &VaultClient,
    db: &Database,
    path: &str,
    error: &std::io::Error,
    skip: &mut HashSet<String>,
) {
    if error.kind() == std::io::ErrorKind::NotFound {
        let (vault_path, rel) = (vault.vault_path().to_path_buf(), path.to_string());
        let cleaned = db.with_conn(move |conn| match reconcile_path(conn, &vault_path, &rel)? {
            // The scan only covers Notebooks/, so a note outside it (a skill, say) that was
            // deleted behind HQ's back is never cleaned there and would be skipped on every
            // run. Its file is missing, so drop what is held for it. An absent Notebooks/ is a
            // missing mount as likely as an empty vault, so remove nothing then.
            PathOutcome::Ignored if vault_path.join(NOTEBOOKS_DIR).is_dir() => {
                remove_path_everywhere(conn, &rel)?;
                Ok(PathOutcome::Removed)
            }
            other => Ok(other),
        });
        if matches!(cleaned, Ok(PathOutcome::Removed)) {
            debug!(path, "note is gone; removed it from the index");
            return;
        }
    }
    warn!(path, error = %error, "skipping note, read failed");
    skip.insert(path.to_string());
}

/// Process a batch of notes that need vector embeddings (up to `batch_size`).
///
/// Generates embeddings with `embedder_for`, and stores the results labeled with the model that produced
/// them and the hash of the note they came from.
///
/// Returns the number of notes successfully embedded.
pub async fn process_embeddings(
    vault: &VaultClient,
    db: &Database,
    batch_size: usize,
    provider: EmbeddingProvider,
    openrouter_api_key: Option<&str>,
) -> Result<usize> {
    let (embedder, model) = embedder_for(provider, openrouter_api_key);
    let mut skip = HashSet::new();
    let report = embed_pending(vault, db, &model, &embedder, batch_size, &mut skip).await?;
    Ok(report.embedded)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hq_db::search::EmbedReason;

    use super::*;

    /// A stand-in embedder that counts its calls, can fail on demand, and returns a vector that
    /// depends on the text so a replaced embedding is visible.
    struct Fake {
        calls: AtomicUsize,
        fail: AtomicUsize,
        texts: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                fail: AtomicUsize::new(0),
                texts: Mutex::new(Vec::new()),
            })
        }

        fn embedder(self: &Arc<Self>) -> Embedder {
            let fake = self.clone();
            Arc::new(move |text| {
                let fake = fake.clone();
                Box::pin(async move {
                    fake.calls.fetch_add(1, Ordering::SeqCst);
                    fake.texts.lock().unwrap().push(text.clone());
                    if fake.fail.load(Ordering::SeqCst) > 0 {
                        fake.fail.fetch_sub(1, Ordering::SeqCst);
                        anyhow::bail!("provider is down");
                    }
                    Ok(vec![text.len() as f32, 1.0])
                })
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    struct Rig {
        dir: tempfile::TempDir,
        vault: VaultClient,
        db: Database,
    }

    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
            Self {
                vault: VaultClient::new(dir.path().to_path_buf()).unwrap(),
                db: Database::open_memory().unwrap(),
                dir,
            }
        }

        fn write(&self, rel: &str, body: &str) {
            let path = self.dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }

        fn reindex(&self) {
            let notebooks = self.dir.path().join("Notebooks");
            self.db
                .with_conn(move |c| hq_db::search::sync_index(c, &notebooks))
                .unwrap();
        }

        async fn embed(&self, fake: &Arc<Fake>, model: &str, batch: usize) -> EmbedReport {
            let mut skip = HashSet::new();
            embed_pending(
                &self.vault,
                &self.db,
                model,
                &fake.embedder(),
                batch,
                &mut skip,
            )
            .await
            .unwrap()
        }

        fn vector(&self, rel: &str) -> Option<Vec<f32>> {
            let rel = rel.to_string();
            self.db
                .with_conn(move |c| hq_db::search::get_embedding(c, &rel))
                .unwrap()
        }

        fn stored_hash(&self, rel: &str) -> Option<String> {
            let rel = rel.to_string();
            self.db
                .with_conn(move |c| {
                    Ok(c.query_row(
                        "SELECT content_hash FROM embeddings WHERE note_path = ?1",
                        [rel],
                        |r| r.get(0),
                    )?)
                })
                .unwrap()
        }

        fn keyword_hits(&self, word: &str) -> usize {
            let word = word.to_string();
            self.db
                .with_conn(move |c| hq_db::search::keyword_search(c, &word, 10))
                .unwrap()
                .len()
        }

        fn pending(&self, model: &str) -> Vec<(String, EmbedReason)> {
            let model = model.to_string();
            self.db
                .with_conn(move |c| pending_embeddings(c, &model, 100))
                .unwrap()
                .into_iter()
                .map(|p| (p.path, p.reason))
                .collect()
        }
    }

    #[tokio::test]
    async fn a_new_note_is_embedded_once_and_an_unchanged_note_is_left_alone() {
        let rig = Rig::new();
        let fake = Fake::new();
        rig.write("Notebooks/a.md", "alpha body");
        rig.reindex();

        let first = rig.embed(&fake, "m", 10).await;
        assert_eq!((first.embedded, first.failed), (1, 0));
        assert_eq!(fake.calls(), 1);
        assert!(rig.vector("Notebooks/a.md").is_some());
        let raw = std::fs::read_to_string(rig.dir.path().join("Notebooks/a.md")).unwrap();
        assert_eq!(rig.stored_hash("Notebooks/a.md"), Some(content_hash(&raw)));

        // Nothing changed, so the provider is not called again.
        let second = rig.embed(&fake, "m", 10).await;
        assert!(second.idle());
        assert_eq!(fake.calls(), 1);
    }

    #[tokio::test]
    async fn an_edited_note_gets_a_new_embedding_that_replaces_the_old() {
        let rig = Rig::new();
        let fake = Fake::new();
        rig.write("Notebooks/a.md", "short");
        rig.reindex();
        rig.embed(&fake, "m", 10).await;
        let before = rig.vector("Notebooks/a.md").unwrap();

        rig.write(
            "Notebooks/a.md",
            "a considerably longer replacement body of text",
        );
        rig.reindex();
        assert_eq!(
            rig.pending("m"),
            vec![("Notebooks/a.md".to_string(), EmbedReason::Edited)]
        );
        let report = rig.embed(&fake, "m", 10).await;

        assert_eq!(report.embedded, 1);
        assert_eq!(fake.calls(), 2);
        let after = rig.vector("Notebooks/a.md").unwrap();
        assert_ne!(before, after);
        let raw = std::fs::read_to_string(rig.dir.path().join("Notebooks/a.md")).unwrap();
        assert_eq!(rig.stored_hash("Notebooks/a.md"), Some(content_hash(&raw)));
        assert!(fake.texts.lock().unwrap()[1].contains("considerably longer"));
        assert!(rig.pending("m").is_empty());
    }

    #[tokio::test]
    async fn a_model_change_embeds_every_note_again() {
        let rig = Rig::new();
        let fake = Fake::new();
        rig.write("Notebooks/a.md", "alpha");
        rig.write("Notebooks/b.md", "bravo");
        rig.reindex();
        rig.embed(&fake, "old-model", 10).await;
        assert_eq!(fake.calls(), 2);

        let report = rig.embed(&fake, "new-model", 10).await;
        assert_eq!(report.embedded, 2);
        assert_eq!(fake.calls(), 4);
        assert!(rig.pending("new-model").is_empty());
    }

    #[tokio::test]
    async fn a_provider_failure_leaves_search_current_and_the_note_pending_for_later() {
        let rig = Rig::new();
        let fake = Fake::new();
        rig.write("Notebooks/a.md", "pelican body");
        rig.reindex();
        fake.fail.store(1, Ordering::SeqCst);

        let report = rig.embed(&fake, "m", 10).await;

        assert_eq!((report.embedded, report.failed), (0, 1));
        assert!(rig.vector("Notebooks/a.md").is_none());
        assert_eq!(
            rig.keyword_hits("pelican"),
            1,
            "keyword search is unaffected"
        );
        assert_eq!(rig.pending("m").len(), 1, "still waiting for an embedding");

        // The provider recovers; the next pass makes it.
        let retry = rig.embed(&fake, "m", 10).await;
        assert_eq!(retry.embedded, 1);
        assert!(rig.vector("Notebooks/a.md").is_some());
    }

    #[tokio::test]
    async fn a_provider_that_keeps_failing_is_asked_a_few_times_not_for_every_note() {
        let rig = Rig::new();
        let fake = Fake::new();
        for i in 0..10 {
            rig.write(&format!("Notebooks/n{i}.md"), &format!("note {i}"));
        }
        rig.reindex();
        fake.fail.store(1_000, Ordering::SeqCst);

        let report = rig.embed(&fake, "m", 10).await;

        assert!(report.gave_up);
        assert_eq!(fake.calls(), MAX_CONSECUTIVE_FAILURES);
        assert_eq!(report.failed, MAX_CONSECUTIVE_FAILURES);
        assert_eq!(report.last_error.as_deref(), Some("provider is down"));
    }

    #[tokio::test]
    async fn a_note_deleted_from_disk_is_cleaned_out_not_retried_forever() {
        let rig = Rig::new();
        let fake = Fake::new();
        rig.write("Notebooks/gone.md", "about to vanish");
        rig.write("Notebooks/kept.md", "stays");
        rig.reindex();
        std::fs::remove_file(rig.dir.path().join("Notebooks/gone.md")).unwrap();

        let report = rig.embed(&fake, "m", 10).await;

        assert_eq!(report.embedded, 1, "the note that exists is embedded");
        assert_eq!(report.unreadable, 1);
        assert!(rig.vector("Notebooks/gone.md").is_none());
        assert_eq!(rig.keyword_hits("vanish"), 0, "removed from the index");
        assert!(rig.pending("m").is_empty());
    }

    #[tokio::test]
    async fn a_note_deleted_from_outside_notebooks_is_cleaned_out_too() {
        let rig = Rig::new();
        let fake = Fake::new();
        // A tool indexed a skill; its folder was later removed some other way. The scan only
        // covers Notebooks/, so nothing else would ever clear it.
        rig.db
            .with_conn(|c| {
                hq_db::search::index_note(c, "skills/old/SKILL.md", "old", "stale words", "")
            })
            .unwrap();
        assert_eq!(rig.keyword_hits("stale"), 1);

        let report = rig.embed(&fake, "m", 10).await;

        assert_eq!(report.unreadable, 1);
        assert_eq!(rig.keyword_hits("stale"), 0, "removed from the index");
        assert!(rig.pending("m").is_empty());
        assert!(
            rig.embed(&fake, "m", 10).await.idle(),
            "nothing left to retry"
        );
    }

    #[tokio::test]
    async fn nothing_is_removed_when_notebooks_itself_is_missing() {
        let rig = Rig::new();
        let fake = Fake::new();
        rig.db
            .with_conn(|c| {
                hq_db::search::index_note(c, "skills/old/SKILL.md", "old", "stale words", "")
            })
            .unwrap();
        // An unmounted vault looks like this: the folder that should hold everything is gone.
        std::fs::remove_dir_all(rig.dir.path().join("Notebooks")).unwrap();

        let report = rig.embed(&fake, "m", 10).await;

        assert_eq!(report.unreadable, 1);
        assert_eq!(
            rig.keyword_hits("stale"),
            1,
            "kept: this may be a missing mount"
        );
    }

    #[tokio::test]
    async fn an_unreadable_note_does_not_starve_the_ones_behind_it() {
        let rig = Rig::new();
        let fake = Fake::new();
        // Sorts first, exists, but is not valid UTF-8, so it can never be embedded.
        std::fs::write(
            rig.dir.path().join("Notebooks/a-broken.md"),
            [0xff, 0xfe, 0x00],
        )
        .unwrap();
        rig.write("Notebooks/b.md", "readable");
        // The scan cannot read the broken file, so index it directly like a tool write would.
        rig.db
            .with_conn(|c| {
                hq_db::search::index_note(c, "Notebooks/a-broken.md", "a-broken", "x", "")
            })
            .unwrap();
        rig.reindex();

        let mut skip = HashSet::new();
        let embedder = fake.embedder();
        // Batch of one: without the skip set the broken note would be picked forever.
        let first = embed_pending(&rig.vault, &rig.db, "m", &embedder, 1, &mut skip)
            .await
            .unwrap();
        assert_eq!((first.unreadable, first.embedded), (1, 0));
        let second = embed_pending(&rig.vault, &rig.db, "m", &embedder, 1, &mut skip)
            .await
            .unwrap();
        assert_eq!(second.embedded, 1);
        assert!(rig.vector("Notebooks/b.md").is_some());
    }

    #[tokio::test]
    async fn the_wrapper_reports_how_many_notes_it_embedded() {
        // process_embeddings is the scheduled task's entry point; with nothing pending it must
        // not call a provider at all.
        let rig = Rig::new();
        let vault = VaultClient::new(Path::new(rig.dir.path()).to_path_buf()).unwrap();
        assert_eq!(
            process_embeddings(&vault, &rig.db, 5, EmbeddingProvider::Remote, None)
                .await
                .unwrap(),
            0
        );
    }
}
