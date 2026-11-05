//! `hq adoption` and `hq offers replay`: how HQ is used and how often the phase 2 offers would
//! appear, read on this machine from its own chat and tool records.
//!
//! Both print counts and the catalog's own wording only, never a message or any part of one, so
//! the output can be read from a phone over `adb` (`scripts/phone-hq.sh`) without the person's
//! chats leaving it. `docs/security/ADOPTION_DATA.md`.

use std::io::Write;

use anyhow::Result;
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_tools::capabilities::adoption::{AdoptionReport, report};
use hq_tools::capabilities::offers::{
    Comparison, DEFAULT_REPLAY_DAYS, ReplayReport, ready_detectors, replay,
};
use hq_tools::capabilities::{Install, catalog, resolve, used_tools};

const DAY: i64 = 86_400;

#[derive(Subcommand)]
pub enum OffersAction {
    /// Run the offer detectors over past chats and print how often each would have fired
    Replay {
        /// How many days back to read (default 56)
        #[arg(long)]
        days: Option<i64>,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
}

/// `hq offers <action>`.
pub fn run_offers(config: &HqConfig, action: OffersAction) -> Result<()> {
    match action {
        OffersAction::Replay { days, json } => run_offers_replay(config, days, json),
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `hq adoption [--json]`.
pub fn run_adoption(config: &HqConfig, as_json: bool) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    let offers = resolve(
        catalog(),
        &Install::detect(config).assuming_model(),
        &|_: &str| true,
        &used_tools(&db),
    );
    let r = db.with_conn(|c| report(c, now(), &offers))?;
    let mut out = std::io::stdout();
    if as_json {
        writeln!(out, "{}", serde_json::to_string_pretty(&r.to_json())?)?;
    } else {
        render_adoption(&r, &mut out)?;
    }
    Ok(())
}

fn days_ago(at: i64, now: i64) -> i64 {
    (now - at).max(0) / DAY
}

fn render_adoption(r: &AdoptionReport, out: &mut dyn Write) -> Result<()> {
    let Some(first) = r.first_message_at else {
        writeln!(out, "Nobody has written to HQ on this machine yet.")?;
        return Ok(());
    };
    writeln!(out, "How HQ is used on this machine")?;
    writeln!(
        out,
        "  First message:        {} days ago",
        days_ago(first, now())
    )?;
    match &r.first_value {
        Some((says, secs)) => writeln!(
            out,
            "  First useful result:  {} min after it ({})",
            secs / 60,
            says.to_lowercase()
        )?,
        None => writeln!(out, "  First useful result:  none beyond plain chat yet")?,
    }
    writeln!(
        out,
        "  Active days:          {} of the last 7, {} of the last 28",
        r.active_days_7, r.active_days_28
    )?;
    writeln!(out, "  Messages this week:   {}", r.messages_this_week)?;
    writeln!(
        out,
        "  Corrections:          {} this week, {} last week",
        r.corrections_this_week, r.corrections_last_week
    )?;
    let month = if r.first_month_over {
        "First four weeks:"
    } else {
        "First weeks so far:"
    };
    writeln!(
        out,
        "  {month:<22}{} different things",
        r.first_month_distinct
    )?;
    match r.returned_week_four {
        Some(true) => writeln!(out, "  Week four:            used again")?,
        Some(false) => writeln!(out, "  Week four:            not used")?,
        None => {}
    }
    writeln!(
        out,
        "\nUsed this week ({} of {} ready; last week {}):",
        r.used_this_week.len(),
        r.ready,
        r.used_last_week
    )?;
    for says in &r.used_this_week {
        writeln!(out, "  - {says}")?;
    }
    writeln!(out, "\nNever tried ({}):", r.never_used.len())?;
    for says in &r.never_used {
        writeln!(out, "  - {says}")?;
    }
    writeln!(
        out,
        "\nCounts only, read on this machine. Nothing was sent anywhere."
    )?;
    Ok(())
}

/// `hq offers replay [--days N] [--json]`.
pub fn run_offers_replay(config: &HqConfig, days: Option<i64>, as_json: bool) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    let days = days.unwrap_or(DEFAULT_REPLAY_DAYS);
    let offers = resolve(
        catalog(),
        &Install::detect(config).assuming_model(),
        &|_: &str| true,
        &used_tools(&db),
    );
    let ready = ready_detectors(&offers);
    let r = db.with_conn(|c| replay(c, now(), days, &ready))?;
    let mut out = std::io::stdout();
    if as_json {
        writeln!(out, "{}", serde_json::to_string_pretty(&r)?)?;
    } else {
        render_replay(&r, &mut out)?;
    }
    Ok(())
}

fn render_replay(r: &Comparison, out: &mut dyn Write) -> Result<()> {
    render_pass("Phase 1 rules (every message, no spacing)", &r.phase1, out)?;
    writeln!(out)?;
    render_pass(
        "Tuned rules, as the apps offer them (your own messages, ready capabilities, spaced out)",
        &r.tuned,
        out,
    )?;
    writeln!(
        out,
        "\n\"Already done\": HQ used that capability within ten minutes anyway, so no offer.\n\
         \"Held back\": not ready on this install, or the same offer came too recently.\n\
         At most one offer per reply. Counts only, read on this machine. Nothing was sent anywhere."
    )?;
    Ok(())
}

fn render_pass(heading: &str, r: &ReplayReport, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "{heading}")?;
    writeln!(
        out,
        "  Last {} days, {} messages read. Replies with an offer: {} ({:.1} a week)",
        r.days,
        r.messages,
        r.replies_with_offer,
        r.offers_per_week()
    )?;
    writeln!(
        out,
        "  {:<18} {:<16} {:>6} {:>13} {:>10} {:>12}",
        "detector", "offers", "fired", "already done", "held back", "would offer"
    )?;
    for c in &r.detectors {
        writeln!(
            out,
            "  {:<18} {:<16} {:>6} {:>13} {:>10} {:>12}",
            c.detector.as_str(),
            c.capability,
            c.fired,
            c.already_done,
            c.held_back,
            c.would_offer
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_tools::capabilities::offers::{Detector, DetectorCount};

    #[test]
    fn the_adoption_page_names_capabilities_and_numbers_only() {
        let r = AdoptionReport {
            first_message_at: Some(now() - 30 * DAY),
            first_value: Some(("Find something you wrote down earlier".into(), 300)),
            used_this_week: vec!["Find something you wrote down earlier".into()],
            used_last_week: 2,
            never_used: vec!["Read the text in a photo or a scanned page".into()],
            ready: 10,
            first_month_distinct: 4,
            first_month_over: true,
            messages_this_week: 12,
            corrections_this_week: 1,
            corrections_last_week: 3,
            active_days_7: 4,
            active_days_28: 15,
            returned_week_four: Some(true),
        };
        let mut out = Vec::new();
        render_adoption(&r, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("30 days ago"));
        assert!(text.contains("5 min after it (find something you wrote down earlier)"));
        assert!(text.contains("4 of the last 7, 15 of the last 28"));
        assert!(text.contains("1 this week, 3 last week"));
        assert!(text.contains("Used this week (1 of 10 ready; last week 2)"));
        assert!(text.contains("- Read the text in a photo"));
        assert!(text.contains("Nothing was sent anywhere"));
        let json = r.to_json();
        assert_eq!(json["first_value"]["after_secs"], 300);
        assert_eq!(json["returned_week_four"], true);
    }

    #[test]
    fn an_empty_machine_says_so() {
        let mut out = Vec::new();
        render_adoption(&AdoptionReport::default(), &mut out).unwrap();
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("Nobody has written")
        );
    }

    #[test]
    fn the_replay_table_has_one_row_per_detector() {
        let pass = |replies| ReplayReport {
            days: 56,
            messages: 200,
            replies_with_offer: replies,
            detectors: Detector::ALL
                .iter()
                .map(|d| DetectorCount {
                    detector: *d,
                    capability: d.capability(),
                    fired: 3,
                    already_done: 1,
                    held_back: 0,
                    would_offer: 2,
                })
                .collect(),
        };
        let r = Comparison {
            phase1: pass(16),
            tuned: pass(8),
        };
        let mut out = Vec::new();
        render_replay(&r, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Last 56 days, 200 messages read"));
        assert!(text.contains("16 (2.0 a week)"));
        assert!(text.contains("8 (1.0 a week)"));
        assert_eq!(text.matches("\n  dated_todo ").count(), 2);
        assert_eq!(
            text.lines().filter(|l| l.contains("    3 ")).count(),
            2 * Detector::ALL.len()
        );
    }
}
