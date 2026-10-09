//! MemoryIngester — converts raw text into a structured memory entry.
//!
//! One LLM call extracts a summary, entities, topics and importance; the
//! result is stored in `memories`, and each entity gets a concept page.

use anyhow::Result;
use hq_db::Database;
use hq_llm::provider::LlmProvider;
use regex::Regex;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use tracing::{info, warn};

use crate::db::{StoreMemoryParams, store_memory};
use crate::llm_bridge::MemoryLlm;
use crate::types::ExtractedMemory;

/// Metadata extraction prompt: summary, entities, topics, importance.
const METADATA_PROMPT: &str = r#"You are a memory extraction assistant for a personal AI agent hub called Agent-HQ.

Read the text and extract structured metadata. Return a JSON object with exactly:
{
  "summary": "1-2 sentence summary of what happened or was learned",
  "entities": ["array", "of", "key", "names", "projects", "tools"],
  "topics": ["2-4", "topic", "tags"],
  "importance": 0.7
}

importance scale:
- 0.9-1.0: critical decisions, major achievements, key user preferences
- 0.7-0.8: significant work done, useful insights, project updates
- 0.5-0.6: routine task completions, minor notes
- 0.3-0.4: low-value background info"#;

/// Router alias for background extraction; resolves through the backend
/// chain when `backends:` is configured.
pub const INGEST_MODEL_ALIAS: &str = "bulk";

/// Texts shorter than this carry nothing worth remembering.
const MIN_TEXT_CHARS: usize = 30;
/// How much of the text the extraction call sees.
const PROMPT_TEXT_CHARS: usize = 2000;
/// How much raw text is stored alongside the summary.
const STORED_TEXT_CHARS: usize = 4000;

/// Salience markers boost importance at encoding time, so decisions,
/// failures and milestones survive decay.
static SALIENCE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(deadline|urgent|critical|blocker|blocked|decision|breakthrough|failed|failure|error|crash|security|vulnerability|breach|approved|rejected|milestone|launch|shipped|signed|cancelled|crisis|risk|escalat|budget|contract|deal|acquisition|pivot|layoff|hire|resign|fund(?:ing|ed)?|raise|partnership)\b").unwrap()
});

fn apply_salience_boost(text: &str, importance: f64, topics: &mut Vec<String>) -> f64 {
    if !SALIENCE_PATTERN.is_match(text) {
        return importance;
    }
    if !topics.iter().any(|t| t == "high-salience") {
        topics.push("high-salience".to_string());
    }
    (importance * 1.5).min(1.0)
}

/// Memory ingester — extracts and stores structured memory from raw text.
pub struct MemoryIngester {
    db: Database,
    vault_path: PathBuf,
    llm: MemoryLlm,
}

impl MemoryIngester {
    /// An ingester on the given LLM. Build the LLM once per session (see
    /// [`MemoryLlm::with_provider`] with [`INGEST_MODEL_ALIAS`]), not per turn.
    pub fn new(db: Database, vault_path: PathBuf, llm: MemoryLlm) -> Self {
        Self {
            db,
            vault_path,
            llm,
        }
    }

    pub fn new_with_provider(
        db: Database,
        vault_path: PathBuf,
        provider: Arc<dyn LlmProvider>,
        model: String,
    ) -> Self {
        Self::new(db, vault_path, MemoryLlm::with_provider(provider, model))
    }

    /// Ingest a piece of text as a memory. Returns the new memory ID, or
    /// None when the text is too short or extraction fails.
    pub async fn ingest(
        &mut self,
        text: &str,
        source: &str,
        harness: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Option<i64>> {
        let harness = harness.unwrap_or("unknown");
        if text.trim().len() < MIN_TEXT_CHARS {
            return Ok(None);
        }
        // FR-008: turn text can carry pasted secrets, and both the extraction LLM and the store would keep them.
        let text = &hq_core::redact::redact_secrets(text);

        let truncated: String = text.chars().take(PROMPT_TEXT_CHARS).collect();
        let user_msg = format!("Extract from this text:\n\n{truncated}");
        let extracted = match self
            .llm
            .json::<ExtractedMemory>(METADATA_PROMPT, &user_msg)
            .await
        {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "memory extraction failed");
                return Ok(None);
            }
        };
        if extracted.summary.is_empty() {
            warn!("memory extraction returned an empty summary, skipping");
            return Ok(None);
        }

        let mut topics: Vec<String> = extracted.topics.into_iter().take(5).collect();
        let importance =
            apply_salience_boost(text, extracted.importance.clamp(0.0, 1.0), &mut topics);
        let raw_text: String = text.chars().take(STORED_TEXT_CHARS).collect();
        let entities: Vec<String> = extracted.entities.into_iter().take(10).collect();

        let id = store_memory(
            &self.db,
            &StoreMemoryParams {
                source,
                harness,
                raw_text: &raw_text,
                summary: &hq_core::redact::redact_secrets(&extracted.summary),
                entities: &entities,
                topics: &topics,
                importance,
                user_id,
            },
        )?;

        // One concept page per entity, wikilinked to the entities it appeared
        // with. derive_entity_index (consolidator) rebuilds the entity tables.
        if !entities.is_empty()
            && let Ok(vault) = hq_vault::VaultClient::new(self.vault_path.clone())
        {
            let source_ref = format!("memory:{id}");
            for entity in &entities {
                let related: Vec<String> =
                    entities.iter().filter(|e| *e != entity).cloned().collect();
                let entity_type = crate::entity_graph::classify_entity_type(entity);
                let _ = crate::concept_pages::upsert_concept_page(
                    &vault,
                    entity,
                    entity_type,
                    &related,
                    &source_ref,
                );
            }
        }

        info!(id, source, summary = %extracted.summary.chars().take(80).collect::<String>(), "Ingested memory");
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ingest_column_user_id_exists() {
        use hq_db::Database;
        let db = Database::open_memory().unwrap();

        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO memories (content, source, created_at, importance, user_id)
                 VALUES ('test content', 'test', unixepoch(), 0.5, 'alice')",
                [],
            )?;
            let row_id = conn.last_insert_rowid();
            let uid: String = conn.query_row(
                "SELECT user_id FROM memories WHERE id = ?1",
                rusqlite::params![row_id],
                |r: &rusqlite::Row| r.get(0),
            )?;
            assert_eq!(uid, "alice");
            Ok(())
        })
        .unwrap();
    }

    /// In-process fake `LlmProvider` returning a fixed extraction so tests
    /// don't depend on a live model.
    #[derive(Default)]
    struct FakeProvider {
        prompts: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl hq_llm::provider::LlmProvider for FakeProvider {
        fn name(&self) -> &str {
            "fake"
        }

        async fn chat(
            &self,
            request: &hq_llm::provider::ChatRequest,
        ) -> anyhow::Result<hq_llm::provider::ChatResponse> {
            use hq_core::types::{ChatMessage, MessageRole};

            let mut prompts = self.prompts.lock().unwrap();
            prompts.extend(request.messages.iter().map(|m| m.content.clone()));

            Ok(hq_llm::provider::ChatResponse {
                message: ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::Assistant,
                    content: r#"{"summary": "test summary", "entities": ["Rust", "Agent HQ"], "topics": ["test"], "importance": 0.5}"#.to_string(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                },
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                provider_cost_usd: None,
                model: "fake-model".to_string(),
            })
        }

        async fn chat_stream(
            &self,
            _request: &hq_llm::provider::ChatRequest,
        ) -> anyhow::Result<
            std::pin::Pin<
                Box<
                    dyn tokio_stream::Stream<Item = anyhow::Result<hq_llm::provider::StreamChunk>>
                        + Send,
                >,
            >,
        > {
            anyhow::bail!("chat_stream not used by MemoryLlm::json")
        }
    }

    #[tokio::test]
    async fn ingest_writes_concept_pages_for_co_occurring_entities_instead_of_direct_edges() {
        use super::MemoryIngester;
        use hq_db::Database;
        use hq_vault::VaultClient;
        use std::sync::Arc;

        let db = Database::open_memory().unwrap();

        let dir = std::env::temp_dir().join(format!(
            "hq-ingester-concept-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut ingester = MemoryIngester::new_with_provider(
            db.clone(),
            dir.clone(),
            Arc::new(FakeProvider::default()),
            "fake-model".to_string(),
        );
        let result = ingester
            .ingest(
                "Alex discussed Rust and Agent HQ together in this message.",
                "test",
                Some("test-harness"),
                None,
            )
            .await;
        assert!(result.is_ok(), "ingest should succeed: {result:?}");

        // The old direct-write path would have inserted rows straight into
        // entity_nodes/entity_edges. Confirm ingest no longer does that —
        // entity_nodes/entity_edges stay empty until derive_entity_index runs.
        let node_count: i64 = db
            .with_conn(|conn| {
                conn.query_row("SELECT COUNT(*) FROM entity_nodes", [], |r| r.get(0))
                    .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(
            node_count, 0,
            "ingest must not write entity_nodes directly anymore — concept pages are the source of truth"
        );

        // Confirm concept pages were written under _graph/ instead.
        let vault = VaultClient::new(dir).unwrap();
        let pages = vault.list_notes_recursive("_graph").unwrap();
        assert!(
            !pages.is_empty(),
            "ingest should have written at least one concept page for an extracted entity"
        );
    }

    #[tokio::test]
    async fn ingest_redacts_secrets_from_prompt_and_stored_text() {
        use super::MemoryIngester;
        use hq_db::Database;
        use std::sync::Arc;

        const SECRET: &str = "sk-abcdefghijklmnopqrstuvwx1234";
        let db = Database::open_memory().unwrap();
        let provider = Arc::new(FakeProvider::default());
        let mut ingester = MemoryIngester::new_with_provider(
            db.clone(),
            std::env::temp_dir().join("hq-ingester-redact-test"),
            provider.clone(),
            "fake-model".to_string(),
        );

        let id = ingester
            .ingest(
                &format!("Here is my OpenRouter key {SECRET}, please remember it."),
                "test",
                None,
                None,
            )
            .await
            .unwrap()
            .expect("ingest should store a memory");

        let raw_text: String = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT raw_text FROM memories WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert!(
            !raw_text.contains(SECRET),
            "stored raw_text leaked: {raw_text}"
        );
        assert!(raw_text.contains("[REDACTED]"));
        let prompts = provider.prompts.lock().unwrap();
        assert!(!prompts.is_empty());
        assert!(
            prompts.iter().all(|p| !p.contains(SECRET)),
            "prompt leaked the secret"
        );
    }
}
