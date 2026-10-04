//! Records inbound/outbound relay messages into the shared `chat_threads` /
//! `chat_messages` tables (the same store the Web UI's native chat uses) and
//! broadcasts a live event so connected Web UI clients can follow along.
//! This is what lets Telegram and Discord conversations show up as threads
//! in the Web UI.

use anyhow::Result;
use hq_db::Database;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Upserts platform threads/messages into `vault.db` and (optionally)
/// broadcasts a `relay_message` event over the same WebSocket channel the
/// Web UI's native chat uses.
#[derive(Clone)]
pub struct ThreadSync {
    db: Arc<Database>,
    events: Option<broadcast::Sender<String>>,
}

impl ThreadSync {
    pub fn new(db: Arc<Database>, events: Option<broadcast::Sender<String>>) -> Self {
        Self { db, events }
    }

    /// Record one message on the thread bound to `(platform, external_id)`,
    /// creating the thread on first contact. `role` is `"user"` or
    /// `"assistant"` (matches `hq_db::chat::add_message`'s convention).
    ///
    /// Best-effort: DB errors are logged and swallowed so a recording failure
    /// never breaks the actual Telegram/Discord turn.
    pub fn record(
        &self,
        platform: &str,
        external_id: &str,
        title: &str,
        role: &str,
        content: &str,
    ) {
        if let Err(e) = self.try_record(platform, external_id, title, role, content) {
            tracing::warn!(error = %e, platform, external_id, "thread_sync: record failed (non-fatal)");
        }
    }

    fn try_record(
        &self,
        platform: &str,
        external_id: &str,
        title: &str,
        role: &str,
        content: &str,
    ) -> Result<()> {
        let (thread_id, message) = self.db.with_conn(|conn| {
            let thread =
                hq_db::chat::find_or_create_platform_thread(conn, platform, external_id, title)?;
            let message = hq_db::chat::add_message(conn, &thread.thread_id, role, content)?;
            Ok((thread.thread_id, message))
        })?;

        if let Some(events) = &self.events {
            let payload = serde_json::json!({
                "type": "relay_message",
                "thread_id": thread_id,
                "platform": platform,
                "external_id": external_id,
                "role": role,
                "content": content,
                "created_at": message.created_at,
            });
            // A send error just means there are no subscribers right now — not fatal.
            let _ = events.send(payload.to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::chat;

    fn open_test_db() -> Arc<Database> {
        Arc::new(Database::open_memory().expect("open in-memory db"))
    }

    #[test]
    fn record_creates_thread_and_message() {
        let db = open_test_db();
        let sync = ThreadSync::new(db.clone(), None);

        sync.record("telegram", "chat-1", "Telegram: Alice", "user", "hello");

        let threads = db
            .with_conn(|conn| chat::list_threads(conn, 10, 0, Some("telegram")))
            .unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].external_id.as_deref(), Some("chat-1"));

        let messages = db
            .with_conn(|conn| chat::get_messages(conn, &threads[0].thread_id, 10))
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[0].role, "user");
    }

    #[test]
    fn record_reuses_same_thread_across_calls() {
        let db = open_test_db();
        let sync = ThreadSync::new(db.clone(), None);

        sync.record("discord", "chan-1", "Discord: #general", "user", "hi");
        sync.record(
            "discord",
            "chan-1",
            "Discord: #general",
            "assistant",
            "hello back",
        );

        let threads = db
            .with_conn(|conn| chat::list_threads(conn, 10, 0, Some("discord")))
            .unwrap();
        assert_eq!(
            threads.len(),
            1,
            "second record() must reuse the same thread"
        );

        let messages = db
            .with_conn(|conn| chat::get_messages(conn, &threads[0].thread_id, 10))
            .unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn record_broadcasts_event_when_sender_present() {
        let db = open_test_db();
        let (tx, mut rx) = broadcast::channel(8);
        let sync = ThreadSync::new(db, Some(tx));

        sync.record(
            "telegram",
            "1234",
            "Telegram: @someone",
            "user",
            "ping",
        );

        let received = rx.try_recv().expect("expected a broadcast event");
        let value: serde_json::Value = serde_json::from_str(&received).unwrap();
        assert_eq!(value["type"], "relay_message");
        assert_eq!(value["platform"], "telegram");
        assert_eq!(value["content"], "ping");
    }

    #[test]
    fn record_is_non_fatal_without_events_sender() {
        let db = open_test_db();
        let sync = ThreadSync::new(db, None);
        // Should not panic even though there's nobody to broadcast to.
        sync.record(
            "telegram",
            "chat-2",
            "Telegram: Bob",
            "user",
            "no listeners",
        );
    }
}
