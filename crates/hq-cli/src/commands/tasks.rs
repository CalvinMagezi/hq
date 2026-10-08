//! `hq task time [days]`: where the time went, read from work leases and the
//! task event log. Tasks that never recorded a start are counted, not guessed.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::tasks::{TimeReport, time_report};
use std::io::Write;

const DEFAULT_WINDOW_DAYS: i64 = 30;
const MAX_WINDOW_DAYS: i64 = 365;
const SECS_PER_HOUR: f64 = 3600.0;

pub async fn run(config: &HqConfig, sub: &str, days: Option<i64>) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    let ttl = i64::try_from(config.tasks.lease_ttl()).unwrap_or(i64::MAX);
    render(&db, sub, days, ttl, &mut std::io::stdout())
}

fn render(db: &Database, sub: &str, days: Option<i64>, ttl: i64, out: &mut dyn Write) -> Result<()> {
    match sub {
        "time" | "" => {
            let days = days.unwrap_or(DEFAULT_WINDOW_DAYS).clamp(1, MAX_WINDOW_DAYS);
            let report = db.with_conn(|c| time_report(c, days, ttl))?;
            write_report(out, days, &report)
        }
        _ => {
            writeln!(out, "Usage: hq task time [days]\n")?;
            writeln!(out, "  time   Leased hours, cycle time and estimate accuracy by initiative and agent")?;
            Ok(())
        }
    }
}

fn hours(seconds: i64) -> String {
    format!("{:.1}h", seconds as f64 / SECS_PER_HOUR)
}

fn write_report(out: &mut dyn Write, days: i64, report: &TimeReport) -> Result<()> {
    writeln!(out, "Task time, last {days} days (UTC)\n")?;
    if report.initiatives.is_empty() && report.actors.is_empty() {
        writeln!(out, "No work leases or completions in this window.")?;
        writeln!(out, "Time is recorded when an agent calls task_claim, or when an HQ session works a task.")?;
        return Ok(());
    }
    writeln!(out, "By initiative:")?;
    for i in &report.initiatives {
        let cycle = i.mean_cycle_seconds.map_or("unknown".to_string(), hours);
        let accuracy = i
            .mean_actual_over_estimate
            .map_or("no estimates".to_string(), |r| format!("{r:.2}x of estimate over {} tasks", i.estimated_completed));
        writeln!(
            out,
            "  {:<32} {:>8} worked, {} completed, mean cycle {cycle}, {accuracy}, {} with no recorded start",
            i.name,
            hours(i.leased_seconds),
            i.tasks_completed,
            i.unknown_tasks
        )?;
    }
    writeln!(out, "\nBy agent:")?;
    for a in &report.actors {
        writeln!(out, "  {:<32} {:>8} over {} sessions", a.actor, hours(a.leased_seconds), a.sessions)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(db: &Database, sub: &str, days: Option<i64>) -> String {
        let mut buf = Vec::new();
        render(db, sub, days, 900, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn an_empty_vault_says_how_time_gets_recorded() {
        let db = Database::open_memory().unwrap();
        let out = text(&db, "time", None);
        assert!(out.contains("No work leases"), "{out}");
        assert!(out.contains("task_claim"), "{out}");
    }

    #[test]
    fn a_worked_task_shows_up_by_initiative_and_agent() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            hq_db::tasks::create_initiative(c, "in-1", "personal", None, "Agent HQ", "agent-hq", "FR")?;
            hq_db::tasks::create_task(c, "tk-1", "in-1", &hq_db::tasks::NewTask { title: "t", created_by: "x", ..Default::default() })?;
            let who = hq_db::tasks::LeaseIdentity { actor: "builder", ..Default::default() };
            hq_db::tasks::claim(c, "tk-1", &who, 900, false).map(|_| ())
        })
        .unwrap();
        let out = text(&db, "time", Some(7));
        assert!(out.contains("Agent HQ") && out.contains("builder"), "{out}");
        assert!(out.contains("last 7 days"), "{out}");
    }

    #[test]
    fn an_unknown_subcommand_prints_usage() {
        let db = Database::open_memory().unwrap();
        assert!(text(&db, "wat", None).starts_with("Usage: hq task time"));
    }
}
