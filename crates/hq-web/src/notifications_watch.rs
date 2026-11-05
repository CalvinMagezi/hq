//! Background watcher for notification state.
//!
//! Monitors SQLite `value_items` for changes and broadcasts `notification:badge_count` over WS.

use crate::WsState;
use std::sync::Arc;
use std::time::Duration;

/// Poll interval for checking notification changes.
const NOTIFICATION_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Snapshot representing the latest state of notification sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotificationsSnapshot {
    pub(crate) value_items_latest: Option<String>,
}

pub(crate) fn take_notifications_snapshot(db: &hq_db::Database) -> NotificationsSnapshot {
    let value_items_latest = db
        .with_conn(|conn| {
            let row = conn
                .query_row(
                    // The count too: answering any item that is not the newest changes it.
                    "SELECT count(*) || '|' || coalesce(max(created_at), '') FROM value_items \
                     WHERE state IN ('pending', 'delivered')",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .unwrap_or(None);
            Ok(row)
        })
        .unwrap_or(None);

    NotificationsSnapshot { value_items_latest }
}

/// Spawn the notifications watcher loop on the Tokio runtime.
/// What the notes index last saw: how many notes, and the newest sync. A change means a note was
/// written, moved or deleted, by the owner or an agent.
pub(crate) fn notes_fingerprint(db: &hq_db::Database) -> Option<(i64, i64)> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT COUNT(*), COALESCE(MAX(synced_at), 0) FROM sync_state",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })
    .ok()
}

pub(crate) fn spawn_notifications_watcher(state: Arc<WsState>) {
    tokio::spawn(async move {
        let mut last = take_notifications_snapshot(&state.db);
        let mut notes = notes_fingerprint(&state.db);
        let mut tick = tokio::time::interval(NOTIFICATION_POLL_INTERVAL);
        loop {
            tick.tick().await;
            let now_notes = notes_fingerprint(&state.db);
            if now_notes != notes {
                notes = now_notes;
                state.announce_change(crate::CHANGE_NOTES);
            }
            let current = take_notifications_snapshot(&state.db);
            if current == last {
                continue;
            }
            last = current;
            state.announce_change(crate::CHANGE_INBOX);

            // Fetch fresh aggregated counts
            if let Ok(res) = crate::notifications_api::aggregate_notifications(
                &state.db,
                Some("pending"),
                None,
                50,
            ) {
                state.broadcast(
                    &serde_json::json!({
                        "type": "notification:badge_count",
                        "unread_count": res.unread_count,
                        "pending_approvals_count": res.pending_approvals_count,
                    })
                    .to_string(),
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_note_changes_the_notes_fingerprint() {
        let db = hq_db::Database::open_memory().unwrap();
        let before = notes_fingerprint(&db);
        db.with_conn(|c| {
            Ok(c.execute(
                "INSERT INTO sync_state (path, content_hash, modified_at, synced_at) VALUES ('a.md', 'h', 1, 2)",
                [],
            )?)
        })
        .unwrap();
        assert_ne!(notes_fingerprint(&db), before);
    }

    #[tokio::test]
    async fn a_change_is_announced_by_place() {
        let dir = tempfile::tempdir().unwrap();
        let state = WsState::new(dir.path().to_path_buf(), None);
        let mut rx = state.tx.subscribe();
        state.announce_change(crate::CHANGE_BUDGET);
        let msg: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        assert_eq!(msg["type"], "budget:change");
    }

    #[test]
    fn test_notifications_snapshot_is_stable_when_nothing_changes() {
        let db = hq_db::Database::open_memory().unwrap();
        let a = take_notifications_snapshot(&db);
        let b = take_notifications_snapshot(&db);
        assert_eq!(a, b);
    }

    #[test]
    fn answering_an_older_item_is_a_change() {
        let db = hq_db::Database::open_memory().unwrap();
        let old = hq_core::types::ValueItem::new(
            "t",
            hq_core::types::ValueKind::ActionNeeded,
            "old",
            "b",
        );
        hq_db::value_items::insert(&db, &old).unwrap();
        let mut new = hq_core::types::ValueItem::new(
            "t",
            hq_core::types::ValueKind::ActionNeeded,
            "new",
            "b",
        );
        new.created_at = old.created_at + chrono::Duration::seconds(5);
        hq_db::value_items::insert(&db, &new).unwrap();
        let before = take_notifications_snapshot(&db);
        assert!(hq_db::value_items::dismiss_by_id(&db, &old.id).unwrap());
        assert_ne!(take_notifications_snapshot(&db), before);
    }
}
