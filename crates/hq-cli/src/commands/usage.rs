//! `hq usage`, `hq cost` and `hq summary`: LLM spend read from hq-db
//! `task_outcomes`, the one row per LLM call that hq-agent's outcome sink writes.

use anyhow::Result;
use hq_core::config::{HqConfig, openrouter_key, openrouter_primary};
use hq_db::Database;
use hq_db::usage_ledger::{
    GroupBy, UsageRow, grouped_usage, provider_spend, sum_rows, unpriced_models, utc_period_starts,
};
use hq_llm::openrouter_usage::{OpenRouterUsage, fetch_usage};
use hq_llm::reconcile::{Verdict, WindowDrift, compare};
use std::io::Write;

const SECS_PER_DAY: i64 = 86_400;
const SUMMARY_WINDOW_DAYS: i64 = 30;
const DAILY_WINDOW_DAYS: i64 = 7;
const TOKENS_PER_MILLION: f64 = 1_000_000.0;
const TOKENS_PER_THOUSAND: f64 = 1_000.0;
const DEFAULT_OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
const OPENROUTER_HOST: &str = "openrouter.ai";
/// How many unpriced models the summary names before saying "and more".
const UNPRICED_LIST_MAX: usize = 5;

pub async fn run(config: &HqConfig, sub: &str) -> Result<()> {
    let db = Database::open(&config.db_path())?;
    if sub == "reconcile" {
        return render_reconcile(config, &db, now_epoch(), &mut std::io::stdout()).await;
    }
    render(&db, sub, now_epoch(), &mut std::io::stdout())
}

fn render(db: &Database, sub: &str, now: i64, out: &mut dyn Write) -> Result<()> {
    match sub {
        "summary" | "cost" | "" => render_summary(db, now, out),
        "daily" | "day" => render_daily(db, now, out),
        "origin" | "origins" => render_origins(db, now, out),
        _ => render_help(out),
    }
}

fn render_help(out: &mut dyn Write) -> Result<()> {
    writeln!(out, "Usage: hq usage [summary|daily|origin|reconcile]\n")?;
    writeln!(
        out,
        "  summary   Cost and tokens for the last {SUMMARY_WINDOW_DAYS} days, by model (default, same as `hq cost`)"
    )?;
    writeln!(
        out,
        "  daily     Cost and tokens for the last {DAILY_WINDOW_DAYS} days, by day and model (same as `hq summary`)"
    )?;
    writeln!(
        out,
        "  origin    Cost by kind of work (chat, memory, subagent...) over the same window"
    )?;
    writeln!(
        out,
        "  reconcile Compare the ledger with what OpenRouter says it billed"
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
    writeln!(out, "Cost:           ${:.4}", total.cost_usd)?;
    if let Some(rate) = total.cache_hit_rate() {
        writeln!(out, "Cache hit rate: {:.0}% of prompt tokens", rate * 100.0)?;
    }
    writeln!(out)?;
    write_unpriced_warning(db, since, &total, out)?;
    writeln!(out, "By model:")?;
    write_rows(out, &by_model)?;
    let by_origin = db.with_conn(|conn| grouped_usage(conn, since, GroupBy::Origin))?;
    writeln!(out, "\nBy origin:")?;
    write_rows(out, &by_origin)?;

    let by_day = db.with_conn(|conn| grouped_usage(conn, since, GroupBy::Day))?;
    writeln!(out, "\nBy day:")?;
    write_rows(out, &by_day)
}

fn render_origins(db: &Database, now: i64, out: &mut dyn Write) -> Result<()> {
    let since = now - SUMMARY_WINDOW_DAYS * SECS_PER_DAY;
    let rows = db.with_conn(|conn| grouped_usage(conn, since, GroupBy::Origin))?;
    writeln!(
        out,
        "LLM usage by origin, last {SUMMARY_WINDOW_DAYS} days\n"
    )?;
    if rows.is_empty() {
        return write_no_data(out);
    }
    write_rows(out, &rows)
}

/// Said loudly because an unpriced call is an unknown cost, and the total is then a lower bound.
fn write_unpriced_warning(
    db: &Database,
    since: i64,
    total: &UsageRow,
    out: &mut dyn Write,
) -> Result<()> {
    if total.unpriced_calls == 0 {
        return Ok(());
    }
    writeln!(
        out,
        "Warning: {} calls have no known price, so the cost above is a lower bound.",
        total.unpriced_calls
    )?;
    let models = db.with_conn(|conn| unpriced_models(conn, since))?;
    for (model, calls) in models.iter().take(UNPRICED_LIST_MAX) {
        writeln!(out, "  unpriced: {model} ({calls} calls)")?;
    }
    if models.len() > UNPRICED_LIST_MAX {
        writeln!(
            out,
            "  and {} more models",
            models.len() - UNPRICED_LIST_MAX
        )?;
    }
    writeln!(out)?;
    Ok(())
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
        let unpriced = match row.unpriced_calls {
            0 => String::new(),
            n => format!(" ({n} unpriced)"),
        };
        writeln!(
            out,
            "  {}  {} calls, {} in / {} out, ${:.4}{unpriced}",
            row.key,
            row.calls,
            format_tokens(row.input_tokens),
            format_tokens(row.output_tokens),
            row.cost_usd,
        )?;
    }
    Ok(())
}

/// Ledger backends that spend on the OpenRouter key: the primary and any backend on the same
/// endpoint and credential, plus the plain `openrouter` provider name used without a chain.
fn openrouter_ledger_names(config: &HqConfig) -> Vec<String> {
    let primary = openrouter_primary(config);
    let mut names = vec!["openrouter".to_string()];
    for b in config.backends.backends.iter().filter(|b| b.enabled) {
        let on_openrouter = b
            .resolved_endpoint()
            .is_some_and(|e| e.contains(OPENROUTER_HOST));
        let same_key = primary.is_some_and(|p| p.credential_env == b.credential_env);
        if on_openrouter && same_key {
            names.push(b.name.clone());
        }
    }
    names
}

fn reconcile_windows(
    db: &Database,
    names: &[String],
    now: i64,
    usage: &OpenRouterUsage,
) -> Result<Vec<WindowDrift>> {
    let provider = [usage.usage_daily, usage.usage_weekly, usage.usage_monthly];
    utc_period_starts(now)
        .into_iter()
        .zip(provider)
        .map(|((label, since), billed)| {
            let (usd, unpriced) = db.with_conn(|conn| {
                names.iter().try_fold((0.0, 0), |(usd, unpriced), name| {
                    let (u, n) = provider_spend(conn, name, since, now + 1)?;
                    Ok((usd + u, unpriced + n))
                })
            })?;
            Ok(compare(label, usd, unpriced, billed))
        })
        .collect()
}

fn verdict_text(d: &WindowDrift) -> &'static str {
    match d.verdict {
        Verdict::Aligned => "matches",
        Verdict::LedgerLow => "ledger is LOW: calls are missing or priced too low",
        Verdict::LedgerHigh => "ledger is HIGH: prices too high or calls double counted",
        Verdict::ProviderUnknown => "OpenRouter gave no figure",
    }
}

/// The ledger-vs-billing windows for the OpenRouter key, or `None` when OpenRouter is not in use.
pub(crate) async fn drift_against_openrouter(
    config: &HqConfig,
    db: &Database,
    now: i64,
) -> Result<Option<Vec<WindowDrift>>> {
    let Some(backend) = openrouter_primary(config) else {
        return Ok(None);
    };
    let Some(key) = openrouter_key(config, backend) else {
        return Ok(None);
    };
    let base = backend
        .resolved_endpoint()
        .unwrap_or_else(|| DEFAULT_OPENROUTER_BASE.into());
    let usage = fetch_usage(&base, &key).await?;
    reconcile_windows(db, &openrouter_ledger_names(config), now, &usage).map(Some)
}

async fn render_reconcile(
    config: &HqConfig,
    db: &Database,
    now: i64,
    out: &mut dyn Write,
) -> Result<()> {
    let Some(backend) = openrouter_primary(config) else {
        writeln!(
            out,
            "OpenRouter is not the primary backend, so there is nothing to reconcile."
        )?;
        return Ok(());
    };
    let Some(key) = openrouter_key(config, backend) else {
        writeln!(
            out,
            "The OpenRouter key is not set, so its billed spend cannot be read."
        )?;
        return Ok(());
    };
    let base = backend
        .resolved_endpoint()
        .unwrap_or_else(|| DEFAULT_OPENROUTER_BASE.into());
    let usage = fetch_usage(&base, &key).await?;
    let names = openrouter_ledger_names(config);
    writeln!(out, "Ledger vs OpenRouter billing (UTC periods)\n")?;
    for d in reconcile_windows(db, &names, now, &usage)? {
        let billed = d
            .provider_usd
            .map_or("n/a".to_string(), |p| format!("${p:.4}"));
        let drift = d
            .drift_pct
            .map_or(String::new(), |p| format!(" ({p:+.1}%)"));
        let unpriced = match d.unpriced_calls {
            0 => String::new(),
            n => format!(", {n} unpriced calls"),
        };
        writeln!(
            out,
            "  {:<11} ledger ${:.4}  billed {billed}{drift}  {}{unpriced}",
            d.window,
            d.ledger_usd,
            verdict_text(&d)
        )?;
    }
    writeln!(
        out,
        "\nThe key may also be used outside this HQ, which shows up as a LOW ledger."
    )?;
    Ok(())
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
    use hq_db::usage_ledger::GroupBy;

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

    fn usage_with(daily: f64, weekly: f64, monthly: f64) -> OpenRouterUsage {
        OpenRouterUsage {
            usage: monthly,
            usage_daily: Some(daily),
            usage_weekly: Some(weekly),
            usage_monthly: Some(monthly),
            limit: None,
            limit_remaining: None,
            limit_reset: None,
            is_free_tier: false,
            total_credits: None,
            total_usage: None,
        }
    }

    fn record_for(db: &Database, provider: &str, at: i64, cost: f64, source: &str) {
        let mut o = TaskOutcome::now("s", 0, "m", provider, "chat");
        o.recorded_at = at;
        o.cost_usd = cost;
        o.cost_source = source.into();
        o.origin = "chat".into();
        db.with_conn(|conn| insert(conn, &o)).unwrap();
    }

    #[test]
    fn reconcile_counts_only_the_openrouter_backends_inside_each_period() {
        let db = Database::open_memory().unwrap();
        let names = vec!["openrouter".to_string(), "haiku".to_string()];
        record_for(&db, "haiku", NOW - 60, 1.0, "table");
        record_for(&db, "openrouter", NOW - 120, 0.5, "table");
        record_for(&db, "deepseek", NOW - 60, 9.0, "table");
        // Earlier this week, so it counts for the week and month but not today.
        record_for(&db, "haiku", NOW - 2 * SECS_PER_DAY, 2.0, "unpriced");

        let drift = reconcile_windows(&db, &names, NOW, &usage_with(1.5, 3.5, 3.5)).unwrap();
        let labels: Vec<&str> = drift.iter().map(|d| d.window.as_str()).collect();
        assert_eq!(labels, ["today", "this week", "this month"]);
        assert_eq!(drift[0].ledger_usd, 1.5);
        assert_eq!(drift[0].verdict, Verdict::Aligned);
        assert_eq!(drift[1].ledger_usd, 3.5);
        assert_eq!(drift[1].unpriced_calls, 1);
    }

    #[test]
    fn periods_start_at_utc_midnight_monday_and_the_first() {
        // 2026-09-20 is a Sunday, so the week began on Monday 2026-09-14.
        let [(_, day), (_, week), (_, month)] = utc_period_starts(NOW);
        assert_eq!(day, NOW - 12 * 3600);
        assert_eq!(week, day - 6 * SECS_PER_DAY);
        assert_eq!(month, day - 19 * SECS_PER_DAY);
    }

    #[test]
    fn the_summary_warns_when_calls_have_no_known_price() {
        let db = Database::open_memory().unwrap();
        record_for(&db, "openrouter", NOW - 60, 0.0, "unpriced");
        record_for(&db, "openrouter", NOW - 30, 0.1, "table");
        let mut out = Vec::new();
        render(&db, "summary", NOW, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("1 calls have no known price"), "{text}");
        assert!(text.contains("By origin:"), "{text}");
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
