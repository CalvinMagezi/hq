//! Reads and housekeeping over the spend ledger (`task_outcomes` and its `usage_daily` rollup).
//! Shared by `hq usage`, reconciliation and, later, budgets and forecasts, so every reader
//! counts a call the same way.

use anyhow::Result;
use chrono::{Datelike, TimeZone, Utc};
use rusqlite::{Connection, params};
use serde::Serialize;

const SECS_PER_DAY: i64 = 86_400;

/// Which column the ledger is grouped by.
#[derive(Clone, Copy)]
pub enum GroupBy {
    Model,
    Provider,
    Origin,
    Day,
    DayAndModel,
}

impl GroupBy {
    fn key_sql(self) -> &'static str {
        match self {
            GroupBy::Model => "model",
            GroupBy::Provider => "provider",
            GroupBy::Origin => "origin",
            GroupBy::Day => "date(recorded_at, 'unixepoch')",
            GroupBy::DayAndModel => "date(recorded_at, 'unixepoch') || '  ' || model",
        }
    }
}

/// Calls, tokens and cost for one group. `unpriced_calls` are calls whose cost is unknown, not zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageRow {
    pub key: String,
    pub calls: i64,
    pub failed_calls: i64,
    pub unpriced_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cost_usd: f64,
}

impl UsageRow {
    /// Share of prompt tokens served from the provider's cache, when any prompt was sent.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        (self.input_tokens > 0).then(|| self.cache_read_tokens as f64 / self.input_tokens as f64)
    }
}

pub fn sum_rows(rows: &[UsageRow]) -> UsageRow {
    rows.iter().fold(UsageRow::default(), |acc, r| UsageRow {
        key: acc.key,
        calls: acc.calls + r.calls,
        failed_calls: acc.failed_calls + r.failed_calls,
        unpriced_calls: acc.unpriced_calls + r.unpriced_calls,
        input_tokens: acc.input_tokens + r.input_tokens,
        output_tokens: acc.output_tokens + r.output_tokens,
        cache_read_tokens: acc.cache_read_tokens + r.cache_read_tokens,
        cost_usd: acc.cost_usd + r.cost_usd,
    })
}

pub fn grouped_usage(conn: &Connection, since: i64, group: GroupBy) -> Result<Vec<UsageRow>> {
    let sql = format!(
        "SELECT {key} AS k, COUNT(*),
                COALESCE(SUM(success = 0), 0),
                COALESCE(SUM(cost_source = 'unpriced'), 0),
                COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(cache_read_tokens), 0), COALESCE(SUM(cost_usd), 0.0)
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
                failed_calls: row.get(2)?,
                unpriced_calls: row.get(3)?,
                input_tokens: row.get(4)?,
                output_tokens: row.get(5)?,
                cache_read_tokens: row.get(6)?,
                cost_usd: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Dollars recorded for one provider in `[since, until)`, plus how many calls had no known cost.
pub fn provider_spend(
    conn: &Connection,
    provider: &str,
    since: i64,
    until: i64,
) -> Result<(f64, i64)> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(cost_usd), 0.0), COALESCE(SUM(cost_source = 'unpriced'), 0)
         FROM task_outcomes
         WHERE provider = ?1 AND recorded_at >= ?2 AND recorded_at < ?3",
        params![provider, since, until],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// Start of the UTC day, Monday-based week and month containing `now`: the periods providers
/// such as OpenRouter and Anthropic reset on.
pub fn utc_period_starts(now: i64) -> [(&'static str, i64); 3] {
    let day = now - now.rem_euclid(SECS_PER_DAY);
    let dt = Utc.timestamp_opt(now, 0).single().unwrap_or_default();
    let week = day - i64::from(dt.weekday().num_days_from_monday()) * SECS_PER_DAY;
    let month = Utc
        .with_ymd_and_hms(dt.year(), dt.month(), 1, 0, 0, 0)
        .single()
        .map_or(day, |m| m.timestamp());
    [("today", day), ("this week", week), ("this month", month)]
}

/// What HQ's own ledger recorded for one provider, in dollars, over the current UTC periods.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct LedgerWindows {
    pub today: f64,
    pub week: f64,
    pub month: f64,
    /// Calls this month whose cost HQ could not price, so the figures are a lower bound.
    pub unpriced_calls: i64,
}

pub fn ledger_windows(conn: &Connection, provider: &str, now: i64) -> Result<LedgerWindows> {
    let [(_, day), (_, week), (_, month)] = utc_period_starts(now);
    let until = now + 1;
    let (today, _) = provider_spend(conn, provider, day, until)?;
    let (week_usd, _) = provider_spend(conn, provider, week, until)?;
    let (month_usd, unpriced) = provider_spend(conn, provider, month, until)?;
    Ok(LedgerWindows {
        today,
        week: week_usd,
        month: month_usd,
        unpriced_calls: unpriced,
    })
}

/// Models that ran with no known price, newest first, so a doctor can name them.
pub fn unpriced_models(conn: &Connection, since: i64) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT model, COUNT(*) FROM task_outcomes
         WHERE cost_source = 'unpriced' AND success = 1 AND recorded_at >= ?1
         GROUP BY model ORDER BY COUNT(*) DESC",
    )?;
    let rows = stmt
        .query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Fold raw rows older than `retain_days` into `usage_daily`, then delete them. One transaction,
/// so a crash cannot count a day twice or drop one. Returns the number of raw rows folded.
pub fn rollup_and_prune(conn: &Connection, now: i64, retain_days: i64) -> Result<usize> {
    let cutoff = now - retain_days * SECS_PER_DAY;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO usage_daily
            (day, provider, model, origin, calls, failed_calls, unpriced_calls, input_tokens,
             output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens, cost_usd)
         SELECT date(recorded_at, 'unixepoch'), provider, model, origin, COUNT(*),
                SUM(success = 0), SUM(cost_source = 'unpriced'),
                COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                SUM(cache_read_tokens), SUM(cache_write_tokens), SUM(reasoning_tokens),
                SUM(cost_usd)
         FROM task_outcomes WHERE recorded_at < ?1
         GROUP BY 1, 2, 3, 4
         ON CONFLICT(day, provider, model, origin) DO UPDATE SET
            calls = calls + excluded.calls,
            failed_calls = failed_calls + excluded.failed_calls,
            unpriced_calls = unpriced_calls + excluded.unpriced_calls,
            input_tokens = input_tokens + excluded.input_tokens,
            output_tokens = output_tokens + excluded.output_tokens,
            cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
            cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
            reasoning_tokens = reasoning_tokens + excluded.reasoning_tokens,
            cost_usd = cost_usd + excluded.cost_usd",
        params![cutoff],
    )?;
    let folded = tx.execute(
        "DELETE FROM task_outcomes WHERE recorded_at < ?1",
        params![cutoff],
    )?;
    tx.commit()?;
    Ok(folded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_outcomes::{TaskOutcome, insert};

    const NOW: i64 = 1_789_905_600;
    const RETAIN_DAYS: i64 = 90;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn
    }

    fn put(conn: &Connection, at: i64, source: &str, origin: &str, cost: f64, cache: i64) {
        let mut o = TaskOutcome::now("s", 0, "m", "openrouter", "chat");
        o.recorded_at = at;
        o.input_tokens = Some(1000);
        o.output_tokens = Some(10);
        o.cache_read_tokens = cache;
        o.cost_usd = cost;
        o.cost_source = source.into();
        o.origin = origin.into();
        insert(conn, &o).unwrap();
    }

    #[test]
    fn origin_grouping_counts_unpriced_calls_and_cache_hits() {
        let conn = db();
        put(&conn, NOW - 10, "table", "chat", 0.5, 800);
        put(&conn, NOW - 20, "unpriced", "memory", 0.0, 0);
        let rows = grouped_usage(&conn, NOW - 100, GroupBy::Origin).unwrap();
        let by_key = |k: &str| rows.iter().find(|r| r.key == k).unwrap().clone();
        assert_eq!(by_key("memory").unpriced_calls, 1);
        assert_eq!(by_key("chat").cache_hit_rate(), Some(0.8));
        assert_eq!(by_key("memory").cache_hit_rate(), Some(0.0));
    }

    #[test]
    fn rollup_keeps_totals_and_removes_only_old_raw_rows() {
        let conn = db();
        let old = NOW - (RETAIN_DAYS + 5) * SECS_PER_DAY;
        put(&conn, old, "table", "chat", 1.0, 0);
        put(&conn, old + 60, "table", "chat", 2.0, 0);
        put(&conn, NOW - 60, "table", "chat", 4.0, 0);

        assert_eq!(rollup_and_prune(&conn, NOW, RETAIN_DAYS).unwrap(), 2);
        assert_eq!(rollup_and_prune(&conn, NOW, RETAIN_DAYS).unwrap(), 0);

        let rolled: (i64, f64) = conn
            .query_row(
                "SELECT SUM(calls), SUM(cost_usd) FROM usage_daily",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(rolled, (2, 3.0));
        let raw = grouped_usage(&conn, 0, GroupBy::Origin).unwrap();
        assert_eq!(sum_rows(&raw).cost_usd, 4.0);
    }

    #[test]
    fn ledger_windows_split_one_provider_into_utc_periods() {
        let conn = db();
        // NOW is Sunday 2026-09-20 12:00 UTC: the week began Monday the 14th, the month on the 1st.
        put(&conn, NOW - 60, "table", "chat", 1.0, 0);
        put(&conn, NOW - 2 * SECS_PER_DAY, "unpriced", "chat", 0.0, 0);
        put(&conn, NOW - 10 * SECS_PER_DAY, "table", "chat", 4.0, 0);
        put(&conn, NOW - 30 * SECS_PER_DAY, "table", "chat", 8.0, 0);
        let w = ledger_windows(&conn, "openrouter", NOW).unwrap();
        assert_eq!((w.today, w.week, w.month, w.unpriced_calls), (1.0, 1.0, 5.0, 1));
        assert_eq!(ledger_windows(&conn, "nobody", NOW).unwrap(), LedgerWindows::default());
    }

    #[test]
    fn provider_spend_is_bounded_by_the_window() {
        let conn = db();
        put(&conn, NOW - 10, "table", "chat", 0.25, 0);
        put(&conn, NOW - 5 * SECS_PER_DAY, "table", "chat", 9.0, 0);
        let (usd, unpriced) = provider_spend(&conn, "openrouter", NOW - SECS_PER_DAY, NOW).unwrap();
        assert_eq!((usd, unpriced), (0.25, 0));
    }
}
