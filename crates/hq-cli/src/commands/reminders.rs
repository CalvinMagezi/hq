//! `hq reminders`: set, list, finish and check the reminders HQ rings. The same rows the
//! `remind_*` tools and the Reminders screen use (`hq_db::reminders`).

use anyhow::{Context, Result, bail};
use chrono::{NaiveDateTime, Utc};
use chrono_tz::Tz;
use clap::Subcommand;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::reminders::{self, Limits, NewReminder, OUTCOME_DISMISSED, STATUS_ACTIVE};
use hq_tools::reminders::fire::when_words;
use hq_tools::reminders::schedule::{Repeat, after_done, resolve_wall_clock, zone_of};

/// A reminder this far past its time means the daemon is not ringing.
const OVERDUE_SECS: i64 = 300;

#[derive(Subcommand)]
pub enum RemindersAction {
    /// Set a reminder. Give a time with --at (local, `2026-11-27 16:00`) or --in-minutes.
    Add {
        title: String,
        /// Local date and time in the reminder's zone, `YYYY-MM-DD HH:MM`
        #[arg(long, conflicts_with = "in_minutes")]
        at: Option<String>,
        /// Minutes from now
        #[arg(long)]
        in_minutes: Option<i64>,
        /// IANA zone the time means. Defaults to the owner's zone preference.
        #[arg(long)]
        tz: Option<String>,
        /// Keep reminding until it is marked done
        #[arg(long)]
        nudge: bool,
        /// daily, every:N, weekdays, weekly:1,4 or monthly:15
        #[arg(long)]
        repeat: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Active reminders, soonest first
    List {
        /// Include finished and stopped ones
        #[arg(long)]
        all: bool,
    },
    /// Mark one done (a repeat moves to its next time)
    Done { id: String },
    /// Stop one
    Cancel { id: String },
    /// Check that reminders are ringing: lists any that are overdue
    Doctor,
}

pub fn run(config: &HqConfig, action: RemindersAction) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    let now = Utc::now().timestamp();
    match action {
        RemindersAction::Add {
            title,
            at,
            in_minutes,
            tz,
            nudge,
            repeat,
            note,
        } => {
            let zone: Tz = match tz {
                Some(z) => z
                    .parse()
                    .map_err(|_| anyhow::anyhow!("unknown zone '{z}'"))?,
                None => hq_db::owner_prefs::timezone(&db)
                    .context("no time zone: pass --tz, e.g. --tz Africa/Kampala")?,
            };
            let due = match (at, in_minutes) {
                (Some(text), None) => {
                    let naive = NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M")
                        .or_else(|_| NaiveDateTime::parse_from_str(&text, "%Y-%m-%dT%H:%M"))
                        .map_err(|_| anyhow::anyhow!("--at looks like 2026-11-27 16:00"))?;
                    resolve_wall_clock(zone, naive)
                }
                (None, Some(m)) if m >= 1 => now + m * 60,
                _ => bail!("give --at or --in-minutes (1 or more)"),
            };
            if due <= now {
                bail!("that time has already passed");
            }
            let rule = repeat
                .as_deref()
                .map(Repeat::from_spec)
                .transpose()?
                .map(|r| r.to_json());
            let r = reminders::create(
                &db,
                &NewReminder {
                    title: &title,
                    note: note.as_deref(),
                    due_at: due,
                    tz: zone.name(),
                    repeat_rule: rule.as_deref(),
                    nudge,
                    task_id: None,
                    thread_id: None,
                    tainted: false,
                    source: "cli",
                },
                now,
                Limits {
                    max_active: i64::from(config.reminders.max_active),
                    max_per_hour: i64::from(config.reminders.max_per_hour),
                },
            )?;
            println!(
                "Reminder {} set for {} ({}).",
                r.id,
                when_words(r.due_at, now, zone),
                zone.name()
            );
        }
        RemindersAction::List { all } => {
            let statuses: &[&str] = if all { &[] } else { &[STATUS_ACTIVE] };
            for r in reminders::list(&db, statuses, 200)? {
                let zone = zone_of(&r);
                let repeat = r
                    .repeat_rule
                    .as_deref()
                    .and_then(|j| Repeat::parse(j).ok())
                    .map(|rule| format!("  {}", rule.describe()))
                    .unwrap_or_default();
                println!(
                    "{}  {:<9}  {}  {}{}{}",
                    r.id,
                    r.status,
                    when_words(r.due_at, now, zone),
                    r.title,
                    if r.nudge { "  (nudge)" } else { "" },
                    repeat
                );
            }
        }
        RemindersAction::Done { id } => {
            let r = reminders::get(&db, &id)?.context("no such reminder")?;
            if reminders::acknowledge(&db, &r.id, now, after_done(&r, now))? {
                println!("Done: {}", r.title);
            } else {
                println!("That reminder is already {}.", r.status);
            }
        }
        RemindersAction::Cancel { id } => {
            let r = reminders::get(&db, &id)?.context("no such reminder")?;
            if reminders::cancel(&db, &r.id, now, OUTCOME_DISMISSED)? {
                println!("Stopped: {}", r.title);
            } else {
                println!("That reminder is already {}.", r.status);
            }
        }
        RemindersAction::Doctor => {
            let active = reminders::count_active(&db)?;
            let overdue: Vec<_> = reminders::due(&db, now - OVERDUE_SECS, 50)?;
            println!("{active} active reminder(s).");
            if !config.daemon.reminders {
                println!("daemon.reminders is off: nothing will ring.");
            }
            if overdue.is_empty() {
                println!("None are overdue. Reminders are ringing.");
            } else {
                println!(
                    "{} overdue by more than 5 minutes. Is the daemon (hq start) running?",
                    overdue.len()
                );
                for r in overdue {
                    println!(
                        "  {}  due {}  {}",
                        r.id,
                        when_words(r.due_at, now, zone_of(&r)),
                        r.title
                    );
                }
            }
        }
    }
    Ok(())
}
