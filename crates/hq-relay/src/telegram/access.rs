//! Telegram owner/guest access control and notification routing.

use hq_core::config::{HqConfig, RelayConfig, TelegramUserRole};
use hq_core::identity::RequestIdentity;
use hq_core::pairing::{self, PairPlatform};
use hq_core::telegram_access::owner_chat_id as core_owner_chat_id;
use std::path::Path;

use super::caller_context::{CallerContext, InstanceMeta};

pub use hq_core::telegram_access::notification_chat_id as get_notification_chat_id;

/// Resolved role for an incoming Telegram chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramRole {
    Owner,
    Guest,
}

/// Read owner chat ID from config or vault auth file.
pub fn owner_chat_id(vault_path: &Path, relay: &RelayConfig) -> Option<i64> {
    core_owner_chat_id(vault_path, relay)
}

/// Outcome of the relay's first gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatGate {
    Proceed,
    Drop,
    /// The sender redeemed a pairing code and is now the owner.
    Paired,
    /// No owner yet: answer once with how to pair, then drop the message.
    PairingHelp(&'static str),
}

/// Reply to a bare `/start` while the bot has no owner.
pub const NOT_PAIRED_REPLY: &str = "This bot is not paired with HQ yet. In HQ, open Settings, then Chat apps, and press Pair my Telegram.";
/// Reply when a pairing code is refused, so an expired link is not a silent dead end.
pub const CODE_REFUSED_REPLY: &str = "That pairing code did not work. It may have expired or been used already. Get a fresh one from HQ Settings, Chat apps.";

/// The command word of `text` without a `@botname` suffix.
fn command_word(text: &str) -> Option<&str> {
    text.split_whitespace().next()?.split('@').next()
}

/// The code from `/pair CODE`, or from `/start CODE`, which a `t.me/<bot>?start=CODE` link sends.
fn pairing_code(text: &str) -> Option<&str> {
    if let Some(code) = pairing::parse_pair_command(text) {
        return Some(code);
    }
    if command_word(text) != Some("/start") {
        return None;
    }
    text.split_whitespace().nth(1)
}

/// The relay's first gate. With no owner known, everything is refused except a
/// valid `/pair <code>` (see `hq pair`); there is no trust on first use.
pub fn authorize_chat(
    vault_path: &Path,
    relay: &RelayConfig,
    chat_id: i64,
    is_private: bool,
    text: &str,
) -> ChatGate {
    authorize_chat_at(
        vault_path,
        relay,
        chat_id,
        is_private,
        text,
        pairing::now_secs(),
    )
}

/// Whether a button tap from `user_id` may change state: only the owner.
pub fn callback_sender_allowed(vault_path: &Path, relay: &RelayConfig, user_id: i64) -> bool {
    owner_chat_id(vault_path, relay) == Some(user_id)
}

fn authorize_chat_at(
    vault_path: &Path,
    relay: &RelayConfig,
    chat_id: i64,
    is_private: bool,
    text: &str,
    now: i64,
) -> ChatGate {
    let is_allowed = relay.telegram_allowed_chat_ids.contains(&chat_id);
    match owner_chat_id(vault_path, relay) {
        Some(owner) if owner != chat_id && !is_allowed => {
            tracing::warn!(
                chat_id,
                authorized = owner,
                "telegram: rejecting message from unauthorized chat"
            );
            ChatGate::Drop
        }
        None if !is_allowed => try_pair(vault_path, chat_id, is_private, text, now),
        _ => ChatGate::Proceed,
    }
}

fn try_pair(vault_path: &Path, chat_id: i64, is_private: bool, text: &str, now: i64) -> ChatGate {
    if !is_private {
        tracing::warn!(
            chat_id,
            "telegram: ignoring message from a group while no owner is configured; pair from a private chat"
        );
        return ChatGate::Drop;
    }
    let Some(code) = pairing_code(text) else {
        tracing::warn!(
            chat_id,
            "telegram: no owner configured, ignoring message. Set relay.telegram_authorized_chat_id or run `hq pair` and send /pair <code>"
        );
        // Only `/start` gets an answer, so other traffic to an unpaired bot still learns nothing.
        if command_word(text) == Some("/start") {
            return ChatGate::PairingHelp(NOT_PAIRED_REPLY);
        }
        return ChatGate::Drop;
    };
    if pairing::redeem_pairing_code(vault_path, PairPlatform::Telegram, code, now).is_err() {
        tracing::warn!(chat_id, "telegram: pairing code refused");
        return ChatGate::PairingHelp(CODE_REFUSED_REPLY);
    }
    let owner_file = hq_core::telegram_access::owner_file(vault_path);
    let written = std::fs::create_dir_all(owner_file.parent().unwrap_or(vault_path))
        .and_then(|()| std::fs::write(&owner_file, chat_id.to_string()));
    if let Err(e) = written {
        tracing::error!(error = %e, "telegram: pairing code accepted but owner file could not be written");
        return ChatGate::Drop;
    }
    tracing::info!(chat_id, "telegram: pairing code accepted, recorded owner");
    ChatGate::Paired
}

/// True if this chat may use the bot (owner or allowlisted guest).
pub fn is_chat_allowed(chat_id: i64, relay: &RelayConfig, owner_id: Option<i64>) -> bool {
    if owner_id == Some(chat_id) {
        return true;
    }
    if relay.telegram_allowed_chat_ids.contains(&chat_id) {
        return true;
    }
    relay.telegram_users.iter().any(|u| u.chat_id == chat_id)
}

/// Role for an allowed chat.
pub fn role_for_chat(chat_id: i64, relay: &RelayConfig, owner_id: Option<i64>) -> TelegramRole {
    if owner_id == Some(chat_id) {
        return TelegramRole::Owner;
    }
    if let Some(entry) = relay.telegram_users.iter().find(|u| u.chat_id == chat_id) {
        return match entry.role {
            TelegramUserRole::Owner => TelegramRole::Owner,
            TelegramUserRole::Guest => TelegramRole::Guest,
        };
    }
    TelegramRole::Guest
}

/// Display name for a chat (from `telegram_users` or fallback `tg-{id}`).
pub fn display_name_for_chat(chat_id: i64, relay: &RelayConfig) -> String {
    relay
        .telegram_users
        .iter()
        .find(|u| u.chat_id == chat_id)
        .map(|u| u.name.clone())
        .unwrap_or_else(|| format!("tg-{chat_id}"))
}

/// Owner display name from config (first owner entry, else a generic fallback).
pub fn owner_display_name(relay: &RelayConfig) -> String {
    relay
        .telegram_users
        .iter()
        .find(|u| u.role == TelegramUserRole::Owner)
        .map(|u| u.name.clone())
        .unwrap_or_else(|| "the owner".to_string())
}

/// Build full caller context for an inbound Telegram message.
pub fn resolve_caller_context(
    caller_chat_id: i64,
    vault_path: &Path,
    config: &HqConfig,
) -> Option<CallerContext> {
    let relay = &config.relay;
    let owner_id = owner_chat_id(vault_path, relay)?;
    if !is_chat_allowed(caller_chat_id, relay, Some(owner_id)) {
        return None;
    }

    let role = role_for_chat(caller_chat_id, relay, Some(owner_id));
    let display_name = display_name_for_chat(caller_chat_id, relay);
    let owner_name = owner_display_name(relay);

    let mut identity = RequestIdentity::from_telegram(caller_chat_id);
    identity.user_name = display_name.clone();

    let relay_model = relay
        .model
        .clone()
        .unwrap_or_else(|| config.default_model.clone());

    Some(CallerContext {
        identity,
        role,
        display_name,
        owner_name,
        owner_chat_id: owner_id,
        caller_chat_id,
        instance: InstanceMeta {
            hostname: hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "localhost".to_string()),
            vault_path: config.vault_path.display().to_string(),
            relay_model,
        },
    })
}

/// Whether the first message in this chat should get a guest intro line.
/// Uses a dedicated marker file written immediately after the intro is sent,
/// not the channel state file (which is only written after the full response).
pub fn guest_thread_is_new(vault_path: &Path, caller_chat_id: i64) -> bool {
    let path = vault_path
        .join("_gateway/channels")
        .join(format!(".tg-{caller_chat_id}-intro-sent"));
    !path.exists()
}

/// Write the intro-sent marker. Call immediately after sending the guest intro.
pub fn mark_guest_intro_sent(vault_path: &Path, caller_chat_id: i64) {
    let dir = vault_path.join("_gateway/channels");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!(".tg-{caller_chat_id}-intro-sent")), "");
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::TelegramUserEntry;
    use tempfile::TempDir;

    #[test]
    fn notification_chat_is_owner_not_presence_guest() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();
        std::fs::create_dir_all(vault.join("_system")).unwrap();
        std::fs::write(vault.join("_system/.telegram-auth-chat"), "111").unwrap();
        std::fs::write(
            vault.join("_system/CHANNEL-PRESENCE.md"),
            "platform: telegram\nchat_id: 999\nlast_active: now\n",
        )
        .unwrap();

        let relay = RelayConfig {
            telegram_authorized_chat_id: Some(111),
            telegram_allowed_chat_ids: vec![999],
            ..Default::default()
        };
        assert_eq!(get_notification_chat_id(vault, &relay), Some(111));
    }

    #[test]
    fn role_owner_vs_guest() {
        let relay = RelayConfig {
            telegram_authorized_chat_id: Some(1),
            telegram_users: vec![
                TelegramUserEntry {
                    chat_id: 1,
                    name: "Alex".into(),
                    role: TelegramUserRole::Owner,
                },
                TelegramUserEntry {
                    chat_id: 2,
                    name: "Bob".into(),
                    role: TelegramUserRole::Guest,
                },
            ],
            ..Default::default()
        };
        assert_eq!(role_for_chat(1, &relay, Some(1)), TelegramRole::Owner);
        assert_eq!(role_for_chat(2, &relay, Some(1)), TelegramRole::Guest);
        assert_eq!(display_name_for_chat(2, &relay), "Bob");
    }

    fn vault() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn gate(v: &Path, relay: &RelayConfig, chat: i64, text: &str, now: i64) -> ChatGate {
        authorize_chat_at(v, relay, chat, true, text, now)
    }

    #[test]
    fn pairing_from_a_group_is_refused_and_keeps_the_code() {
        let v = vault();
        let relay = RelayConfig::default();
        let code = pairing::create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        let cmd = format!("/pair {code}");
        assert_eq!(
            authorize_chat_at(v.path(), &relay, -100, false, &cmd, 1),
            ChatGate::Drop
        );
        assert_eq!(owner_chat_id(v.path(), &relay), None);
        assert_eq!(gate(v.path(), &relay, 7, &cmd, 2), ChatGate::Paired);
    }

    #[test]
    fn only_the_owner_may_tap_buttons() {
        let v = vault();
        let relay = RelayConfig {
            telegram_authorized_chat_id: Some(1),
            telegram_allowed_chat_ids: vec![2],
            ..Default::default()
        };
        assert!(callback_sender_allowed(v.path(), &relay, 1));
        assert!(!callback_sender_allowed(v.path(), &relay, 2));
        assert!(!callback_sender_allowed(v.path(), &relay, 3));
        assert!(!callback_sender_allowed(
            v.path(),
            &RelayConfig::default(),
            1
        ));
    }

    #[test]
    fn unconfigured_relay_refuses_everyone_and_records_nothing() {
        let v = vault();
        let relay = RelayConfig::default();
        assert_eq!(gate(v.path(), &relay, 5, "hello", 0), ChatGate::Drop);
        assert_eq!(
            gate(v.path(), &relay, 5, "/pair ABCDE-FGHJK", 0),
            ChatGate::PairingHelp(CODE_REFUSED_REPLY)
        );
        assert!(!v.path().join("_system/.telegram-auth-chat").exists());
        assert_eq!(owner_chat_id(v.path(), &relay), None);
    }

    #[test]
    fn correct_code_makes_exactly_one_owner() {
        let v = vault();
        let relay = RelayConfig::default();
        let code = pairing::create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        let cmd = format!("/pair {code}");
        assert_eq!(gate(v.path(), &relay, 7, &cmd, 1), ChatGate::Paired);
        assert_eq!(owner_chat_id(v.path(), &relay), Some(7));
        assert_eq!(gate(v.path(), &relay, 8, &cmd, 2), ChatGate::Drop);
        assert_eq!(gate(v.path(), &relay, 8, "hi", 3), ChatGate::Drop);
        assert_eq!(gate(v.path(), &relay, 7, "hi", 3), ChatGate::Proceed);
    }

    #[test]
    fn wrong_expired_and_reused_codes_are_refused() {
        let v = vault();
        let relay = RelayConfig::default();
        let code = pairing::create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        assert_eq!(
            gate(v.path(), &relay, 7, "/pair WRONG-WRONG", 1),
            ChatGate::PairingHelp(CODE_REFUSED_REPLY)
        );
        let late = pairing::PAIRING_TTL_SECS + 1;
        assert_eq!(
            gate(v.path(), &relay, 7, &format!("/pair {code}"), late),
            ChatGate::PairingHelp(CODE_REFUSED_REPLY)
        );
        assert_eq!(owner_chat_id(v.path(), &relay), None);
    }

    #[test]
    fn a_start_link_pairs_like_pair() {
        let v = vault();
        let relay = RelayConfig::default();
        let code = pairing::create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        assert_eq!(
            gate(v.path(), &relay, 7, &format!("/start {code}"), 1),
            ChatGate::Paired
        );
        assert_eq!(owner_chat_id(v.path(), &relay), Some(7));
    }

    #[test]
    fn a_start_link_with_the_bot_suffix_pairs_too() {
        let v = vault();
        let relay = RelayConfig::default();
        let code = pairing::create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        assert_eq!(
            gate(v.path(), &relay, 7, &format!("/start@my_bot {code}"), 1),
            ChatGate::Paired
        );
    }

    #[test]
    fn a_bare_start_gets_one_hint_and_other_text_gets_nothing() {
        let v = vault();
        let relay = RelayConfig::default();
        let hint = ChatGate::PairingHelp(NOT_PAIRED_REPLY);
        assert_eq!(gate(v.path(), &relay, 7, "/start", 0), hint);
        assert_eq!(gate(v.path(), &relay, 7, "/start@my_bot", 0), hint);
        assert_eq!(gate(v.path(), &relay, 7, "/started", 0), ChatGate::Drop);
        assert_eq!(gate(v.path(), &relay, 7, "start", 0), ChatGate::Drop);
        assert_eq!(gate(v.path(), &relay, 7, "/help", 0), ChatGate::Drop);
        assert_eq!(
            authorize_chat_at(v.path(), &relay, -100, false, "/start", 0),
            ChatGate::Drop
        );
    }

    #[test]
    fn a_start_once_paired_is_an_ordinary_message() {
        let v = vault();
        let relay = RelayConfig {
            telegram_authorized_chat_id: Some(1),
            ..Default::default()
        };
        assert_eq!(gate(v.path(), &relay, 1, "/start", 0), ChatGate::Proceed);
        assert_eq!(gate(v.path(), &relay, 2, "/start", 0), ChatGate::Drop);
    }

    #[test]
    fn pairing_is_ignored_once_an_owner_is_configured() {
        let v = vault();
        let relay = RelayConfig {
            telegram_authorized_chat_id: Some(1),
            ..Default::default()
        };
        let code = pairing::create_pairing_code(v.path(), PairPlatform::Telegram, 0).unwrap();
        assert_eq!(
            gate(v.path(), &relay, 9, &format!("/pair {code}"), 1),
            ChatGate::Drop
        );
        assert_eq!(gate(v.path(), &relay, 1, "hi", 1), ChatGate::Proceed);
    }

    #[test]
    fn configured_owner_allowlist_and_users_flows_are_unchanged() {
        let v = vault();
        let relay = RelayConfig {
            telegram_authorized_chat_id: Some(1),
            telegram_allowed_chat_ids: vec![2],
            ..Default::default()
        };
        assert_eq!(gate(v.path(), &relay, 1, "x", 0), ChatGate::Proceed);
        assert_eq!(gate(v.path(), &relay, 2, "x", 0), ChatGate::Proceed);
        assert_eq!(gate(v.path(), &relay, 3, "x", 0), ChatGate::Drop);
        let by_users = RelayConfig {
            telegram_users: vec![TelegramUserEntry {
                chat_id: 4,
                name: "A".into(),
                role: TelegramUserRole::Owner,
            }],
            ..Default::default()
        };
        assert_eq!(gate(v.path(), &by_users, 4, "x", 0), ChatGate::Proceed);
        assert_eq!(gate(v.path(), &by_users, 5, "x", 0), ChatGate::Drop);
    }
}
