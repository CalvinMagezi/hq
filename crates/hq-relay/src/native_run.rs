//! Scaffolding shared by every relay native turn (Telegram, Discord and watch
//! firings): config, session settings, the registry row and the run hooks.

use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hq_agent::native_hq::{
    DetachedTurnOutcome, DetachedTurnSink, NativeHqHooks, NativeHqResult, SteerInbox,
};
use hq_core::config::HqConfig;
use hq_core::identity::RequestIdentity;
use hq_core::types::{ChatMessage, ImageAttachment, MessageRole, PermissionPreset};
use hq_db::Database;
use tokio::sync::Mutex as TokioMutex;

use crate::relay_common::{ChannelState, ChatSender, close_turn_row, finish_detached};

const DEFAULT_PROGRESS_SECS: u64 = 300;

/// A chat key a bridge indexes its `ChannelState` map by (Telegram `i64`, Discord `u64`).
pub(crate) trait ChatKey:
    Copy + Eq + Hash + FromStr + Display + Send + Sync + 'static
{
}
impl<T: Copy + Eq + Hash + FromStr + Display + Send + Sync + 'static> ChatKey for T {}

pub(crate) type Threads<K> = Arc<TokioMutex<HashMap<K, ChannelState>>>;

/// The live config, or the defaults with a warning when it can't be read.
pub(crate) fn load_config(site: &str) -> HqConfig {
    HqConfig::load().unwrap_or_else(|e| {
        tracing::warn!(%e, site, "failed to load config, using defaults");
        HqConfig::default()
    })
}

/// Session settings for a relay turn, and the repo root it runs in.
pub(crate) fn relay_session(
    config: &HqConfig,
    live_user: bool,
) -> (hq_agent::session::SessionConfig, PathBuf) {
    let session = hq_agent::session::SessionConfig {
        model: hq_core::config::resolve_session_model(config),
        max_budget_usd: Some(config.budget.session_cap_usd),
        is_live_user_turn: live_user,
        ..hq_agent::session::SessionConfig::default()
    };
    let repo_root = config
        .vault_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| config.vault_path.clone());
    (session, repo_root)
}

/// What a turn reads from its chat under one lock: the stored system prompt,
/// prior history, this turn's images and the pinned permission preset.
pub(crate) struct TurnInputs {
    pub system_prompt: Option<String>,
    pub history: Vec<ChatMessage>,
    pub images: Vec<ImageAttachment>,
    pub permission_preset: Option<PermissionPreset>,
}

pub(crate) async fn turn_inputs<K: ChatKey>(threads: &Threads<K>, key: K) -> TurnInputs {
    let t = threads.lock().await;
    let state = t.get(&key);
    let messages = state.map(|s| s.messages.as_slice()).unwrap_or_default();
    let system_prompt = messages
        .first()
        .filter(|m| m.role == MessageRole::System)
        .map(|m| m.content.clone());
    let history = messages
        .iter()
        .filter(|m| m.role != MessageRole::System)
        .cloned()
        .collect();
    // The last history entry is this turn's own message; the images ride on
    // the triggering prompt instead of a history copy (FR-017).
    let (history, images) = crate::relay_common::split_current_turn_images(history);
    TurnInputs {
        system_prompt,
        history,
        images,
        permission_preset: state.and_then(|s| s.pinned_permission_preset),
    }
}

/// A turn's `background_turns` row. Registered before the session is built so
/// child completions can attach to it and a detached turn survives restarts.
/// Best-effort: the turn runs untracked when vault.db is unavailable.
pub(crate) struct TurnRow {
    pub db: Option<Arc<Database>>,
    pub id: String,
    tracked: Option<String>,
    /// The chat this turn answers, which owner approvals are kept under.
    chat: String,
}

impl TurnRow {
    pub fn register(
        config: &HqConfig,
        platform: &str,
        chat_id: &str,
        identity: Option<&str>,
        prompt: &str,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let db = Database::open(&config.db_path()).ok().map(Arc::new);
        let tracked = crate::relay_common::register_turn(
            db.as_deref(),
            &id,
            platform,
            chat_id,
            identity,
            prompt,
        );
        Self {
            db,
            id,
            tracked,
            chat: format!("{platform}:{chat_id}"),
        }
    }

    /// Hooks every relay turn sets: ack timeout, identity, registry id and progress.
    pub fn hooks(
        &self,
        config: &HqConfig,
        identity: RequestIdentity,
        send: ChatSender,
        max_chars: usize,
    ) -> NativeHqHooks {
        let progress = crate::relay_common::progress_sink(
            self.db.clone(),
            config.vault_path.clone(),
            identity.clone(),
            max_chars,
            send,
        );
        NativeHqHooks {
            timeout: Some(Duration::from_secs(config.relay.turn_ack_timeout_secs)),
            identity: Some(identity),
            turn_id: self.tracked.clone(),
            on_progress: Some(progress),
            progress_interval_secs: Some(
                config
                    .relay
                    .background_progress_secs
                    .unwrap_or(DEFAULT_PROGRESS_SECS),
            ),
            owner_approval: self.owner_approval(config),
            ..Default::default()
        }
    }

    /// A chat turn asks the owner before calls that need them, through the inbox in vault.db.
    fn owner_approval(
        &self,
        config: &HqConfig,
    ) -> Option<Arc<hq_agent::governance::OwnerApproval>> {
        if !config.governance.asks_owner() {
            return None;
        }
        let Some(db) = self.db.clone() else {
            tracing::warn!("vault.db unavailable: this relay turn cannot ask the owner");
            return None;
        };
        Some(hq_agent::governance::OwnerApproval::new(
            db,
            format!("relay:{}", self.chat),
        ))
    }

    /// Attach each finished sub-agent to this row and post its outcome.
    /// `run_native_hq` records the thread entry itself.
    pub fn child_sink(
        &self,
        send: ChatSender,
        max_chars: usize,
    ) -> hq_agent::agents::CompletionSink {
        let (db, turn_id) = (self.db.clone(), self.id.clone());
        Arc::new(move |event: hq_agent::agents::ChildCompletionEvent| {
            if let Some(db) = &db
                && let Err(e) = db.with_conn(|c| {
                    hq_db::background_turns::attach_child_session(c, &turn_id, &event.task_id)
                })
            {
                tracing::warn!(%e, "attach_child_session failed");
            }
            let status = if event.success { "finished" } else { "failed" };
            let verdict = match event.accept_status.as_deref() {
                Some(a @ ("partial" | "blocked")) => format!(", {a}: not a finished deliverable"),
                _ => String::new(),
            };
            let run = event
                .run_id
                .as_deref()
                .map_or(String::new(), |r| format!(" run `{r}`"));
            let text = format!(
                "Sub-agent `{}` ({}) {status}{verdict}{run}:\n{}",
                event.task_id, event.role, event.summary
            );
            send(hq_core::text::truncate_chars_ellipsis(&text, max_chars));
        })
    }

    /// Close the row for a run that did not detach; detached rows are closed
    /// by their sink, or every quick turn would look stranded to the reconciler.
    pub fn close(&self, run: &anyhow::Result<NativeHqResult>) {
        match run {
            Ok(r) if r.detached => {}
            Ok(r) => close_turn_row(self.db.as_deref(), &self.id, &r.text, r.success),
            Err(e) => {
                tracing::warn!(%e, "relay turn: failed to build session");
                close_turn_row(self.db.as_deref(), &self.id, "session build error", false);
            }
        }
    }
}

/// The fold settings for a relay chat. A relay keeps its own recent history (between
/// `KEPT_HISTORY` and `MAX_HISTORY` messages, see `ChannelState::stage_turn`), so its mirrored
/// thread folds down to half of `KEPT_HISTORY`: everything older than the channel's own window
/// is then always in a block, and nothing falls between the two.
pub(crate) fn relay_fold_config(config: &HqConfig) -> hq_core::config::ContextFoldConfig {
    hq_core::config::ContextFoldConfig {
        tail_messages: crate::relay_common::KEPT_HISTORY / 2,
        ..config.context_fold.clone()
    }
}

/// The fold blocks of a relay chat's mirrored thread, to go in front of the channel's own
/// history. Empty when folding is off, the chat was never mirrored or the database is missing.
pub(crate) fn fold_blocks(
    db: Option<&Database>,
    config: &HqConfig,
    platform: &str,
    external_id: &str,
) -> Vec<ChatMessage> {
    if !config.context_fold.enabled {
        return Vec::new();
    }
    let Some(db) = db else { return Vec::new() };
    let blocks = db.with_conn(|conn| {
        let Some(thread) = hq_db::chat::find_platform_thread(conn, platform, external_id)? else {
            return Ok(Vec::new());
        };
        hq_db::thread_folds::active(conn, &thread)
    });
    match blocks {
        Ok(blocks) => hq_agent::fold::block_messages(&blocks),
        Err(e) => {
            tracing::debug!(
                platform,
                external_id,
                "no fold blocks for relay chat: {e:#}"
            );
            Vec::new()
        }
    }
}

/// After a relay reply: fold the mirrored thread's older messages in the background.
pub(crate) fn fold_after_turn(
    db: Option<&Arc<Database>>,
    config: &HqConfig,
    platform: &str,
    external_id: &str,
) {
    let cfg = relay_fold_config(config);
    let Some(db) = db.filter(|_| cfg.enabled) else {
        return;
    };
    let thread =
        db.with_conn(|conn| hq_db::chat::find_platform_thread(conn, platform, external_id));
    if let Ok(Some(thread)) = thread {
        hq_agent::fold::spawn_fold((**db).clone(), thread, cfg);
    }
}

/// Where a chat turn's reply is mirrored for the web UI.
pub(crate) struct Mirror {
    pub platform: &'static str,
    pub thread_sync: crate::thread_sync::ThreadSync,
    pub title: String,
}

/// Deliver a detached turn's result, then make it the turn's answer where the
/// "Parked" ack was recorded: the web UI mirror and the replayed chat history.
pub(crate) fn detached_sink<K: ChatKey>(
    threads: &Threads<K>,
    key: K,
    state_file: String,
    row: &TurnRow,
    send: ChatSender,
    mirror: Mirror,
    vault_path: PathBuf,
) -> DetachedTurnSink {
    let (threads, db) = (threads.clone(), row.db.clone());
    let state_file = Arc::new(state_file);
    Arc::new(move |outcome: DetachedTurnOutcome| {
        finish_detached(db.as_deref(), &send, &outcome);
        mirror.thread_sync.record(
            mirror.platform,
            &key.to_string(),
            &mirror.title,
            "assistant",
            &outcome.text,
        );
        let (threads, vault_path, state_file) =
            (threads.clone(), vault_path.clone(), state_file.clone());
        tokio::spawn(async move {
            if let Some(state) = threads.lock().await.get_mut(&key) {
                state.replace_parked_reply(&outcome.turn_id, &outcome.text);
                state.save(&vault_path, &state_file);
            }
        });
    })
}

/// Store the session's cancel flag so `/cancel` can reach it. Spawned because
/// the runner's hook is sync while the store is async.
pub(crate) fn cancel_hook<K: ChatKey>(
    threads: &Threads<K>,
    key: K,
) -> Box<dyn FnOnce(Arc<AtomicBool>) + Send> {
    let threads = threads.clone();
    Box::new(move |flag| {
        tokio::spawn(async move {
            if let Some(state) = threads.lock().await.get_mut(&key) {
                state.active_cancel = Some(flag);
            }
        });
    })
}

/// Store the session's steer inbox, so a message that arrives mid-turn
/// redirects it instead of starting a second turn.
pub(crate) fn steer_hook<K: ChatKey>(
    threads: &Threads<K>,
    key: K,
) -> Box<dyn FnOnce(SteerInbox) + Send> {
    let threads = threads.clone();
    Box::new(move |inbox| {
        tokio::spawn(async move {
            if let Some(state) = threads.lock().await.get_mut(&key) {
                state.pending_steer = Some(inbox);
            }
        });
    })
}

/// Forget the cancel handle once the session is done.
pub(crate) async fn clear_cancel<K: ChatKey>(threads: &Threads<K>, key: K) {
    if let Some(state) = threads.lock().await.get_mut(&key) {
        state.active_cancel = None;
    }
}

/// Aborts a ticker task even when the run panics.
pub(crate) struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod fold_tests {
    use super::*;

    #[test]
    fn a_relay_chat_gets_its_mirrored_threads_blocks_and_folds_inside_its_own_window() {
        let config = HqConfig::default();
        let cfg = relay_fold_config(&config);
        // Folded down to half of what the channel keeps, so its window always overlaps a block.
        assert!(cfg.target_messages() * 2 <= crate::relay_common::KEPT_HISTORY);

        let db = Database::open_memory().unwrap();
        assert!(fold_blocks(Some(&db), &config, "telegram", "42").is_empty());
        db.with_conn(|c| {
            let t = hq_db::chat::find_or_create_platform_thread(c, "telegram", "42", "chat")?;
            let a = hq_db::chat::add_message(c, &t.thread_id, "user", "first")?;
            let b = hq_db::chat::add_message(c, &t.thread_id, "assistant", "second")?;
            hq_db::thread_folds::insert(
                c,
                &hq_db::thread_folds::NewFold {
                    thread_id: &t.thread_id,
                    tier: 1,
                    first_msg_id: &a.message_id,
                    last_msg_id: &b.message_id,
                    message_count: 2,
                    summary: "Goal as of this block: the rent",
                    source_tokens: 4,
                    summary_tokens: 8,
                    tainted: false,
                    model: "test",
                },
            )
        })
        .unwrap();
        let blocks = fold_blocks(Some(&db), &config, "telegram", "42");
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].content.contains("the rent"));
        assert!(fold_blocks(Some(&db), &config, "discord", "42").is_empty());

        let mut off = config.clone();
        off.context_fold.enabled = false;
        assert!(fold_blocks(Some(&db), &off, "telegram", "42").is_empty());
    }
}
