//! Value Bus delivery: rank pending items, budget them, send each into the relay
//! mailbox (actionable ones with an approve/dismiss token), and record
//! engagement on tap.

use anyhow::Result;
use std::path::Path;

use hq_core::types::{MailboxMessageType, ValueItem, ValueKind, ValueState};
use hq_db::Database;

use hq_core::mailbox;

/// Max items delivered per cycle. Keeps Telegram from being flooded; the rest
/// stay Pending for the next cycle.
const VALUE_BUS_BUDGET: usize = 3;

/// Source tasks whose ActionNeeded items demand same-day attention.
const SAME_DAY_ACTION_SOURCES: &[&str] = &["email-triage"];
/// Source tasks whose items belong in the web inbox only, never the relay.
/// `skill_review` is `hq_agent::skill_review::VALUE_SOURCE`: skill changes are FYI.
/// `session_chat` is a harness session a web chat watches that needs an answer.
pub const WEB_ONLY_SOURCES: &[&str] = &["task_ready_for_review", "skill_review", "session_chat"];

/// Only these kinds ask the owner to decide something, so only they get
/// Approve/Dismiss buttons; everything else is a plain notice.
fn is_actionable(kind: ValueKind) -> bool {
    matches!(kind, ValueKind::ActionNeeded | ValueKind::Proposal)
}

/// Decide whether an item interrupts now or goes through normal relay batching.
pub fn is_urgent(item: &ValueItem) -> bool {
    item.kind == ValueKind::ActionNeeded
        && SAME_DAY_ACTION_SOURCES.contains(&item.source_task.as_str())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueAction {
    Approve,
    Dismiss,
}

/// Rank + budget + deliver pending value items to the relay mailbox.
pub async fn run_value_bus_delivery(vault_path: &Path, db: &Database) -> Result<()> {
    let _ = hq_db::value_items::gc_expired(db)?;
    let pending = hq_db::value_items::list_by_state(db, ValueState::Pending)?;
    let (web_only, relayed): (Vec<_>, Vec<_>) = pending
        .into_iter()
        .partition(|i| WEB_ONLY_SOURCES.contains(&i.source_task.as_str()));
    // Delivered to the web inbox by being listed there; never sent to the relay.
    for item in &web_only {
        hq_db::value_items::set_delivered(db, &item.id)?;
    }
    for item in relayed.into_iter().take(VALUE_BUS_BUDGET) {
        let token: String = item.id.chars().take(8).collect();
        let body = render(&item);
        let mut out = mailbox::new_message(
            "value-bus",
            "relay",
            MailboxMessageType::Nudge,
            Some(&item.title),
            &body,
            None,
        );
        if is_actionable(item.kind) {
            out.meta.insert("value_token".to_string(), token);
        }
        // No digest collects non-urgent items any more, so every item is sent;
        // only urgent ones bypass the notification gate's batching.
        if is_urgent(&item) {
            out.meta.insert(
                hq_core::mailbox::META_INTERRUPT.to_string(),
                "true".to_string(),
            );
        }
        if mailbox::send_message(vault_path, &out).is_ok() {
            hq_db::value_items::set_delivered(db, &item.id)?;
        }
    }
    Ok(())
}

fn render(item: &ValueItem) -> String {
    let mut s = format!("{}\n\n{}", item.title, item.body);
    if let Some(p) = &item.artifact_path {
        s.push_str(&format!("\n\n↪ {p}"));
    }
    // No text instructions here: the relay layer attaches Approve/Dismiss
    // inline-keyboard buttons for this message (see
    // hq-relay/src/telegram/mailbox.rs, `msg.from == "value-bus"` branch).
    // Baking `approve {token}`/`dismiss {token}` text in here as well was
    // stale, kept alongside the button after the pill migration landed.
    s
}

/// Parse a Telegram reply like "approve a1b2c3d4" or "dismiss a1b2c3d4".
/// Distinct keywords from email-triage's send/skip so the two never collide.
pub fn parse_value_command(text: &str) -> Option<(ValueAction, String)> {
    crate::parse_token_command(
        text,
        &[("approve", ValueAction::Approve), ("dismiss", ValueAction::Dismiss)],
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
    use hq_core::types::ValueKind;

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

    #[test]
    fn urgency_by_kind_and_source() {
        use hq_core::types::ValueKind;
        let a = ValueItem::new("email-triage", ValueKind::ActionNeeded, "t", "b");
        assert!(is_urgent(&a));
        let i = ValueItem::new("memory-consolidation", ValueKind::Insight, "t", "b");
        assert!(!is_urgent(&i));
    }

    #[tokio::test]
    async fn task_items_stay_web_only_and_notices_get_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let db = open_vault_db(vault);
        let task = ValueItem::new("task_ready_for_review", ValueKind::ActionNeeded, "FR-1: x", "b");
        let notice = ValueItem::new("memory-consolidation", ValueKind::Fyi, "Notice", "b");
        hq_db::value_items::emit(&db, &task).unwrap();
        hq_db::value_items::emit(&db, &notice).unwrap();

        run_value_bus_delivery(vault, &db).await.unwrap();

        let relay_dir = vault.join("_mailboxes").join("relay");
        let msgs: Vec<String> = std::fs::read_dir(&relay_dir)
            .unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        assert_eq!(msgs.len(), 1, "only the notice reaches the relay");
        assert!(msgs[0].contains("Notice") && !msgs[0].contains("value_token"));
        let pending = hq_db::value_items::list_by_state(&db, ValueState::Pending).unwrap();
        assert!(pending.is_empty(), "the task item is marked delivered for the web inbox");
    }

    #[tokio::test]
    async fn loop_closes_emit_deliver_engage() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let db = open_vault_db(vault);

        // Emit two action items with the SAME dedup key (one must collapse) plus one insight.
        let a = ValueItem::new("email-triage", ValueKind::ActionNeeded, "Reply A", "body")
            .with_dedup_key("thread-1");
        let dup = ValueItem::new(
            "email-triage",
            ValueKind::ActionNeeded,
            "Reply A dup",
            "body",
        )
        .with_dedup_key("thread-1");
        let insight = ValueItem::new(
            "memory-consolidation",
            ValueKind::Insight,
            "Insight",
            "body",
        );
        hq_db::value_items::emit(&db, &a).unwrap();
        hq_db::value_items::emit(&db, &dup).unwrap(); // collapses
        hq_db::value_items::emit(&db, &insight).unwrap();

        run_value_bus_delivery(vault, &db).await.unwrap();

        // Both surviving items are delivered; the duplicate collapsed on emit.
        let relay_dir = vault.join("_mailboxes").join("relay");
        let count = std::fs::read_dir(&relay_dir).unwrap().count();
        assert_eq!(count, 2, "urgent and non-urgent items both delivered");

        // The action item (highest score) is delivered; engage it by token.
        let token: String = a.id.chars().take(8).collect();
        assert!(record_engagement(vault, &token, ValueAction::Approve).unwrap());

        let stats = hq_db::value_items::task_engagement_stats(&db).unwrap();
        let et = stats
            .iter()
            .find(|s| s.source_task == "email-triage")
            .unwrap();
        assert_eq!(et.delivered, 1);
        assert_eq!(et.engaged, 1);
    }

}
