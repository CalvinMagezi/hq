//! Telegram relay session: connection setup and per-message routing.
//!
//! `run_telegram_relay` is the public entry point. It starts the mailbox poller and
//! the watch scheduler, then runs the teloxide dispatcher, which routes each message
//! through authorization, media handling, commands, token replies and finally a
//! native HQ turn.

use anyhow::Result;
use hq_db::Database;
use hq_vault::VaultClient;
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{ChatAction, MessageId, ReactionType as TgReactionType, ReplyParameters};
use tokio::sync::Mutex as TokioMutex;
use tracing::info;

use super::access::{
    ChatGate, TelegramRole, authorize_chat, guest_thread_is_new, mark_guest_intro_sent,
    resolve_caller_context,
};
use super::caller_context::{CallerContext, build_caller_block, guest_intro_line};
use super::commands::{
    CommandContext, handle_backend, handle_cancel, handle_focus, handle_help, handle_model,
    handle_permission_pin, handle_reset, handle_status, normalize_command, register_hq_commands,
};
use super::dispatch::{TG_MESSAGE_CHARS, TG_PROGRESS_CHARS, dispatch_hq, tg_sender};
use super::mailbox::run_mailbox_poller;
use super::media::{MediaResult, handle_media, merge_album};
use crate::chat_commands::CommandReply;
use crate::relay_common::{ChannelState, split_message, write_relay_status};

use crate::relay_common::load_system_prompt_with_env;

// Telegram sends each file of an album as its own update, usually within a second.
// ponytail: a file arriving after this window becomes its own turn; raise it if that shows up.
const ALBUM_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1500);
const TYPING_REFRESH: std::time::Duration = std::time::Duration::from_secs(4);
const PLACEHOLDER_TEXT: &str = "Thinking\u{2026} \u{258D}";
const SKILL_HINT_BUDGET: usize = 2000;

type Threads = Arc<TokioMutex<HashMap<i64, ChannelState>>>;

/// A short human-readable label for a Telegram chat, used as the Web UI
/// thread title. Prefers group/channel title, then username, then first name,
/// falling back to the raw chat id.
fn telegram_thread_title(chat: &teloxide::types::Chat) -> String {
    if let Some(title) = chat.title() {
        return format!("Telegram: {title}");
    }
    if let Some(username) = chat.username() {
        return format!("Telegram: @{username}");
    }
    if let Some(first_name) = chat.first_name() {
        return format!("Telegram: {first_name}");
    }
    format!("Telegram: {}", chat.id.0)
}

/// Everything the message handler shares across updates, behind one `Arc`.
struct TgRelay {
    threads: Threads,
    albums: TokioMutex<HashMap<String, Vec<MediaResult>>>,
    system_prompt: Arc<String>,
    skill_index: hq_tools::skills::SkillHintIndex,
    vault_path: PathBuf,
    bot_token: String,
    db: Arc<Database>,
    hq_cfg: hq_core::config::HqConfig,
    thread_sync: crate::thread_sync::ThreadSync,
}

pub async fn run_telegram_relay(
    token: &str,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
    notifications_token: Option<String>,
    thread_events: Option<tokio::sync::broadcast::Sender<String>>,
) -> Result<()> {
    info!("telegram: starting bot...");
    let system_prompt = Arc::new(load_system_prompt_with_env(&vault));
    info!(
        prompt_len = system_prompt.len(),
        "telegram: loaded system prompt"
    );

    let vault_path = vault.vault_path().to_path_buf();
    let bot = Bot::new(token);
    // Best-effort: stale autocomplete must not block startup.
    if let Err(e) = register_hq_commands(&bot).await {
        tracing::warn!(error = %e, "telegram: setMyCommands failed (autocomplete may be stale)");
    } else {
        tracing::info!("telegram: registered HQ slash-command autocomplete");
    }

    let restored = ChannelState::restore_all::<i64>(&vault_path, "tg-");
    if !restored.is_empty() {
        info!(count = restored.len(), "telegram: restored channel states");
    }
    let threads: Threads = Arc::new(TokioMutex::new(restored));

    let _ = std::fs::create_dir_all(vault_path.join("_mailboxes/relay"));
    // The outreach bot delivers notifications when configured.
    let notif_token = notifications_token.unwrap_or_else(|| token.to_string());
    tokio::spawn(run_mailbox_poller(
        Arc::new(vault_path.clone()),
        Arc::new(notif_token),
    ));

    write_relay_status(
        &vault_path,
        "telegram",
        "connected",
        Some("bot online"),
        None,
    );
    info!("telegram: relay status written — bot is online");

    spawn_watch_scheduler(&bot, db.clone(), threads.clone(), system_prompt.clone());

    let relay = Arc::new(TgRelay {
        threads,
        albums: TokioMutex::default(),
        system_prompt,
        skill_index: hq_tools::skills::SkillHintIndex::build(&hq_core::skills_dir(&vault_path)),
        vault_path,
        bot_token: token.to_string(),
        thread_sync: crate::thread_sync::ThreadSync::new(db.clone(), thread_events),
        db,
        hq_cfg: hq_core::config::HqConfig::load().unwrap_or_default(),
    });
    let message_handler = move |bot: Bot, msg: teloxide::types::Message| {
        let relay = relay.clone();
        async move {
            relay.handle(bot, msg).await;
            respond(())
        }
    };
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(message_handler))
        .branch(
            Update::filter_callback_query().endpoint(super::callbacks::value_bus_callback_handler),
        );

    // No ctrl-c handler: the relay runs as a task inside the daemon, which owns
    // process lifecycle; installing a second SIGINT handler here can panic at startup.
    // No grouping: every update runs concurrently instead of queueing behind
    // whatever's already running for that chat. Per-chat single-flight is
    // enforced by hand via `ChannelState.turn_in_flight` (see `TgRelay::handle`)
    // so /cancel and a plain resteer message reach the handler immediately
    // instead of waiting for the in-flight turn's task to return on its own.
    Dispatcher::builder(bot, handler)
        .distribution_function(|_| None::<()>)
        .build()
        .dispatch()
        .await;

    Ok(())
}

/// Re-dispatches due recurring watch turns through the shared native-hq path, so
/// each firing detaches and delivers like a normal turn. Due watches fire on the
/// first poll after startup.
fn spawn_watch_scheduler(
    bot: &Bot,
    db: Arc<Database>,
    threads: Threads,
    system_prompt: Arc<String>,
) {
    let sender_bot = bot.clone();
    let surface = crate::watch_scheduler::WatchSurface {
        platform: "telegram",
        threads,
        system_prompt,
        progress_chars: TG_PROGRESS_CHARS,
        identity: hq_core::identity::RequestIdentity::from_telegram,
        sender: Arc::new(move |chat: i64| {
            tg_sender(sender_bot.clone(), teloxide::types::ChatId(chat))
        }),
    };
    tokio::spawn(crate::subagent_followup::run_subagent_followups(
        surface.clone(),
        db.clone(),
    ));
    let (dispatch, notify) = crate::watch_scheduler::watch_callbacks(surface);
    tokio::spawn(crate::watch_scheduler::run_watch_scheduler(
        db,
        "telegram",
        std::time::Duration::from_secs(crate::watch_scheduler::DEFAULT_POLL_SECS),
        dispatch,
        notify,
    ));
    info!("telegram: watch scheduler spawned");
}

impl TgRelay {
    async fn handle(&self, bot: Bot, msg: teloxide::types::Message) {
        let chat_key = msg.chat.id.0;
        match authorize_chat(
            &self.vault_path,
            &self.hq_cfg.relay,
            chat_key,
            msg.chat.is_private(),
            msg.text().unwrap_or_default(),
        ) {
            ChatGate::Proceed => {}
            ChatGate::Drop => return,
            ChatGate::Paired => {
                let _ = bot
                    .send_message(msg.chat.id, "Paired. This chat is now the owner.")
                    .await;
                return;
            }
        }
        let Some(media) = self.collect_inbound(&bot, &msg).await else {
            return;
        };
        if media.text.is_empty() {
            tracing::info!(chat_id = chat_key, "telegram: empty message, skipping");
            return;
        }
        let raw_text = media.text;
        let text = match msg.reply_to_message() {
            Some(reply) => match reply.text() {
                Some(quoted) => {
                    let who = reply
                        .from
                        .as_ref()
                        .map(|u| if u.is_bot { "assistant" } else { &u.first_name })
                        .unwrap_or("someone");
                    crate::relay_common::quote_reply(who, quoted, &raw_text)
                }
                None => raw_text.clone(),
            },
            None => raw_text.clone(),
        };

        let Some(caller) = resolve_caller_context(chat_key, &self.vault_path, &self.hq_cfg) else {
            tracing::warn!(
                chat_id = chat_key,
                "telegram: could not resolve caller context"
            );
            return;
        };
        let caller = Arc::new(caller);
        self.announce_caller(&bot, &msg, &caller).await;

        let text_clean = normalize_command(&text);
        let text_lower = text_clean.to_lowercase();
        if self
            .reply_if_busy(&bot, &msg, &text_clean, &text_lower)
            .await
        {
            return;
        }
        let cmd_ctx = CommandContext {
            bot: &bot,
            msg: &msg,
            chat_key,
            text_clean: &text_clean,
            text_lower: &text_lower,
            threads: self.threads.clone(),
            caller: &caller,
        };
        if run_commands(&cmd_ctx).await {
            return;
        }
        // A resume or first watch firing replaces the command text for the rest of the turn.
        let text = match self
            .registry_step(&bot, &msg, &caller, &text_lower, &text_clean)
            .await
        {
            ControlFlow::Break(()) => return,
            ControlFlow::Continue(prompt) => prompt.unwrap_or(text),
        };
        if self.token_reply(&bot, &msg, &caller, &raw_text).await {
            return;
        }
        self.run_turn(&bot, &msg, caller, text, &text_clean, media.images)
            .await;
    }

    /// Text or caption, mirrored to the web UI, with media attached. An album's
    /// later updates only deposit their files and return `None`; the first one
    /// waits for its siblings and answers for all of them.
    async fn collect_inbound(
        &self,
        bot: &Bot,
        msg: &teloxide::types::Message,
    ) -> Option<MediaResult> {
        let raw_text = msg
            .text()
            .or_else(|| msg.caption())
            .unwrap_or_default()
            .to_string();
        tracing::info!(
            chat_id = msg.chat.id.0,
            update_id = ?msg.id,
            text_len = raw_text.len(),
            "telegram: incoming message"
        );
        if !raw_text.trim().is_empty() {
            self.thread_sync.record(
                "telegram",
                &msg.chat.id.0.to_string(),
                &telegram_thread_title(&msg.chat),
                "user",
                &raw_text,
            );
        }
        let media = handle_media(bot, msg, raw_text, &self.vault_path, &self.bot_token).await;
        let Some(group) = msg.media_group_id().map(|g| g.to_string()) else {
            return Some(media);
        };
        let first = {
            let mut pending = self.albums.lock().await;
            let parts = pending.entry(group.clone()).or_default();
            parts.push(media);
            parts.len() == 1
        };
        if !first {
            return None;
        }
        tokio::time::sleep(ALBUM_DEBOUNCE).await;
        let parts = self.albums.lock().await.remove(&group).unwrap_or_default();
        Some(merge_album(parts))
    }

    /// Log the caller, record owner presence, greet a new guest, and react 👀.
    async fn announce_caller(
        &self,
        bot: &Bot,
        msg: &teloxide::types::Message,
        caller: &CallerContext,
    ) {
        let chat_key = msg.chat.id.0;
        tracing::info!(
            chat_id = caller.caller_chat_id,
            caller_role = ?caller.role,
            caller_name = %caller.display_name,
            "telegram: caller context"
        );
        if caller.role == TelegramRole::Owner {
            let detail = format!("chat: {chat_key}");
            write_relay_status(
                &self.vault_path,
                "telegram",
                "connected",
                Some(&detail),
                None,
            );
        }
        if caller.role == TelegramRole::Guest && guest_thread_is_new(&self.vault_path, chat_key) {
            let _ = bot
                .send_message(msg.chat.id, guest_intro_line(caller))
                .await;
            mark_guest_intro_sent(&self.vault_path, chat_key);
        }
        react(bot, msg, "\u{1F440}").await;
    }

    /// While a turn runs, everything except /cancel is input for that turn, so
    /// no command (not even /reset) runs against the busy ChannelState. This is
    /// best-effort; `claim_turn` in `run_turn` is the authoritative check.
    async fn reply_if_busy(
        &self,
        bot: &Bot,
        msg: &teloxide::types::Message,
        text_clean: &str,
        text_lower: &str,
    ) -> bool {
        if text_lower == "/cancel" || text_lower == "!cancel" {
            return false;
        }
        let busy = self
            .threads
            .lock()
            .await
            .get(&msg.chat.id.0)
            .and_then(|s| s.busy_reply(text_clean));
        let Some(reply) = busy else {
            return false;
        };
        let _ = bot.send_message(msg.chat.id, reply).await;
        true
    }

    /// `resume`, `watch` and `unwatch`. `Break` when the command was answered;
    /// `Continue(Some(prompt))` when a stored prompt should run as this turn.
    async fn registry_step(
        &self,
        bot: &Bot,
        msg: &teloxide::types::Message,
        caller: &CallerContext,
        text_lower: &str,
        text_clean: &str,
    ) -> ControlFlow<(), Option<String>> {
        let reply = crate::chat_commands::registry_command(
            text_lower,
            text_clean,
            "telegram",
            msg.chat.id.0,
            Some(&caller.identity.user_id),
            || Some(self.db.clone()),
        );
        match reply {
            Some(CommandReply::Done(reply)) => {
                let _ = bot.send_message(msg.chat.id, reply).await;
                ControlFlow::Break(())
            }
            Some(CommandReply::Redispatch { ack, prompt }) => {
                let _ = bot.send_message(msg.chat.id, ack).await;
                ControlFlow::Continue(Some(prompt))
            }
            None => ControlFlow::Continue(None),
        }
    }

    /// Email send/skip and value-bus approve/dismiss replies.
    /// Each acts only when its token matches something real; true when answered.
    async fn token_reply(
        &self,
        bot: &Bot,
        msg: &teloxide::types::Message,
        caller: &CallerContext,
        raw_text: &str,
    ) -> bool {
        let is_guest = caller.role == TelegramRole::Guest;
        if let Some((action, token, revision)) = hq_daemon::parse_email_command(raw_text) {
            if is_guest {
                let reply = format!("Email approvals are for {} only.", caller.owner_name);
                let _ = bot.send_message(msg.chat.id, reply).await;
                return true;
            }
            let resolved =
                hq_daemon::resolve_email_action(&self.vault_path, &token, action, revision).await;
            if let Some(reply) = resolved {
                let _ = bot.send_message(msg.chat.id, reply).await;
                return true;
            }
        }
        if !is_guest && let Some((action, token)) = hq_daemon::parse_value_command(raw_text) {
            match hq_daemon::record_engagement(&self.vault_path, &token, action) {
                Ok(true) => {
                    let _ = bot.send_message(msg.chat.id, "Recorded.").await;
                    return true;
                }
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "value-bus: record_engagement failed"),
            }
        }
        false
    }

    /// Claim the chat, run one native HQ turn, release the chat and deliver.
    async fn run_turn(
        &self,
        bot: &Bot,
        msg: &teloxide::types::Message,
        caller: Arc<CallerContext>,
        text: String,
        text_clean: &str,
        images: Vec<hq_core::types::ImageAttachment>,
    ) {
        let chat_key = msg.chat.id.0;
        let typing = spawn_typing(bot.clone(), msg.chat.id);
        let placeholder_id = match bot.send_message(msg.chat.id, PLACEHOLDER_TEXT).await {
            Ok(m) => m.id,
            Err(e) => {
                tracing::error!("telegram: failed to send placeholder: {e}");
                typing.abort();
                return;
            }
        };
        let (base_enriched, _matched_skills) = hq_tools::skills::enrich_system_prompt(
            &self.skill_index,
            &self.system_prompt,
            &text,
            None,
            Some(SKILL_HINT_BUDGET),
        );
        let enriched_prompt = format!("{}\n\n{base_enriched}", build_caller_block(&caller));

        let claimed = {
            let mut t = self.threads.lock().await;
            let state = t.entry(chat_key).or_insert_with(ChannelState::new_default);
            let claimed = state.claim_turn(text_clean);
            if claimed.is_ok() {
                state.stage_turn(&enriched_prompt, &text, images);
            }
            claimed
        };
        if let Err(reply) = claimed {
            typing.abort();
            let _ = bot.delete_message(msg.chat.id, placeholder_id).await;
            let _ = bot.send_message(msg.chat.id, reply).await;
            return;
        }

        // Whitespace-only text can survive media handling; there is no task to run.
        let result = if text.trim().is_empty() {
            Err(anyhow::anyhow!("empty task description"))
        } else {
            let mirror = crate::native_run::Mirror {
                platform: "telegram",
                thread_sync: self.thread_sync.clone(),
                title: telegram_thread_title(&msg.chat),
            };
            let threads = self.threads.clone();
            let reply = dispatch_hq(
                &text,
                &enriched_prompt,
                bot,
                msg.chat.id,
                placeholder_id,
                chat_key,
                threads,
                caller,
                mirror,
            );
            Ok(reply.await)
        };
        typing.abort();
        self.release_turn(chat_key, &result).await;
        self.deliver_reply(bot, msg, placeholder_id, result).await;
    }

    /// Record the outcome (failures too, see `record_turn_outcome`), free the
    /// single-flight slot and persist. Not panic-safe, like `active_cancel`'s
    /// clear in `dispatch_hq`.
    async fn release_turn(&self, chat_key: i64, result: &Result<String>) {
        let mut t = self.threads.lock().await;
        if let Some(state) = t.get_mut(&chat_key) {
            state.record_turn_outcome(result);
            state.turn_in_flight = false;
            state.pending_steer = None;
            state.save(&self.vault_path, &format!("tg-{chat_key}"));
        }
    }

    async fn deliver_reply(
        &self,
        bot: &Bot,
        msg: &teloxide::types::Message,
        placeholder_id: MessageId,
        result: Result<String>,
    ) {
        let reply = match result {
            Ok(reply) if reply.is_empty() => {
                let _ = bot
                    .edit_message_text(msg.chat.id, placeholder_id, "No response received.")
                    .await;
                return;
            }
            Ok(reply) => reply,
            Err(e) => {
                tracing::error!(error = %e, "hq harness error");
                let _ = bot
                    .edit_message_text(msg.chat.id, placeholder_id, format!("Error (hq): {e}"))
                    .await;
                return;
            }
        };
        let _ = bot.delete_message(msg.chat.id, placeholder_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.thread_sync.record(
            "telegram",
            &msg.chat.id.0.to_string(),
            &telegram_thread_title(&msg.chat),
            "assistant",
            &reply,
        );
        for (i, chunk) in split_message(&reply, TG_MESSAGE_CHARS).iter().enumerate() {
            let mut req = bot.send_message(msg.chat.id, chunk.as_str());
            if i == 0 {
                req = req.reply_parameters(ReplyParameters::new(msg.id));
            }
            if let Err(e) = req.await {
                tracing::error!("telegram send error: {e}");
            }
        }
        react(bot, msg, "\u{2705}").await;
    }
}

/// Built-in slash commands, in order. True when one of them consumed the message.
async fn run_commands(ctx: &CommandContext<'_>) -> bool {
    handle_reset(ctx).await.unwrap_or(false)
        || handle_cancel(ctx).await.unwrap_or(false)
        || handle_focus(ctx).await.unwrap_or(false)
        || handle_model(ctx).await.unwrap_or(false)
        || handle_backend(ctx).await.unwrap_or(false)
        || handle_permission_pin(ctx).await.unwrap_or(false)
        || handle_help(ctx).await.unwrap_or(false)
        || handle_status(ctx).await.unwrap_or(false)
}

async fn react(bot: &Bot, msg: &teloxide::types::Message, emoji: &str) {
    let _ = bot
        .set_message_reaction(msg.chat.id, msg.id)
        .reaction(vec![TgReactionType::Emoji {
            emoji: emoji.to_string(),
        }])
        .await;
}

fn spawn_typing(bot: Bot, chat: teloxide::types::ChatId) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let _ = bot.send_chat_action(chat, ChatAction::Typing).await;
            tokio::time::sleep(TYPING_REFRESH).await;
        }
    })
}

#[cfg(test)]
mod identity_tests {
    use hq_core::identity::RequestIdentity;

    #[test]
    fn telegram_identity_from_chat_id() {
        let id = RequestIdentity::from_telegram(1234567890);
        assert_eq!(id.user_id, "telegram:1234567890");
        assert_eq!(id.session_key, "hq-tg-1234567890"); // gitleaks:allow
        assert_eq!(id.source.label(), "telegram");
        assert_eq!(id.vault_paths, vec!["*"]);
    }
}
