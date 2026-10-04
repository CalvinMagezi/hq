//! `hq usage`, `hq cost` and `hq summary`: LLM spend read from hq-db
//! `task_outcomes`, the one row per LLM call that hq-agent's outcome sink writes.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use rusqlite::{Connection, params};
use std::io::Write;

const SECS_PER_DAY: i64 = 86_400;
const SUMMARY_WINDOW_DAYS: i64 = 30;
const DAILY_WINDOW_DAYS: i64 = 7;
const TOKENS_PER_MILLION: f64 = 1_000_000.0;
const TOKENS_PER_THOUSAND: f64 = 1_000.0;

/// Which column `task_outcomes` rows are grouped by.
#[derive(Clone, Copy)]
enum GroupBy {
    Model,
    Day,
    DayAndModel,
}

impl GroupBy {
    fn key_sql(self) -> &'static str {
        match self {
            GroupBy::Model => "model",
            GroupBy::Day => "date(recorded_at, 'unixepoch')",
            GroupBy::DayAndModel => "date(recorded_at, 'unixepoch') || '  ' || model",
        }
    }
}

/// Aggregated calls, tokens and cost for one group of `task_outcomes` rows.
#[derive(Debug, Clone, Default, PartialEq)]
struct UsageRow {
    key: String,
    calls: i64,
    input_tokens: i64,
    output_tokens: i64,
    cost_usd: f64,
}

pub async fn run(config: &HqConfig, sub: &str) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    render(&db, sub, now_epoch(), &mut std::io::stdout())
}

fn render(db: &Database, sub: &str, now: i64, out: &mut dyn Write) -> Result<()> {
    match sub {
        "summary" | "cost" | "" => render_summary(db, now, out),
        "daily" | "day" => render_daily(db, now, out),
        _ => render_help(out),
    }
}

fn render_help(out: &mut dyn Write) -> Result<()> {
    writeln!(out, "Usage: hq usage [summary|daily]\n")?;
    writeln!(
        out,
        "  summary   Cost and tokens for the last {SUMMARY_WINDOW_DAYS} days, by model (default, same as `hq cost`)"
    )?;
    writeln!(
        out,
        "  daily     Cost and tokens for the last {DAILY_WINDOW_DAYS} days, by day and model (same as `hq summary`)"
    )?;
    Ok(())
}

fn render_summary(db: &Database, now: i64, out: &mut dyn Write) -> Result<()> {
    let since = now - SUMMARY_WINDOW_DAYS * SECS_PER_DAY;
    let by_model = db.with_conn(|conn| grouped_usage(conn, since, GroupBy::Model))?;
    writeln!(out, "LLM usage, last {SUMMARY_WINDOW_DAYS} days\n")?;
    if by_model.is_empty() {
        return write_no_data(out);
    }
    let total = sum_rows(&by_model);
    writeln!(out, "Calls:          {}", total.calls)?;
    writeln!(out, "Input tokens:   {}", format_tokens(total.input_tokens))?;
    writeln!(
        out,
        "Output tokens:  {}",
        format_tokens(total.output_tokens)
    )?;
    writeln!(out, "Cost:           ${:.4}\n", total.cost_usd)?;
    writeln!(out, "By model:")?;
    write_rows(out, &by_model)?;

    let by_day = db.with_conn(|conn| grouped_usage(conn, since, GroupBy::Day))?;
    writeln!(out, "\nBy day:")?;
    write_rows(out, &by_day)
}

fn render_daily(db: &Database, now: i64, out: &mut dyn Write) -> Result<()> {
    let since = now - DAILY_WINDOW_DAYS * SECS_PER_DAY;
    let rows = db.with_conn(|conn| grouped_usage(conn, since, GroupBy::DayAndModel))?;
    writeln!(
        out,
        "LLM usage by day and model, last {DAILY_WINDOW_DAYS} days\n"
    )?;
    if rows.is_empty() {
        return write_no_data(out);
    }
    write_rows(out, &rows)
}

fn write_no_data(out: &mut dyn Write) -> Result<()> {
    writeln!(out, "No LLM calls recorded in this window.")?;
    writeln!(
        out,
        "Calls are recorded in the task_outcomes table of _data/vault.db."
    )?;
    Ok(())
}

fn write_rows(out: &mut dyn Write, rows: &[UsageRow]) -> Result<()> {
    for row in rows {
        writeln!(
            out,
            "  {}  {} calls, {} in / {} out, ${:.4}",
            row.key,
            row.calls,
            format_tokens(row.input_tokens),
            format_tokens(row.output_tokens),
            row.cost_usd,
        )?;
    }
    Ok(())
}

fn grouped_usage(conn: &Connection, since: i64, group: GroupBy) -> Result<Vec<UsageRow>> {
    let sql = format!(
        "SELECT {key} AS k, COUNT(*), COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0), COALESCE(SUM(cost_usd), 0.0)
         FROM task_outcomes
         WHERE recorded_at >= ?1
         GROUP BY k
         ORDER BY k",
        key = group.key_sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![since], |row| {
            Ok(UsageRow {
                key: row.get(0)?,
                calls: row.get(1)?,
                input_tokens: row.get(2)?,
                output_tokens: row.get(3)?,
                cost_usd: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn sum_rows(rows: &[UsageRow]) -> UsageRow {
    rows.iter().fold(UsageRow::default(), |acc, r| UsageRow {
        key: acc.key,
        calls: acc.calls + r.calls,
        input_tokens: acc.input_tokens + r.input_tokens,
        output_tokens: acc.output_tokens + r.output_tokens,
        cost_usd: acc.cost_usd + r.cost_usd,
    })
}

fn now_epoch() -> i64 {
    chrono::Utc::now().timestamp()
}

fn format_tokens(tokens: i64) -> String {
    let t = tokens as f64;
    if t >= TOKENS_PER_MILLION {
        format!("{:.1}M", t / TOKENS_PER_MILLION)
    } else if t >= TOKENS_PER_THOUSAND {
        format!("{:.1}K", t / TOKENS_PER_THOUSAND)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::task_outcomes::{TaskOutcome, insert};

    // 2026-09-20 12:00:00 UTC, so day keys are deterministic.
    const NOW: i64 = 1_789_905_600;

    fn record(db: &Database, model: &str, at: i64, input: i64, output: i64, cost: f64) {
        let mut o = TaskOutcome::now("s1", 0, model, "backends", "chat");
        o.recorded_at = at;
        o.input_tokens = Some(input);
        o.output_tokens = Some(output);
        o.cost_usd = cost;
        db.with_conn(|conn| insert(conn, &o)).unwrap();
    }

    fn seeded() -> Database {
        let db = Database::open_memory().unwrap();
        record(&db, "model-a", NOW - 60, 1_000, 200, 0.01);
        record(&db, "model-a", NOW - SECS_PER_DAY, 500, 100, 0.02);
        record(&db, "model-b", NOW - 120, 3_000, 0, 0.5);
        // Outside every window, so it must never be counted.
        record(&db, "model-old", NOW - 90 * SECS_PER_DAY, 9, 9, 9.0);
        db
    }

    fn rows(db: &Database, since: i64, group: GroupBy) -> Vec<UsageRow> {
        db.with_conn(|conn| grouped_usage(conn, since, group))
            .unwrap()
    }

    #[test]
    fn groups_task_outcomes_by_model_and_day() {
        let db = seeded();
        let since = NOW - SUMMARY_WINDOW_DAYS * SECS_PER_DAY;

        let by_model = rows(&db, since, GroupBy::Model);
        let keys: Vec<&str> = by_model.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["model-a", "model-b"]);
        assert_eq!(by_model[0].calls, 2);
        assert_eq!(by_model[0].input_tokens, 1_500);
        assert!((by_model[0].cost_usd - 0.03).abs() < 1e-9);

        let by_day = rows(&db, since, GroupBy::Day);
        let days: Vec<&str> = by_day.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(days, ["2026-09-19", "2026-09-20"]);
        assert_eq!(by_day[1].calls, 2);

        let total = sum_rows(&by_model);
        assert_eq!(total.calls, 3);
        assert!((total.cost_usd - 0.53).abs() < 1e-9);
    }

    #[test]
    fn summary_and_daily_render_from_task_outcomes() {
        let db = seeded();
        let mut out = Vec::new();
        render(&db, "summary", NOW, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("Calls:          3"), "{text}");
        assert!(text.contains("$0.5300"), "{text}");
        assert!(!text.contains("model-old"), "{text}");

        let mut out = Vec::new();
        render(&db, "daily", NOW, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("2026-09-20  model-b"), "{text}");
        assert!(text.contains("2026-09-19  model-a"), "{text}");
    }

    #[test]
    fn empty_table_says_so() {
        let db = Database::open_memory().unwrap();
        let mut out = Vec::new();
        render(&db, "", NOW, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("No LLM calls recorded"), "{text}");
    }
}
