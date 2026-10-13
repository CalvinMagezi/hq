//! Telegram owner chat resolution (shared by relay and daemon).

use crate::config::RelayConfig;
use std::path::{Path, PathBuf};

/// Where a redeemed pairing code records the owner chat.
pub fn owner_file(vault_path: &Path) -> PathBuf {
    vault_path.join("_system").join(".telegram-auth-chat")
}

/// The owner chat a pairing recorded, if any.
pub fn paired_owner(vault_path: &Path) -> Option<i64> {
    std::fs::read_to_string(owner_file(vault_path))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Owner chat ID from config or vault auth file.
pub fn owner_chat_id(vault_path: &Path, relay: &RelayConfig) -> Option<i64> {
    relay
        .telegram_authorized_chat_id
        .or_else(|| paired_owner(vault_path))
        .or_else(|| {
            relay
                .telegram_users
                .iter()
                .find(|u| u.role == crate::config::TelegramUserRole::Owner)
                .map(|u| u.chat_id)
        })
}

/// Whether the config itself names an owner, which pairing cannot replace.
pub fn owner_set_in_config(relay: &RelayConfig) -> bool {
    relay.telegram_authorized_chat_id.is_some()
        || relay
            .telegram_users
            .iter()
            .any(|u| u.role == crate::config::TelegramUserRole::Owner)
}

/// Proactive Telegram notifications always go to the owner, never guests.
pub fn notification_chat_id(vault_path: &Path, relay: &RelayConfig) -> Option<i64> {
    owner_chat_id(vault_path, relay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn notification_ignores_presence_guest() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();
        std::fs::create_dir_all(vault.join("_system")).unwrap();
        std::fs::write(vault.join("_system/.telegram-auth-chat"), "111").unwrap();
        let relay = RelayConfig {
            telegram_authorized_chat_id: Some(111),
            ..Default::default()
        };
        assert_eq!(notification_chat_id(vault, &relay), Some(111));
    }
}
