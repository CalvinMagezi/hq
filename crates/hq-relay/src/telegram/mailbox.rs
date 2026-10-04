//! Mailbox polling and proactive Telegram notification delivery.
//!
//! Reads from the relay mailbox on a 45-second tick, buffers low-priority messages
//! into digests, and delivers urgent messages immediately.
//! When no active Telegram session exists, urgent messages are persisted to disk
//! and replayed on the next tick that has an active chat_id.

use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup};


use super::access::get_notification_chat_id;

// ─── Value-bus nudge approval (inline keyboard, not reply-keyboard pills) ──
//
// Email-triage send/skip pills still use a reply keyboard whose text goes
// through parse_email_command. Value-bus nudges and keep/mute use inline
// keyboards, answered in callbacks.rs.

const VALUE_APPROVE_PREFIX: &str = "value_approve_";
const VALUE_DISMISS_PREFIX: &str = "value_dismiss_";
const NOTIF_KEEP_PREFIX: &str = "notif_keep_";
const NOTIF_MUTE_PREFIX: &str = "notif_mute_";

/// Build a Telegram inline-keyboard callback_data string for a value-bus
/// approve action. Mirrors `discord.rs::make_value_approve_button_id`.
pub fn make_value_approve_callback_data(token: &str) -> String {
    format!("{VALUE_APPROVE_PREFIX}{token}")
}

/// Build a Telegram inline-keyboard callback_data string for a value-bus
/// dismiss action. Mirrors `discord.rs::make_value_dismiss_button_id`.
pub fn make_value_dismiss_callback_data(token: &str) -> String {
    format!("{VALUE_DISMISS_PREFIX}{token}")
}

/// Parse a value-bus callback_data string back to (action, token).
/// Mirrors `discord.rs::parse_value_button_id`.
pub fn parse_value_callback_data(data: &str) -> Option<(&'static str, &str)> {
    if let Some(id) = data.strip_prefix(VALUE_APPROVE_PREFIX) {
        Some(("approve", id))
    } else if let Some(id) = data.strip_prefix(VALUE_DISMISS_PREFIX) {
        Some(("dismiss", id))
    } else {
        None
    }
}

/// Build the inline keyboard for a value-bus nudge (replaces the old
/// reply-keyboard "approve {token}"/"dismiss {token}" pills).
pub fn build_value_action_keyboard(token: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Approve", make_value_approve_callback_data(token)),
        InlineKeyboardButton::callback("Dismiss", make_value_dismiss_callback_data(token)),
    ]])
}

/// A message to send plus the `notif.<source>.<kind>` trait it teaches, if known.
/// `trait_key` is `Some` only when the send is a single item (so reaction buttons
/// map unambiguously to one trait); multi-item digests carry `None`.
pub type Sendable = (String, Option<String>);

/// Accumulates low-priority messages and flushes them as a digest after a cooldown.
/// Urgent messages (Nudge type) bypass the buffer and are returned immediately.
pub struct DigestBuffer {
    pending: Vec<Sendable>,
    last_flushed: std::time::Instant,
    cooldown: std::time::Duration,
}

impl DigestBuffer {
    pub fn new(cooldown: std::time::Duration) -> Self {
        Self {
            pending: Vec::new(),
            last_flushed: std::time::Instant::now(),
            cooldown,
        }
    }

    /// Add a message. Returns `Some((text, trait_key))` if it should be sent now,
    /// `None` if buffered. `trait_key` is the reaction target for single-item sends.
    pub fn add(
        &mut self,
        content: String,
        trait_key: Option<String>,
        is_urgent: bool,
    ) -> Option<Sendable> {
        if is_urgent {
            return Some((content, trait_key));
        }
        self.pending.push((content, trait_key));
        if self.last_flushed.elapsed() >= self.cooldown {
            Some(self.flush())
        } else {
            None
        }
    }

    /// Check if the buffer is ready to flush (cooldown expired with pending messages).
    pub fn try_flush(&mut self) -> Option<Sendable> {
        if !self.pending.is_empty() && self.last_flushed.elapsed() >= self.cooldown {
            Some(self.flush())
        } else {
            None
        }
    }

    fn flush(&mut self) -> Sendable {
        self.last_flushed = std::time::Instant::now();
        let msgs = std::mem::take(&mut self.pending);
        if msgs.len() == 1 {
            msgs.into_iter().next().unwrap()
        } else {
            let text = format!(
                "HQ Digest ({} updates):\n\n{}",
                msgs.len(),
                msgs.iter()
                    .enumerate()
                    .map(|(i, (m, _))| format!("{}. {}", i + 1, m))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            (text, None)
        }
    }
}

/// Inner chunked send to a pre-resolved ChatId. No config I/O.
async fn do_send_chunked(bot: &Bot, chat: teloxide::types::ChatId, message: &str) {
    use teloxide::prelude::Requester;
    for chunk in crate::relay_common::split_message(message, MAILBOX_CHUNK_CHARS) {
        if let Err(e) = bot.send_message(chat, chunk).await {
            tracing::warn!(error = %e, "mailbox-poller: Telegram send failed");
        }
        tokio::time::sleep(CHUNK_GAP).await;
    }
}

/// Under Telegram's 4096 limit, leaving room for a caption or markup.
const MAILBOX_CHUNK_CHARS: usize = 4000;
const CHUNK_GAP: tokio::time::Duration = tokio::time::Duration::from_millis(100);

/// Two-button inline keyboard that teaches the gate. Attached to the
/// notification message itself (not a chat-wide reply keyboard), so it never
/// docks at the bottom of the chat outliving its own message — the bug that
/// made stale keep/mute pills from old notifications pile up under the
/// message box. Mirrors `build_value_action_keyboard`.
fn reaction_keyboard(trait_key: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("👍 keep", format!("{NOTIF_KEEP_PREFIX}{trait_key}")),
        InlineKeyboardButton::callback("👎 mute", format!("{NOTIF_MUTE_PREFIX}{trait_key}")),
    ]])
}

/// Parse a keep/mute inline-keyboard tap's callback_data back into
/// `(trait_key, gate_value)`. Kept beside `reaction_keyboard` so the
/// callback_data format and its parser stay in sync.
pub fn parse_reaction_callback_data(data: &str) -> Option<(String, &'static str)> {
    if let Some(key) = data.strip_prefix(NOTIF_KEEP_PREFIX) {
        Some((key.to_string(), "signal"))
    } else if let Some(key) = data.strip_prefix(NOTIF_MUTE_PREFIX) {
        Some((key.to_string(), "noise"))
    } else {
        None
    }
}

/// Send a gated notification to a pre-resolved chat. When `trait_key` is set and the text
/// fits in a single message, attach 👍/👎 reaction buttons. Otherwise chunk-send.
async fn send_notification_to_chat(
    bot: &Bot,
    chat: teloxide::types::ChatId,
    text: &str,
    trait_key: Option<&str>,
) {
    use teloxide::prelude::Requester;
    match trait_key {
        Some(k) if text.len() <= 4000 => {
            if let Err(e) = bot
                .send_message(chat, text)
                .reply_markup(reaction_keyboard(k))
                .await
            {
                tracing::warn!(error = %e, "mailbox-poller: reaction send failed");
            }
        }
        _ => do_send_chunked(bot, chat, text).await,
    }
}

fn pending_notifs_path(vault_path: &std::path::Path) -> std::path::PathBuf {
    vault_path.join("_gateway").join("pending_notifs.jsonl")
}

/// Pull the token out of an email nudge's `` `send <token>` `` instruction line.
/// The backtick prefix avoids matching a stray "send" inside the drafted reply.
fn extract_send_token(content: &str) -> Option<String> {
    let idx = content.find("`send ")?;
    let after = &content[idx + "`send ".len()..];
    let token: String = after
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (token.len() >= 4 && token.len() <= 16).then_some(token)
}

/// Append a formatted message to the pending notification queue.
pub fn save_pending_notif(vault_path: &std::path::Path, message: &str) {
    let path = pending_notifs_path(vault_path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let entry = serde_json::json!({ "msg": message, "ts": chrono::Utc::now().to_rfc3339() });
    let line = format!("{}\n", serde_json::to_string(&entry).unwrap_or_default());
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Read and atomically clear the pending notification queue.
/// Returns all queued formatted messages in insertion order.
pub fn drain_pending_notifs(vault_path: &std::path::Path) -> Vec<String> {
    let path = pending_notifs_path(vault_path);
    if !path.exists() {
        return Vec::new();
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    // Delete before sending so a crash between read and send doesn't re-deliver
    let _ = std::fs::remove_file(&path);
    content
        .lines()
        .filter_map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v["msg"].as_str().map(|s| s.to_string()))
        })
        .collect()
}

const POLL_EVERY: tokio::time::Duration = tokio::time::Duration::from_secs(45);
const DIGEST_COOLDOWN: tokio::time::Duration = tokio::time::Duration::from_secs(1800);

/// Background task: polls _mailboxes/relay/ every 45s and forwards messages to Telegram.
/// Urgent messages bypass the digest buffer and are sent immediately.
/// Other messages are buffered and flushed as a digest every 30 minutes.
///
/// When no active chat session is present, urgent messages are queued to disk and
/// replayed on the next tick where a chat_id is available — preventing notification loss.
pub async fn run_mailbox_poller(vault_path: Arc<std::path::PathBuf>, bot_token: Arc<String>) {
    let bot = Bot::new(bot_token.as_str());
    let mut interval = tokio::time::interval(POLL_EVERY);
    let mut digest = DigestBuffer::new(DIGEST_COOLDOWN);
    loop {
        interval.tick().await;
        // Resolved once per tick; every send in this tick goes to this chat.
        let relay = hq_core::config::HqConfig::load().map(|c| c.relay).unwrap_or_default();
        let poller = Poller {
            bot: &bot,
            vault_path: &vault_path,
            owner_chat: get_notification_chat_id(&vault_path, &relay).map(teloxide::types::ChatId),
        };
        poller.tick(&mut digest).await;
    }
}

/// One tick's view: the bot, the vault and the owner chat, if known.
struct Poller<'a> {
    bot: &'a Bot,
    vault_path: &'a std::path::Path,
    owner_chat: Option<teloxide::types::ChatId>,
}

impl Poller<'_> {
    async fn tick(&self, digest: &mut DigestBuffer) {
        // Replay what earlier ticks queued while no chat was known.
        if let Some(chat) = self.owner_chat {
            for pending_msg in drain_pending_notifs(self.vault_path) {
                do_send_chunked(self.bot, chat, &pending_msg).await;
            }
        }
        match hq_core::mailbox::receive_messages(self.vault_path, "relay") {
            Ok(messages) => {
                for msg in messages {
                    self.route_one(msg, digest).await;
                }
            }
            Err(e) => tracing::debug!(error = %e, "mailbox-poller: failed to read relay mailbox"),
        }
        // Digest flushes are non-urgent, so they drop when no chat is known.
        if let Some((to_send, react_key)) = digest.try_flush()
            && let Some(chat) = self.owner_chat
        {
            send_notification_to_chat(self.bot, chat, &to_send, react_key.as_deref()).await;
        }
    }

    async fn route_one(&self, msg: hq_core::types::MailboxMessage, digest: &mut DigestBuffer) {
        // Categories Discord owns are re-enqueued for its poller, before the
        // gate, so they are not gated and audit-logged twice.
        #[cfg(feature = "discord")]
        if crate::discord_channels::telegram_defers_to_discord(self.vault_path, &msg, chrono::Utc::now()) {
            let _ = hq_core::mailbox::send_message(self.vault_path, &msg);
            return;
        }
        // The gate drops learned noise and is the sole authority on urgency (FR-001b).
        let gate = hq_daemon::notif_gate::gate_decision(self.vault_path, &msg);
        if gate == hq_daemon::notif_gate::GateDecision::Drop {
            return;
        }
        if self.send_email_nudge(&msg).await || self.send_value_item(&msg).await {
            return;
        }
        let is_urgent = gate == hq_daemon::notif_gate::GateDecision::Urgent;
        let trait_key = hq_daemon::notif_gate::trait_key_for(&msg.content);
        let Some((to_send, react_key)) = digest.add(msg.content, trait_key, is_urgent) else {
            return;
        };
        match self.owner_chat {
            Some(chat) => {
                send_notification_to_chat(self.bot, chat, &to_send, react_key.as_deref()).await
            }
            // Urgent ones wait for the next tick with a chat; the rest drop.
            None if is_urgent => {
                save_pending_notif(self.vault_path, &to_send);
                tracing::info!("mailbox-poller: no active chat_id, queued notification for later delivery");
            }
            None => {}
        }
    }

    /// Email-triage nudges get tap-to-send/skip pills (handled by
    /// parse_email_command). The token comes from meta, else from the body.
    async fn send_email_nudge(&self, msg: &hq_core::types::MailboxMessage) -> bool {
        let is_email_nudge =
            msg.from == "email-triage" || msg.subject.as_deref() == Some("Email needs reply");
        if !is_email_nudge {
            return false;
        }
        let Some(token) = msg
            .meta
            .get("email_token")
            .cloned()
            .or_else(|| extract_send_token(&msg.content))
        else {
            return false;
        };
        let Some(chat) = self.owner_chat else {
            save_pending_notif(self.vault_path, &msg.content);
            return true;
        };
        // Persistent, not one_time: the client auto-hides one_time keyboards,
        // which forces the user to type the token.
        let keyboard = KeyboardMarkup::new(vec![vec![
            KeyboardButton::new(format!("send {token}")),
            KeyboardButton::new(format!("skip {token}")),
        ]])
        .resize_keyboard();
        if let Err(e) = self.bot.send_message(chat, &msg.content).reply_markup(keyboard).await {
            tracing::warn!(error = %e, "mailbox-poller: email-nudge send failed");
        }
        true
    }

    /// Value-bus items with a token get inline Approve/Dismiss buttons;
    /// notices go out as plain text.
    async fn send_value_item(&self, msg: &hq_core::types::MailboxMessage) -> bool {
        if msg.from != "value-bus" {
            return false;
        }
        let Some(chat) = self.owner_chat else {
            save_pending_notif(self.vault_path, &msg.content);
            return true;
        };
        let send = self.bot.send_message(chat, &msg.content);
        let sent = match msg.meta.get("value_token") {
            Some(token) => send.reply_markup(build_value_action_keyboard(token)).await,
            None => send.await,
        };
        if let Err(e) = sent {
            tracing::warn!(error = %e, "mailbox-poller: value-bus send failed");
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reaction_keyboard_round_trips_through_callback_data() {
        use teloxide::types::InlineKeyboardButtonKind;

        let kb = reaction_keyboard("notif.github.ci.failed");
        let row = &kb.inline_keyboard[0];
        fn data(btn: &InlineKeyboardButton) -> &str {
            match &btn.kind {
                InlineKeyboardButtonKind::CallbackData(d) => d.as_str(),
                _ => panic!("expected CallbackData"),
            }
        }
        assert_eq!(
            parse_reaction_callback_data(data(&row[0])),
            Some(("notif.github.ci.failed".to_string(), "signal"))
        );
        assert_eq!(
            parse_reaction_callback_data(data(&row[1])),
            Some(("notif.github.ci.failed".to_string(), "noise"))
        );
        assert!(parse_reaction_callback_data("value_approve_abc").is_none());
    }

    #[test]
    fn urgent_send_carries_trait_key() {
        let mut b = DigestBuffer::new(Duration::from_secs(1800));
        let out = b.add("red".into(), Some("notif.github.ci.failed".into()), true);
        assert_eq!(
            out,
            Some((
                "red".to_string(),
                Some("notif.github.ci.failed".to_string())
            ))
        );
    }

    #[test]
    fn notification_chat_id_uses_owner_not_presence_guest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();
        std::fs::create_dir_all(vault.join("_system")).unwrap();
        std::fs::write(vault.join("_system/.telegram-auth-chat"), "111").unwrap();
        std::fs::write(
            vault.join("_system/CHANNEL-PRESENCE.md"),
            "platform: telegram\nchat_id: 999\n",
        )
        .unwrap();
        let relay = hq_core::config::RelayConfig {
            telegram_authorized_chat_id: Some(111),
            ..Default::default()
        };
        assert_eq!(get_notification_chat_id(vault, &relay), Some(111));
    }

    /// Proves the function actually reads the passed-in `RelayConfig` rather than
    /// some cached/global state: two different configs against the same vault
    /// produce two different owner chat IDs.
    #[test]
    fn notification_chat_id_reflects_passed_relay_config_not_global_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let vault = tmp.path();
        std::fs::create_dir_all(vault.join("_system")).unwrap();
        // No .telegram-auth-chat file at all, so the only source of truth is
        // whatever RelayConfig the caller passes in.
        let relay_a = hq_core::config::RelayConfig {
            telegram_authorized_chat_id: Some(111),
            ..Default::default()
        };
        let relay_b = hq_core::config::RelayConfig {
            telegram_authorized_chat_id: Some(222),
            ..Default::default()
        };
        assert_eq!(get_notification_chat_id(vault, &relay_a), Some(111));
        assert_eq!(get_notification_chat_id(vault, &relay_b), Some(222));
    }

    #[test]
    fn single_item_flush_keeps_trait_key_multi_drops_it() {
        // cooldown 0 so the buffer flushes on the next add
        let mut b = DigestBuffer::new(Duration::from_secs(0));
        let one = b.add("only".into(), Some("notif.a.b".into()), false);
        assert_eq!(
            one,
            Some(("only".to_string(), Some("notif.a.b".to_string())))
        );

        // Two pending before a flush → digest text, no single reactable key
        let mut b2 = DigestBuffer::new(Duration::from_secs(3600));
        assert!(
            b2.add("first".into(), Some("notif.a.b".into()), false)
                .is_none()
        );
        b2.cooldown = Duration::from_secs(0);
        let flush = b2
            .add("second".into(), Some("notif.c.d".into()), false)
            .unwrap();
        assert!(flush.0.contains("HQ Digest (2 updates)"));
        assert_eq!(flush.1, None);
    }
}
