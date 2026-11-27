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

/// Whether a turn may be learned from. Memory belongs to the owner, so a family guest's or a
/// restricted audience's turns are never ingested: they would resurface later as facts about
/// the owner. A turn with no identity (CLI, daemon) is the owner's own. `skip` is the caller's
/// own opt-out (an MCP `hq_ask` turn).
pub(super) fn learns_from_turn(
    identity: Option<&hq_core::identity::RequestIdentity>,
    skip: bool,
) -> bool {
    !skip && identity.is_none_or(hq_core::identity::RequestIdentity::acts_for_owner)
}

/// Replies shorter than this carry nothing worth remembering.
const MIN_REPLY_CHARS_FOR_MEMORY: usize = 100;

/// Where a turn came from, kept with the memory learned from it.
#[derive(Debug, Clone)]
pub struct TurnSource {
    /// The interface that produced the turn: "telegram", "discord", "web", "cli", ...
    pub harness: String,
    /// Whose turn it was, when the request carried an identity.
    pub user_id: Option<String>,
    /// The web chat it happened in, so the memory can be traced back to it.
    pub thread_id: Option<String>,
}

impl TurnSource {
    /// A turn typed into the terminal chat.
    pub fn cli() -> Self {
        Self {
            harness: "cli".to_string(),
            user_id: None,
            thread_id: None,
        }
    }

    /// The source of a turn from this identity; no identity is a CLI or daemon turn.
    pub fn from_identity(identity: Option<&hq_core::identity::RequestIdentity>) -> Self {
        let Some(id) = identity else {
            return Self::cli();
        };
        let thread_id = match &id.source {
            hq_core::identity::RequestSource::Web { thread_id, .. } => Some(thread_id.clone()),
            _ => None,
        };
        Self {
            harness: id.source.label().to_string(),
            user_id: Some(id.user_id.clone()),
            thread_id,
        }
    }
}

/// Wire memory extraction: after each completed prompt, the user message and
/// final reply go (fire-and-forget, so it never delays a reply) through the
/// turn gate and one `MemoryIngester` call. `source` says which interface and
/// chat the turn came from. Only the owner's turns are ever wired (see
/// `learns_from_turn`), so every memory made here is the owner's.
pub fn wire_post_turn_ingestion(
    session: &mut AgentSession,
    vault_path: &Path,
    db_path: &Path,
    source: TurnSource,
) {
    let Ok(db) = hq_db::Database::open(db_path) else {
        return;
    };
    let vault_path = vault_path.to_path_buf();
    let session_id = session.session_id.clone();
    // Built once per session; the router reads config from disk. `None` extracts without a model.
    let local = hq_core::config::HqConfig::load()
        .is_ok_and(|c| c.daemon.memory_extraction == hq_core::config::MemoryExtraction::Local);
    let llm = (!local).then(|| {
        let router = Arc::new(hq_llm::router::LlmRouter::from_env())
            as Arc<dyn hq_llm::provider::LlmProvider>;
        hq_memory::MemoryLlm::with_provider(
            router,
            hq_memory::ingester::INGEST_MODEL_ALIAS.to_string(),
        )
    });
    session.set_post_turn_callback(Arc::new(move |exchange| {
        // Paused from Settings, What HQ remembers: nothing new is saved. Read every turn so a
        // change applies to a chat that is already open.
        if !hq_db::owner_prefs::memory_learning_on(&db) {
            return;
        }
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
        let (db, vault_path, source) = (db.clone(), vault_path.clone(), source.clone());
        let (session_id, llm) = (session_id.clone(), llm.clone());
        tokio::spawn(async move {
            // Skips the extraction call for a turn that is confidently ephemeral.
            let decisions = hq_llm::decision::get();
            if !hq_memory::turn_gate::admit(decisions.as_ref(), &session_id, &text).await {
                return;
            }
            let mut ingester = match llm {
                Some(llm) => hq_memory::MemoryIngester::new(db, vault_path, llm),
                None => hq_memory::MemoryIngester::new_local(db, vault_path),
            };
            let trace = hq_memory::Trace {
                thread_id: source.thread_id.as_deref(),
                ..hq_memory::Trace::default()
            };
            if let Err(e) = ingester
                .ingest(
                    &text,
                    "chat-turn",
                    Some(&source.harness),
                    source.user_id.as_deref(),
                    &trace,
                )
                .await
            {
                tracing::debug!(error = %e, "post-turn memory ingestion skipped");
            }
        });
    }));
}
