//! `scheduled-offers`: brings back the next steps the owner scheduled, each as a new offer in its
//! chat, with one inbox item that names no task. Nothing runs until the owner taps it. No model.

use anyhow::Result;
use hq_core::types::{SCHEDULED_OFFER_SOURCE, ValueItem, ValueKind};
use hq_db::Database;
use tracing::info;

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
    let delivered = db.with_conn(|c| hq_db::offers::deliver_due(c, now, MAX_PER_PASS))?;
    if delivered > 0 {
        hq_db::value_items::emit(db, &notice(now))?;
        info!(
            delivered,
            "scheduled-offers: brought steps back to their chats"
        );
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
}
