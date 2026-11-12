//! `inbox-tidy`: once a day, helper requests nobody is waiting on leave Needs you. Nothing is
//! deleted: they show in Done as "No longer waiting" and `hq inbox restore` brings them back for
//! 30 days. Off with `daemon.inbox_tidy: false`. Rules: `docs/plans/quiet-fleet.md`.

use anyhow::Result;
use chrono::Utc;
use hq_core::config::HqConfig;
use hq_db::Database;
use tracing::info;

pub async fn run_inbox_tidy(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.inbox_tidy || !config.inbox.fleet.enabled {
        return Ok(());
    }
    // Right after a start the registry still holds the last run's `last_seen_at`; tidy only
    // once the supervisor has swept in this process.
    if !hq_db::inbox_tidy::supervisor_swept() {
        info!("inbox-tidy: waiting for the first supervisor sweep");
        return Ok(());
    }
    let report = hq_db::inbox_tidy::tidy(db, Utc::now(), &config.inbox.fleet, false)?;
    if report.total > 0 {
        info!(
            moved = report.total,
            "inbox-tidy: moved stale helper requests out of Needs you"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::{ValueItem, ValueKind};

    fn old_signin(db: &Database) -> String {
        let mut item = ValueItem::new("device-sign-in", ValueKind::ActionNeeded, "t", "b")
            .with_dedup_key("device-sign-in-x")
            .owner_only(true);
        item.created_at = Utc::now() - chrono::Duration::hours(48);
        hq_db::value_items::insert(db, &item).unwrap();
        item.id
    }

    fn state(db: &Database, id: &str) -> String {
        db.with_conn(|c| {
            Ok(
                c.query_row("SELECT state FROM value_items WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap()
    }

    #[tokio::test]
    async fn the_switch_turns_it_off_and_a_run_applies_the_tidy() {
        let db = Database::open_memory().unwrap();
        let id = old_signin(&db);
        let mut config = HqConfig::default();
        config.daemon.inbox_tidy = false;
        run_inbox_tidy(&db, &config).await.unwrap();
        assert_eq!(state(&db, &id), "pending");
        config.daemon.inbox_tidy = true;
        // No sweep yet in this process: nothing is touched.
        if !hq_db::inbox_tidy::supervisor_swept() {
            run_inbox_tidy(&db, &config).await.unwrap();
            assert_eq!(state(&db, &id), "pending");
        }
        hq_db::inbox_tidy::mark_supervisor_swept();
        run_inbox_tidy(&db, &config).await.unwrap();
        assert_eq!(state(&db, &id), "expired");
    }
}
