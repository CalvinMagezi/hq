//! The relay mailbox, drained without sending: Telegram carries chat only, never notifications.
//!
//! Background work still writes notices to `_mailboxes/relay` in places; the inbox (web, web
//! push and the phone app) is where the owner sees what needs them. The callback parsers stay so
//! a tap on a button of a message sent before this change is still understood.

use std::sync::Arc;

const VALUE_APPROVE_PREFIX: &str = "value_approve_";
const VALUE_DISMISS_PREFIX: &str = "value_dismiss_";
const NOTIF_KEEP_PREFIX: &str = "notif_keep_";
const NOTIF_MUTE_PREFIX: &str = "notif_mute_";

/// Parse a value-bus callback_data string back to (action, token).
pub fn parse_value_callback_data(data: &str) -> Option<(&'static str, &str)> {
    if let Some(id) = data.strip_prefix(VALUE_APPROVE_PREFIX) {
        Some(("approve", id))
    } else if let Some(id) = data.strip_prefix(VALUE_DISMISS_PREFIX) {
        Some(("dismiss", id))
    } else {
        None
    }
}

/// Parse a keep/mute tap's callback_data back into `(trait_key, gate_value)`.
pub fn parse_reaction_callback_data(data: &str) -> Option<(String, &'static str)> {
    if let Some(key) = data.strip_prefix(NOTIF_KEEP_PREFIX) {
        Some((key.to_string(), "signal"))
    } else if let Some(key) = data.strip_prefix(NOTIF_MUTE_PREFIX) {
        Some((key.to_string(), "noise"))
    } else {
        None
    }
}

const POLL_EVERY: tokio::time::Duration = tokio::time::Duration::from_secs(45);

/// Where notices waited for a chat before relays stopped carrying them.
const LEGACY_PENDING_NOTIFS: &str = "_gateway/pending_notifs.jsonl";

/// Background task: empties `_mailboxes/relay/` every 45s so it cannot grow, and sends nothing.
/// A chat flow Discord owns is put back for Discord's poller.
pub async fn run_mailbox_poller(vault_path: Arc<std::path::PathBuf>) {
    let _ = std::fs::remove_file(vault_path.join(LEGACY_PENDING_NOTIFS));
    let mut interval = tokio::time::interval(POLL_EVERY);
    loop {
        interval.tick().await;
        for msg in crate::relay_common::take_chat_flows(&vault_path) {
            let _ = hq_core::mailbox::send_message(&vault_path, &msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_buttons_are_still_understood() {
        assert_eq!(
            parse_value_callback_data("value_approve_ab12cd34"),
            Some(("approve", "ab12cd34"))
        );
        assert_eq!(
            parse_reaction_callback_data("notif_mute_notif.github.ci"),
            Some(("notif.github.ci".to_string(), "noise"))
        );
        assert_eq!(parse_value_callback_data("other"), None);
    }
}
