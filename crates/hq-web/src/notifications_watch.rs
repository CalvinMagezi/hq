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
                    "SELECT max(created_at) FROM value_items WHERE state IN ('pending', 'delivered')",
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
pub(crate) fn spawn_notifications_watcher(state: Arc<WsState>) {
    tokio::spawn(async move {
        let mut last = take_notifications_snapshot(&state.db);
        let mut tick = tokio::time::interval(NOTIFICATION_POLL_INTERVAL);
        loop {
            tick.tick().await;
            let current = take_notifications_snapshot(&state.db);
            if current == last {
                continue;
            }
            last = current;

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
    fn test_notifications_snapshot_is_stable_when_nothing_changes() {
        let db = hq_db::Database::open_memory().unwrap();
        let a = take_notifications_snapshot(&db);
        let b = take_notifications_snapshot(&db);
        assert_eq!(a, b);
    }
}
