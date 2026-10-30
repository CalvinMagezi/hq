//! Value Bus delivery: new inbox items are marked delivered, which is all the web inbox, web
//! push and the phone app need. Chat relays (Telegram, Discord) carry chat only, never these
//! notifications. Taps on Approve or Dismiss in messages sent before that still record
//! engagement.

use anyhow::Result;
use std::path::Path;

use hq_core::types::ValueState;
use hq_db::Database;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueAction {
    Approve,
    Dismiss,
}

/// Marks every pending item delivered. Nothing is sent to a chat relay.
pub async fn run_value_bus_delivery(_vault_path: &Path, db: &Database) -> Result<()> {
    let _ = hq_db::value_items::gc_expired(db)?;
    for item in hq_db::value_items::list_by_state(db, ValueState::Pending)? {
        hq_db::value_items::set_delivered(db, &item.id)?;
    }
    Ok(())
}

/// Parse a Telegram reply like "approve a1b2c3d4" or "dismiss a1b2c3d4".
/// Distinct keywords from email-triage's send/skip so the two never collide.
pub fn parse_value_command(text: &str) -> Option<(ValueAction, String)> {
    crate::parse_token_command(
        text,
        &[
            ("approve", ValueAction::Approve),
            ("dismiss", ValueAction::Dismiss),
        ],
    )
    .map(|(action, token, _)| (action, token))
}

/// Consume a tapped token and record engagement. Returns true if it matched a
/// delivered item. Opens the canonical vault database (called from the relay layer).
pub fn record_engagement(vault_path: &Path, token: &str, action: ValueAction) -> Result<bool> {
    let db_path = vault_path.join("_data").join("vault.db");
    let db = Database::open(&db_path)?;
    let outcome = match action {
        ValueAction::Approve => "approved",
        ValueAction::Dismiss => "dismissed",
    };
    hq_db::value_items::record_engagement_by_token(&db, token, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::{ValueItem, ValueKind};

    /// Open the canonical vault db so that handles obtained via path (emit_at,
    /// record_engagement) and via this handle point at the SAME file.
    fn open_vault_db(vault: &Path) -> Database {
        let db_path = vault.join("_data").join("vault.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        Database::open(&db_path).unwrap()
    }

    #[test]
    fn parse_value_command_extracts_action_and_token() {
        assert_eq!(
            parse_value_command("approve a1b2c3d4"),
            Some((ValueAction::Approve, "a1b2c3d4".to_string()))
        );
        assert_eq!(
            parse_value_command("dismiss a1b2c3d4"),
            Some((ValueAction::Dismiss, "a1b2c3d4".to_string()))
        );
        assert_eq!(parse_value_command("hello there"), None);
        assert_eq!(parse_value_command("approve ab"), None); // token too short
    }

    #[tokio::test]
    async fn no_item_reaches_a_chat_relay() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let db = open_vault_db(vault);
        for (source, kind) in [
            ("task_ready_for_review", ValueKind::ActionNeeded),
            ("email-triage", ValueKind::ActionNeeded),
            ("memory-consolidation", ValueKind::Fyi),
            ("connector", ValueKind::ActionNeeded),
        ] {
            hq_db::value_items::emit(&db, &ValueItem::new(source, kind, "t", "b")).unwrap();
        }

        run_value_bus_delivery(vault, &db).await.unwrap();

        assert!(
            hq_core::mailbox::peek_messages(vault, "relay")
                .unwrap()
                .is_empty(),
            "chat relays carry chat only"
        );
        assert!(
            hq_db::value_items::list_by_state(&db, ValueState::Pending)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            hq_db::value_items::list_by_state(&db, ValueState::Delivered)
                .unwrap()
                .len(),
            4,
            "every item is delivered to the inbox"
        );
    }
}
