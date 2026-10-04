//! Telegram owner chat resolution (shared by relay and daemon).

use crate::config::RelayConfig;
use std::path::Path;

/// Owner chat ID from config or vault auth file.
pub fn owner_chat_id(vault_path: &Path, relay: &RelayConfig) -> Option<i64> {
    relay
        .telegram_authorized_chat_id
        .or_else(|| {
            std::fs::read_to_string(vault_path.join("_system/.telegram-auth-chat"))
                .ok()
                .and_then(|s| s.trim().parse().ok())
        })
        .or_else(|| {
            relay
                .telegram_users
                .iter()
                .find(|u| u.role == crate::config::TelegramUserRole::Owner)
                .map(|u| u.chat_id)
        })
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
