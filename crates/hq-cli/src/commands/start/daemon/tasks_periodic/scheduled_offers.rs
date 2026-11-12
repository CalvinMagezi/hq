//! `scheduled-offers`: brings back the next steps the owner scheduled, each as a new offer in its
//! chat, with one inbox item that names no task. Nothing runs until the owner taps it. No model.
//! It also settles the "Later" offers: once the owner answers a Later item in the inbox, the offer
//! returns to its chat, once.

use anyhow::Result;
use hq_core::types::{SCHEDULED_OFFER_SOURCE, ValueItem, ValueKind};
use hq_db::Database;
use tracing::{info, warn};

/// Most steps brought back in one pass; the rest follow on the next.
const MAX_PER_PASS: usize = 20;

/// The inbox item for a pass that brought steps back. Inbox items can reach other devices, so it
/// says nothing about the task.
fn notice(now: i64) -> ValueItem {
    ValueItem::new(
        SCHEDULED_OFFER_SOURCE,
        ValueKind::Fyi,
        "A step you scheduled is ready".to_string(),
        "Open your chat to take it or dismiss it.".to_string(),
    )
    .with_dedup_key(format!("scheduled-offer-{now}"))
}

pub async fn run_scheduled_offers(db: &Database) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    // Each part is on its own: one failing never stops the other.
    match db.with_conn(|c| hq_db::offers::deliver_later(c, now)) {
        Ok(0) => {}
        Ok(back) => info!(
            back,
            "scheduled-offers: brought Later offers back to their chats"
        ),
        Err(e) => warn!("scheduled-offers: Later offers failed: {e}"),
    }
    match db.with_conn(|c| hq_db::offers::deliver_due(c, now, MAX_PER_PASS)) {
        Ok(0) => {}
        Ok(delivered) => {
            if let Err(e) = hq_db::value_items::emit(db, &notice(now)) {
                warn!("scheduled-offers: could not write the notice: {e}");
            }
            info!(
                delivered,
                "scheduled-offers: brought steps back to their chats"
            );
        }
        Err(e) => warn!("scheduled-offers: scheduled steps failed: {e}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_stays_on_the_owners_devices() {
        assert!(notice(1).is_device_only());
    }

    #[tokio::test]
    async fn a_failing_later_pass_does_not_stop_the_scheduled_steps() {
        let db = Database::open_memory().unwrap();
        let thread = db
            .with_conn(|c| {
                let t = hq_db::chat::create_thread(c, "t", "user", "user")?;
                c.execute("DROP TABLE offer_later", [])?;
                c.execute(
                    "INSERT INTO offer_schedule (thread_id, offer, line, reply, send, due_at)
                     VALUES (?1, 'next_email', 'l', 'r', 0, 1)",
                    [&t.thread_id],
                )?;
                Ok(t.thread_id)
            })
            .unwrap();
        run_scheduled_offers(&db).await.unwrap();
        let back: i64 = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM chat_messages WHERE thread_id = ?1
                      AND json_extract(meta, '$.offer.kind') = 'next_email'",
                    [&thread],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(back, 1);
    }
}
