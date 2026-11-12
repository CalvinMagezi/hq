//! `hq inbox tidy` and `hq inbox restore`: move requests nobody is waiting on out of Needs you,
//! and bring them back. Prints counts by source, kind and rule, never an item's text.
//! Rules: `docs/plans/quiet-fleet.md`.

use std::io::Write;

use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Utc};
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::inbox_tidy::{TidyReport, restore_expired, tidy};

#[derive(Subcommand)]
pub enum InboxAction {
    /// Show (default) or apply the tidy: helper requests whose session ended, stopped or went
    /// quiet, or that are older than `inbox.fleet.ttl_hours`, leave Needs you. Nothing is deleted.
    Tidy {
        /// Only count what would move (the default)
        #[arg(long, conflicts_with = "apply")]
        dry_run: bool,
        /// Move them, with the reason `auto_tidy`
        #[arg(long)]
        apply: bool,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Bring back what the tidy moved since a point in time (at most 30 days back)
    Restore {
        /// How far back to look: 30m, 12h or 7d
        #[arg(long)]
        since: String,
    },
}

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// `30m`, `12h` or `7d`.
fn parse_since(text: &str) -> Result<Duration> {
    let text = text.trim();
    // The unit is the last character, which may be more than one byte.
    let Some(unit) = text.chars().next_back() else {
        bail!("--since wants a number and a unit, like 12h or 7d");
    };
    let digits = &text[..text.len() - unit.len_utf8()];
    let n: i64 = digits
        .parse()
        .map_err(|_| anyhow::anyhow!("--since wants a number and a unit, like 12h or 7d"))?;
    let seconds = match unit {
        'm' => MINUTE,
        'h' => HOUR,
        'd' => DAY,
        _ => bail!("--since wants a unit of m, h or d, like 12h or 7d"),
    };
    if n <= 0 {
        bail!("--since must be more than zero");
    }
    Ok(Duration::seconds(n.saturating_mul(seconds)))
}

pub fn run(config: &HqConfig, action: InboxAction) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    let mut out = std::io::stdout();
    match action {
        InboxAction::Tidy { apply, json, .. } => {
            let report = tidy(&db, Utc::now(), &config.inbox.fleet, !apply)?;
            if json {
                writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?;
            } else {
                render(&report, &mut out)?;
            }
        }
        InboxAction::Restore { since } => {
            let from: DateTime<Utc> = Utc::now() - parse_since(&since)?;
            let n = restore_expired(&db, from)?;
            writeln!(out, "Restored {n} item(s) to Needs you.")?;
        }
    }
    Ok(())
}

fn render(r: &TidyReport, out: &mut impl Write) -> Result<()> {
    let verb = if r.dry_run { "Would move" } else { "Moved" };
    writeln!(out, "{verb} {} item(s) out of Needs you.", r.total)?;
    for g in &r.groups {
        writeln!(
            out,
            "  {:>5}  {} / {} ({})",
            g.count,
            g.source,
            g.kind,
            g.rule.as_str()
        )?;
    }
    if r.dry_run && r.total > 0 {
        writeln!(
            out,
            "Run with --apply to move them. `hq inbox restore --since 1d` undoes it."
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_takes_minutes_hours_and_days() {
        assert_eq!(parse_since("30m").unwrap(), Duration::minutes(30));
        assert_eq!(parse_since("12h").unwrap(), Duration::hours(12));
        assert_eq!(parse_since("7d").unwrap(), Duration::days(7));
        assert!(parse_since("7").is_err());
        assert!(parse_since("0d").is_err());
        assert!(parse_since("soon").is_err());
        assert!(parse_since("5é").is_err(), "multi-byte unit must not panic");
        assert!(parse_since("é").is_err());
        assert!(parse_since("").is_err());
    }

    #[test]
    fn the_report_names_counts_and_never_text() {
        let r = TidyReport {
            dry_run: true,
            total: 2,
            groups: vec![hq_db::inbox_tidy::TidyGroup {
                source: "session-supervisor".into(),
                kind: "action_needed".into(),
                rule: hq_db::inbox_tidy::Rule::SessionEnded,
                count: 2,
            }],
        };
        let mut buf = Vec::new();
        render(&r, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("Would move 2 item(s)"));
        assert!(text.contains("session-supervisor / action_needed (session_ended)"));
        assert!(text.contains("--apply"));
    }
}
