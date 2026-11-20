//! `finance-nudge`: once a week, for a person who has a finance ledger, one inbox item that says
//! how many rows wait for review and how old the newest transaction is. It never reads a message
//! and never imports anything: importing needs the owner's approval each time. Counts and a date
//! only, no model, and the item is device-only so no push service carries it.

use std::path::Path;

use anyhow::Result;
use chrono::{Datelike, NaiveDate, Utc};
use hq_core::config::HqConfig;
use hq_core::types::{FINANCE_NUDGE_SOURCE, ValueItem, ValueKind};
use hq_db::Database;
use hq_finance::store::{Summary, summary};
use tracing::info;

/// A newest transaction older than this is worth mentioning, because the ledger has stopped
/// following the person's money.
const STALE_AFTER_DAYS: i64 = 7;

/// The week's item, or none when there is nothing to say.
pub fn nudge_item(s: &Summary, today: NaiveDate, week: &str) -> Option<ValueItem> {
    let age = s
        .latest_date
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| (today - d).num_days());
    let stale = age.is_some_and(|a| a > STALE_AFTER_DAYS);
    if s.waiting_for_review == 0 && !stale {
        return None;
    }
    let mut lines = Vec::new();
    if s.waiting_for_review > 0 {
        lines.push(format!(
            "{} finance row{} waiting for you to review. Totals count only rows you have confirmed.",
            s.waiting_for_review,
            if s.waiting_for_review == 1 { " is" } else { "s are" }
        ));
    }
    if let (true, Some(a)) = (stale, age) {
        lines.push(format!(
            "The newest transaction in your ledger is {a} days old. Ask HQ to import your mobile-money messages to bring it up to date."
        ));
    }
    let title = if s.waiting_for_review > 0 {
        format!("{} finance rows to review", s.waiting_for_review)
    } else {
        "Your finance ledger is out of date".to_string()
    };
    lines.push("Counted from your own ledger on this device. Nothing was read or sent.".into());
    let item = ValueItem::new(
        FINANCE_NUDGE_SOURCE,
        ValueKind::Fyi,
        title,
        lines.join("\n"),
    );
    Some(item.with_dedup_key(format!("finance-{week}")))
}

/// Weekly finance reminder. Reads the ledger's counts only.
pub async fn run_finance_nudge(db: &Database, config: &HqConfig, vault: &Path) -> Result<()> {
    if !config.daemon.finance_nudge {
        return Ok(());
    }
    let Some(s) = summary(vault) else {
        return Ok(());
    };
    let now = Utc::now();
    let iso = now.iso_week();
    let week = format!("{}-W{:02}", iso.year(), iso.week());
    if let Some(item) = nudge_item(&s, now.date_naive(), &week) {
        hq_db::value_items::emit(db, &item)?;
        info!(
            waiting = s.waiting_for_review,
            "finance-nudge: reminded about the finance ledger"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn summary_of(waiting: usize, latest: &str) -> Summary {
        Summary {
            rows: 20,
            waiting_for_review: waiting,
            latest_date: Some(latest.into()),
        }
    }

    #[test]
    fn rows_waiting_and_a_stale_ledger_each_get_a_line() {
        let item =
            nudge_item(&summary_of(12, "2026-03-01"), day("2026-03-20"), "2026-W12").unwrap();
        assert_eq!(item.title, "12 finance rows to review");
        assert!(item.body.contains("12 finance rows are waiting"));
        assert!(item.body.contains("19 days old"));
        assert!(
            item.is_device_only(),
            "never pushed, never shown to a model"
        );
        assert_eq!(item.dedup_key.as_deref(), Some("finance-2026-W12"));
    }

    #[test]
    fn a_fresh_reviewed_ledger_is_left_alone() {
        assert!(nudge_item(&summary_of(0, "2026-03-18"), day("2026-03-20"), "w").is_none());
    }

    #[test]
    fn the_item_holds_no_amount_name_or_dash() {
        let item = nudge_item(&summary_of(1, "2026-03-01"), day("2026-03-20"), "w").unwrap();
        assert!(item.body.contains("1 finance row is waiting"));
        assert!(!item.body.contains('\u{2014}') && !item.body.contains("UGX"));
    }
}
