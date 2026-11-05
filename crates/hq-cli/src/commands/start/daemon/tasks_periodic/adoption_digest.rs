//! `adoption-digest`: once a week, one item in the owner's inbox that says how HQ was used:
//! which capabilities, how soon the first useful result came, how often HQ had to be corrected,
//! and whether the person keeps coming back. The "before" numbers for every change to how HQ
//! shows itself (`docs/plans/simple-hq.md`, phase 0). Local records only, no model.

use anyhow::Result;
use chrono::Datelike;
use hq_core::config::HqConfig;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::offers::OfferStats;
use hq_tools::capabilities::adoption::{AdoptionReport, report};
use hq_tools::capabilities::offers::offer_says;
use hq_tools::capabilities::{Install, catalog, resolve, used_tools};
use tracing::info;

/// Source name of the digest item, shown in the web inbox. Device-only: web push skips it.
pub use hq_core::types::ADOPTION_DIGEST_SOURCE;
/// Capabilities named in one line; the count still covers all of them.
const MAX_NAMED: usize = 4;
const MINUTE: i64 = 60;
const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

fn after(secs: i64) -> String {
    match secs {
        s if s < MINUTE => "within a minute".to_string(),
        s if s < HOUR => format!("{} minutes", s / MINUTE),
        s if s < DAY => format!("{} hours", s / HOUR),
        s => format!("{} days", s / DAY),
    }
}

fn named(list: &[String]) -> String {
    let mut out = list
        .iter()
        .take(MAX_NAMED)
        .map(|s| s.to_lowercase())
        .collect::<Vec<_>>()
        .join("; ");
    if list.len() > MAX_NAMED {
        out.push_str(&format!("; and {} more", list.len() - MAX_NAMED));
    }
    out
}

/// One line per kind of offer shown under a reply this week, or one muted.
fn offers_lines(offers: &[OfferStats]) -> String {
    if offers.is_empty() {
        return "No offers were shown under replies this week.\n".to_string();
    }
    let mut out = String::from("Offers under replies this week:\n");
    for o in offers {
        out.push_str(&format!(
            "  {}: shown {}, yes {}, not now {}{}\n",
            offer_says(&o.offer).to_lowercase(),
            o.shown,
            o.accepted,
            o.declined,
            if o.muted { ", muted for now" } else { "" }
        ));
    }
    out
}

/// The week's item, or none for a person who has never written to HQ.
pub fn digest_item(r: &AdoptionReport, week: &str) -> Option<ValueItem> {
    r.first_message_at?;
    let title = format!(
        "How HQ was used this week: {} of {} things, {} correction{}",
        r.used_this_week.len(),
        r.ready,
        r.corrections_this_week,
        if r.corrections_this_week == 1 {
            ""
        } else {
            "s"
        }
    );
    let mut body = String::new();
    if r.used_this_week.is_empty() {
        body.push_str("Nothing beyond plain chat was used this week.\n");
    } else {
        body.push_str(&format!(
            "Used this week: {} (last week: {}).\n",
            named(&r.used_this_week),
            r.used_last_week
        ));
    }
    if !r.never_used.is_empty() {
        body.push_str(&format!("Never tried: {}.\n", named(&r.never_used)));
    }
    body.push_str(&format!(
        "Days with a conversation: {} of the last 7, {} of the last 28 ({} messages this week).\n",
        r.active_days_7, r.active_days_28, r.messages_this_week
    ));
    body.push_str(&format!(
        "Corrections (asked for shorter, said it was wrong or not what was meant): {} this week, \
         {} last week.\n",
        r.corrections_this_week, r.corrections_last_week
    ));
    match &r.first_value {
        Some((says, secs)) => body.push_str(&format!(
            "First useful result: {} after the first message ({}).\n",
            after(*secs),
            says.to_lowercase()
        )),
        None => body.push_str("No useful result beyond plain chat yet.\n"),
    }
    let month = if r.first_month_over {
        "In its first four weeks"
    } else {
        "So far in its first four weeks"
    };
    body.push_str(&format!(
        "{month} HQ was used for {} different things.",
        r.first_month_distinct
    ));
    match r.returned_week_four {
        Some(true) => body.push_str(" It was used again in week four.\n"),
        Some(false) => body.push_str(" It was not used in week four.\n"),
        None => body.push('\n'),
    }
    body.push_str(&offers_lines(&r.offers_this_week));
    body.push_str(
        "\nCounted on this machine from chat messages and tool calls. Nothing was sent anywhere.",
    );
    let item = ValueItem::new(ADOPTION_DIGEST_SOURCE, ValueKind::Fyi, title, body);
    Some(item.with_dedup_key(format!("adoption-{week}")))
}

/// Weekly adoption digest. Reports only, from local records.
pub async fn run_adoption_digest(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.adoption_digest {
        return Ok(());
    }
    let now = chrono::Utc::now();
    // The daemon holds no tool registry; every catalog tool is in a category each edition that
    // runs this task keeps (a test in hq-mcp checks they are registered).
    let offers = resolve(
        catalog(),
        &Install::detect(config).assuming_model(),
        &|_: &str| true,
        &used_tools(db),
    );
    let r = db.with_conn(|c| report(c, now.timestamp(), &offers))?;
    let iso = now.iso_week();
    let week = format!("{}-W{:02}", iso.year(), iso.week());
    if let Some(item) = digest_item(&r, &week) {
        hq_db::value_items::emit(db, &item)?;
        info!(
            used = r.used_this_week.len(),
            corrections = r.corrections_this_week,
            "adoption-digest: reported this week's use of HQ"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used() -> AdoptionReport {
        AdoptionReport {
            first_message_at: Some(1),
            first_value: Some(("Find something you wrote down earlier".into(), 300)),
            used_this_week: vec!["A".into(), "B".into(), "C".into(), "D".into(), "E".into()],
            used_last_week: 2,
            never_used: vec!["Read the text in a photo".into()],
            ready: 11,
            first_month_distinct: 6,
            first_month_over: true,
            messages_this_week: 30,
            corrections_this_week: 1,
            corrections_last_week: 4,
            active_days_7: 5,
            active_days_28: 19,
            returned_week_four: Some(true),
            offers_this_week: vec![OfferStats {
                offer: "pasted_list".into(),
                shown: 3,
                accepted: 1,
                declined: 2,
                muted: true,
            }],
        }
    }

    #[test]
    fn nothing_is_posted_for_a_person_who_never_wrote() {
        assert!(digest_item(&AdoptionReport::default(), "2026-W45").is_none());
    }

    #[test]
    fn the_digest_says_what_was_used_and_how_often_hq_was_corrected() {
        let item = digest_item(&used(), "2026-W45").unwrap();
        assert_eq!(
            item.title,
            "How HQ was used this week: 5 of 11 things, 1 correction"
        );
        assert!(item.body.contains("a; b; c; d; and 1 more (last week: 2)"));
        assert!(item.body.contains("Never tried: read the text in a photo."));
        assert!(item.body.contains("5 of the last 7, 19 of the last 28"));
        assert!(item.body.contains("1 this week, 4 last week"));
        assert!(item.body.contains("5 minutes after the first message"));
        assert!(item.body.contains("used again in week four"));
        assert!(item.body.contains(
            "turn a list you paste into separate tasks: shown 3, yes 1, not now 2, muted for now"
        ));
        assert!(item.body.contains("Nothing was sent anywhere"));
        assert!(!item.body.contains('\u{2014}'));
        assert_eq!(item.dedup_key.as_deref(), Some("adoption-2026-W45"));
        assert!(
            item.is_device_only(),
            "never pushed, never shown to a model"
        );
    }

    #[test]
    fn a_quiet_new_install_says_so_plainly() {
        let r = AdoptionReport {
            first_message_at: Some(1),
            ..Default::default()
        };
        let item = digest_item(&r, "2026-W45").unwrap();
        assert!(
            item.body
                .contains("Nothing beyond plain chat was used this week.")
        );
        assert!(
            item.body
                .contains("No useful result beyond plain chat yet.")
        );
        assert!(item.body.contains("So far in its first four weeks"));
        assert!(item.body.contains("No offers were shown"));
    }

    #[test]
    fn durations_read_like_a_person_would_say_them() {
        assert_eq!(after(20), "within a minute");
        assert_eq!(after(600), "10 minutes");
        assert_eq!(after(3 * HOUR), "3 hours");
        assert_eq!(after(2 * DAY + 5), "2 days");
    }
}
