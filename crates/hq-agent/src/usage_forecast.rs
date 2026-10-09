//! Spend forecasts from the ledger, for the CLI, the web API and later the agent itself.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use hq_core::config::{BudgetPeriod, BudgetScope, BudgetsConfig};
use hq_db::Database;
use hq_db::usage_ledger::{GroupBy, UsageRow, period_bounds, scope_spend, spend_by_hour, top_drivers};
use hq_llm::forecast::{Forecast, SpendEvent, forecast};
use serde::Serialize;

/// Enough history to learn a weekly pattern (it needs 14 days) with room to spare.
const HISTORY_DAYS: i64 = 28;
const SECS_PER_DAY: i64 = 86_400;
const SECS_PER_HOUR: i64 = 3_600;
/// What explains the burn: the last week, top few.
const DRIVER_WINDOW_DAYS: i64 = 7;
const DRIVER_COUNT: usize = 3;

fn utc(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_default()
}

/// Forecast one scope over one period. `limit_usd` is the budget the spend is counting against.
pub fn forecast_scope(
    db: &Database,
    scope: &BudgetScope,
    period: BudgetPeriod,
    limit_usd: Option<f64>,
    now: i64,
) -> Result<Forecast> {
    let (start, end) = period_bounds(period, now);
    let (spent, hourly) = db.with_conn(|conn| {
        let spent = scope_spend(conn, scope, start, end)?;
        let hourly = spend_by_hour(conn, scope, now - HISTORY_DAYS * SECS_PER_DAY, now + 1)?;
        Ok((spent, hourly))
    })?;
    // A bucket is dated at its end, clipped to now, so the current hour counts in the last hour.
    let events: Vec<SpendEvent> = hourly
        .into_iter()
        .map(|(hour, usd)| SpendEvent {
            ts: utc((hour + SECS_PER_HOUR).min(now)),
            usd,
        })
        .collect();
    Ok(forecast(&events, spent, utc(end), limit_usd, utc(now)))
}

#[derive(Debug, Clone, Serialize)]
pub struct Driver {
    pub name: String,
    pub usd: f64,
    pub calls: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Drivers {
    pub models: Vec<Driver>,
    pub origins: Vec<Driver>,
}

fn drivers_of(rows: Vec<UsageRow>) -> Vec<Driver> {
    rows.into_iter()
        .map(|r| Driver {
            name: r.key,
            usd: r.cost_usd,
            calls: r.calls,
        })
        .collect()
}

pub fn drivers(db: &Database, now: i64) -> Result<Drivers> {
    let since = now - DRIVER_WINDOW_DAYS * SECS_PER_DAY;
    db.with_conn(|conn| {
        Ok(Drivers {
            models: drivers_of(top_drivers(conn, GroupBy::Model, since, DRIVER_COUNT)?),
            origins: drivers_of(top_drivers(conn, GroupBy::Origin, since, DRIVER_COUNT)?),
        })
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedForecast {
    pub budget: String,
    pub scope: String,
    pub forecast: Forecast,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForecastReport {
    /// All spend this UTC month, against the tightest global monthly budget if there is one.
    pub month: Forecast,
    pub budgets: Vec<NamedForecast>,
    pub drivers: Drivers,
}

pub fn forecast_report(db: &Database, budgets: &BudgetsConfig, now: i64) -> Result<ForecastReport> {
    let enforceable = budgets.enforceable();
    let month_limit = enforceable
        .iter()
        .filter(|b| b.scope == BudgetScope::Global && b.period == BudgetPeriod::Month)
        .map(|b| b.limit_usd)
        .min_by(f64::total_cmp);
    let month = forecast_scope(db, &BudgetScope::Global, BudgetPeriod::Month, month_limit, now)?;
    let per_budget = enforceable
        .into_iter()
        .map(|b| {
            Ok(NamedForecast {
                budget: b.name.clone(),
                scope: b.scope.to_string(),
                forecast: forecast_scope(db, &b.scope, b.period, Some(b.limit_usd), now)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ForecastReport {
        month,
        budgets: per_budget,
        drivers: drivers(db, now)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::task_outcomes::{TaskOutcome, insert};

    // 2026-09-20 12:00:00 UTC, a Sunday.
    const NOW: i64 = 1_789_905_600;

    fn spend(db: &Database, at: i64, model: &str, origin: &str, usd: f64) {
        let mut o = TaskOutcome::now("s", 0, model, "haiku", "chat");
        o.recorded_at = at;
        o.cost_usd = usd;
        o.origin = origin.into();
        db.with_conn(|c| insert(c, &o)).unwrap();
    }

    #[test]
    fn a_steady_burn_projects_the_month_and_the_day_a_budget_runs_out() {
        let db = Database::open_memory().unwrap();
        for h in 0..48 {
            spend(&db, NOW - h * SECS_PER_HOUR - 60, "m", "chat", 0.5);
        }
        let cfg: BudgetsConfig = serde_yaml::from_str(
            "budgets:\n  - {name: month, scope: global, period: month, limit_usd: 40}\n",
        )
        .unwrap();
        let report = forecast_report(&db, &cfg, NOW).unwrap();
        assert!((report.month.rate_per_hour.unwrap() - 0.5).abs() < 0.05, "{:?}", report.month.rate_per_hour);
        assert!(report.month.exhausts_before_reset);
        assert_eq!(report.budgets[0].budget, "month");
        assert_eq!(report.month.limit_usd, Some(40.0));
    }

    #[test]
    fn drivers_name_the_biggest_spenders_first() {
        let db = Database::open_memory().unwrap();
        spend(&db, NOW - 60, "cheap", "memory", 0.1);
        spend(&db, NOW - 120, "pricey", "chat", 5.0);
        spend(&db, NOW - 180, "pricey", "chat", 3.0);
        let d = drivers(&db, NOW).unwrap();
        assert_eq!((d.models[0].name.as_str(), d.models[0].calls), ("pricey", 2));
        assert_eq!(d.origins[0].name, "chat");
        assert!((d.origins[0].usd - 8.0).abs() < 1e-9);
    }

    #[test]
    fn an_empty_ledger_forecasts_nothing_rather_than_zero() {
        let db = Database::open_memory().unwrap();
        let r = forecast_report(&db, &BudgetsConfig::default(), NOW).unwrap();
        assert!(r.month.rate_per_hour.is_none() && r.month.projected_period_total.is_none());
        assert!(r.budgets.is_empty() && r.drivers.models.is_empty());
    }
}
