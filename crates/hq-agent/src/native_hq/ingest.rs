//! Thread recording, reply annotation, and post-turn memory ingestion.

use std::path::Path;
use std::sync::Arc;

use hq_core::config::HqConfig;

use super::NativeHqHooks;
use crate::session::AgentSession;

pub(super) fn estimate_tokens(text: &str) -> u32 {
    (text.len() as f32 / 4.0) as u32
}

/// Append one entry to the interface's thread file. Best-effort: continuity
/// loss is logged, never fatal to the reply.
pub(super) fn record_thread_entry(
    config: &HqConfig,
    identity: &Option<hq_core::identity::RequestIdentity>,
    role: &str,
    content: &str,
) {
    let Some(id) = identity else { return };
    if let Err(e) = crate::threads::append_thread_entry(&config.vault_path, id, role, content) {
        tracing::warn!(%e, role, "failed to append thread entry");
    }
}

/// The identity whose `_threads/` file this run appends to; none when isolated.
pub(super) fn thread_identity(hooks: &NativeHqHooks) -> Option<hq_core::identity::RequestIdentity> {
    if hooks.isolated {
        None
    } else {
        hooks.identity.clone()
    }
}

pub(super) fn record_thread_turn(
    config: &HqConfig,
    identity: &Option<hq_core::identity::RequestIdentity>,
    prompt: &str,
    reply: &str,
) {
    record_thread_entry(config, identity, "user", prompt);
    record_thread_entry(config, identity, "assistant", reply);
}

/// The reply text, annotated when the turn failed after partial output so it
/// is never shown as a clean answer.
pub(super) fn result_text(result: &hq_core::types::SessionResult) -> String {
    use hq_core::types::SessionResult;
    let text = result.text();
    let marker = match result {
        SessionResult::Failed { error, .. } => format!("_[turn failed: {error}]_"),
        SessionResult::BudgetExhausted(_) => {
            "_[incomplete: stopped at the session budget cap]_".to_string()
        }
        // The session loop already appends its own "[Session stopped: time limit ...]" note.
        SessionResult::TimeLimitReached(_)
        | SessionResult::Complete(_)
        | SessionResult::Cancelled(_) => return text.to_string(),
    };
    if text.is_empty() {
        marker
    } else {
        format!("{text}\n\n{marker}")
    }
}

/// Replies shorter than this carry nothing worth remembering.
const MIN_REPLY_CHARS_FOR_MEMORY: usize = 100;

/// Wire memory extraction: after each completed prompt, the user message and
/// final reply go (fire-and-forget, so it never delays a reply) through the
/// turn gate and one `MemoryIngester` call. `harness` tags the interface that
/// produced the turn ("telegram", "discord", "cli", ...).
pub fn wire_post_turn_ingestion(
    session: &mut AgentSession,
    vault_path: &Path,
    db_path: &Path,
    harness: &str,
) {
    let Ok(db) = hq_db::Database::open(db_path) else {
        return;
    };
    let vault_path = vault_path.to_path_buf();
    let harness = harness.to_string();
    let session_id = session.session_id.clone();
    // Built once per session; the router reads config from disk.
    let router =
        Arc::new(hq_llm::router::LlmRouter::from_env()) as Arc<dyn hq_llm::provider::LlmProvider>;
    let llm = hq_memory::MemoryLlm::with_provider(
        router,
        hq_memory::ingester::INGEST_MODEL_ALIAS.to_string(),
    );
    session.set_post_turn_callback(Arc::new(move |exchange| {
        let reply_chars: usize = exchange
            .iter()
            .filter(|(role, _)| role == "assistant")
            .map(|(_, content)| content.len())
            .sum();
        if reply_chars < MIN_REPLY_CHARS_FOR_MEMORY {
            return;
        }
        let text = exchange
            .iter()
            .map(|(role, content)| format!("{role}: {content}"))
            .collect::<Vec<_>>()
            .join("\n---\n");
        let (db, vault_path, harness) = (db.clone(), vault_path.clone(), harness.clone());
        let (session_id, llm) = (session_id.clone(), llm.clone());
        tokio::spawn(async move {
            // Skips the extraction call for a turn that is confidently ephemeral.
            let decisions = hq_llm::decision::get();
            if !hq_memory::turn_gate::admit(decisions.as_ref(), &session_id, &text).await {
                return;
            }
            let mut ingester = hq_memory::MemoryIngester::new(db, vault_path, llm);
            if let Err(e) = ingester
                .ingest(&text, "chat-turn", Some(&harness), None)
                .await
            {
                tracing::debug!(error = %e, "post-turn memory ingestion skipped");
            }
        });
    }));
}
