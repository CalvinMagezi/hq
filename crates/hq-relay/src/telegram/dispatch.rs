//! Native HQ dispatch for one Telegram message: live activity feed, detach, progress.

use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::MessageId;
use tokio::sync::Mutex as TokioMutex;

use super::access::TelegramRole;
use super::caller_context::CallerContext;
use crate::heartbeat::SharedActivityFeed;
use crate::native_run::{AbortOnDrop, Mirror, TurnRow};
use crate::relay_common::{ChannelState, split_message};

/// Telegram's per-message character limit.
pub(super) const TG_MESSAGE_CHARS: usize = 4096;
/// Progress notes are cut shorter than a full message so they stay glanceable.
pub(super) const TG_PROGRESS_CHARS: usize = 3500;
/// The live feed edit loop wakes this often, and edits at most this often
/// (Telegram's edit rate limit).
const FEED_POLL: std::time::Duration = std::time::Duration::from_secs(2);
const FEED_MIN_EDIT_GAP: std::time::Duration = std::time::Duration::from_secs(3);

/// Send text to a chat, split at Telegram's message limit.
pub(super) fn tg_sender(
    bot: Bot,
    chat: teloxide::types::ChatId,
) -> crate::relay_common::ChatSender {
    Arc::new(move |text: String| {
        let bot = bot.clone();
        tokio::spawn(async move {
            for chunk in split_message(&text, TG_MESSAGE_CHARS) {
                if let Err(e) = bot.send_message(chat, chunk).await {
                    tracing::warn!(error = %e, "telegram delivery failed");
                    break;
                }
            }
        });
    })
}

/// Run a prompt through the native HQ session with a live activity feed in
/// the placeholder, wiring cancel and steer into `ChannelState`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_hq(
    text: &str,
    enriched_prompt: &str,
    bot: &Bot,
    chat_id: teloxide::types::ChatId,
    placeholder_id: MessageId,
    chat_key: i64,
    threads: Arc<TokioMutex<HashMap<i64, ChannelState>>>,
    caller: Arc<CallerContext>,
    mirror: Mirror,
) -> String {
    let config = crate::native_run::load_config("telegram dispatch");
    let inputs = crate::native_run::turn_inputs(&threads, chat_key).await;
    // A live person is driving this turn; a guest is further restricted by
    // tool_policy's category deny-list, not by this flag.
    let (mut session, repo_root) = crate::native_run::relay_session(&config, true);
    if caller.role == TelegramRole::Guest {
        session.agent_name = "telegram_guest".to_string();
    }
    let instructions = match inputs.system_prompt.as_deref() {
        Some(sp) => crate::session_runner::strip_loaded_soul(sp, &config.vault_path),
        None => enriched_prompt.to_string(),
    };

    let feed = crate::heartbeat::new_activity_feed();
    // Set from a real BackendEvent::Failover (CostUpdate.is_fallback) to the
    // model that actually served this turn.
    let fallback_seen = Arc::new(std::sync::Mutex::new(None::<String>));
    let _ticker = AbortOnDrop(spawn_feed_ticker(
        bot.clone(),
        chat_id,
        placeholder_id,
        feed.clone(),
    ));

    let send = tg_sender(bot.clone(), chat_id);
    let row = TurnRow::register(
        &config,
        "telegram",
        &chat_id.0.to_string(),
        Some(&caller.identity.user_id),
        text,
    );
    let on_detached = crate::native_run::detached_sink(
        &threads,
        chat_key,
        format!("tg-{chat_key}"),
        &row,
        send.clone(),
        mirror,
        config.vault_path.clone(),
    );
    let run = hq_agent::native_hq::run_native_hq(
        &config,
        text,
        instructions,
        repo_root,
        session,
        hq_agent::native_hq::NativeHqHooks {
            history: inputs.history,
            image_parts: inputs.images,
            on_event: Some(feed_hook(feed, fallback_seen.clone())),
            on_cancel: Some(crate::native_run::cancel_hook(&threads, chat_key)),
            on_steer: Some(crate::native_run::steer_hook(&threads, chat_key)),
            on_detached: Some(on_detached),
            on_child_completion: Some(row.child_sink(send.clone(), TG_PROGRESS_CHARS)),
            permission_preset: inputs.permission_preset,
            ..row.hooks(&config, caller.identity.clone(), send, TG_PROGRESS_CHARS)
        },
    )
    .await;
    row.close(&run);
    drop(_ticker);
    crate::native_run::clear_cancel(&threads, chat_key).await;

    let result_text = match run {
        Ok(r) => r.text,
        Err(e) => format!("(HQ session error: {e})"),
    };
    let served_by = fallback_seen.lock().ok().and_then(|m| m.clone());
    match served_by {
        Some(served_by) if config.backends.is_configured() => {
            flag_fallback(&config, result_text, &served_by)
        }
        _ => result_text,
    }
}

/// The primary was unavailable and a fallback served the reply. A standalone
/// interrupting notice covers turns with no rendered reply (detached or watch),
/// and the reply itself is annotated so it does not look like a normal turn.
fn flag_fallback(
    config: &hq_core::config::HqConfig,
    result_text: String,
    served_by: &str,
) -> String {
    let primary = &config.backends.primary;
    let notice = format!(
        "Provider fallback: `{primary}` was unavailable this turn, served by `{served_by}` instead. See daemon logs for the error."
    );
    let mut msg = hq_core::mailbox::new_message(
        "provider-chain",
        "relay",
        hq_core::types::MailboxMessageType::Nudge,
        Some("Provider fallback"),
        &notice,
        None,
    );
    msg.meta.insert(
        hq_core::mailbox::META_INTERRUPT.to_string(),
        "true".to_string(),
    );
    if let Err(e) = hq_core::mailbox::send_message(&config.vault_path, &msg) {
        tracing::warn!(%e, "dispatch_hq: fallback notification failed");
    }
    format!(
        "{result_text}\n\n_(served by `{served_by}` — primary `{primary}` was unavailable this turn; see daemon logs)_"
    )
}

/// Session events into the rolling activity feed, noting any provider fallback.
fn feed_hook(
    feed: SharedActivityFeed,
    fallback_seen: Arc<std::sync::Mutex<Option<String>>>,
) -> Box<dyn Fn(hq_core::types::SessionEvent) + Send + Sync + 'static> {
    use hq_core::types::SessionEvent;
    Box::new(move |event| {
        if let SessionEvent::CostUpdate {
            model,
            is_fallback: true,
            ..
        } = &event
            && let Ok(mut m) = fallback_seen.lock()
        {
            *m = Some(model.clone());
        }
        let Ok(mut f) = feed.lock() else {
            return;
        };
        match &event {
            SessionEvent::TurnEnd { turn } => f.on_turn(*turn),
            SessionEvent::ToolStart { tool_name, .. } => f.on_tool_start(tool_name),
            SessionEvent::ToolProgress {
                tool_name, message, ..
            } => f.on_tool_progress(tool_name, message),
            SessionEvent::ToolEnd {
                tool_name, result, ..
            } => f.on_tool_end(tool_name, result),
            SessionEvent::Reasoning(delta) => f.on_reasoning(delta),
            SessionEvent::SubagentCompleted {
                agent_type,
                harness,
                result_preview,
            } => f.on_subagent(agent_type, harness, result_preview),
            SessionEvent::RetryAttempt {
                attempt,
                max_retries,
                error,
                ..
            } => f.on_retry(*attempt, *max_retries, error),
            SessionEvent::Compaction {
                old_messages,
                new_messages,
            } => f.on_compaction(*old_messages, *new_messages),
            SessionEvent::CostUpdate {
                total_usd,
                input_tokens,
                output_tokens,
                model,
                ..
            } => f.on_cost(*total_usd, *input_tokens, *output_tokens, model),
            SessionEvent::Error(msg) => f.on_error(msg),
            _ => {}
        }
    })
}

/// Throttled live editor: edits only when the feed changed, no more often
/// than `FEED_MIN_EDIT_GAP`, and never with unchanged text (Telegram rejects it).
fn spawn_feed_ticker(
    bot: Bot,
    chat: teloxide::types::ChatId,
    placeholder: MessageId,
    feed: SharedActivityFeed,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let mut tick = tokio::time::interval(FEED_POLL);
        let mut last_edit: Option<std::time::Instant> = None;
        let mut last_text = String::new();
        tick.tick().await;
        loop {
            tick.tick().await;
            if last_edit.is_some_and(|t| t.elapsed() < FEED_MIN_EDIT_GAP) {
                continue;
            }
            let Some(text) = take_render(&feed, start.elapsed().as_secs()) else {
                continue;
            };
            if text == last_text {
                continue;
            }
            if bot
                .edit_message_text(chat, placeholder, &text)
                .await
                .is_ok()
            {
                last_text = text;
                last_edit = Some(std::time::Instant::now());
            }
        }
    })
}

/// The feed's new rendering, or `None` when nothing changed or it is blank.
fn take_render(feed: &SharedActivityFeed, elapsed_secs: u64) -> Option<String> {
    let mut f = feed.lock().unwrap();
    if !f.dirty {
        return None;
    }
    f.dirty = false;
    let rendered = f.render(elapsed_secs);
    (!rendered.trim().is_empty()).then_some(rendered)
}
