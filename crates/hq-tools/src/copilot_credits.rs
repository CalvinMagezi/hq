//! Copilot credit sampling and reporting, shared by the daemon sampler, the `copilot_credits`
//! tool and the web API.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use hq_db::Database;
use hq_db::copilot_usage_samples::{self as store, UsageSample};
use hq_llm::copilot_burn::{BurnReport, BurnSample, compute_burn};
use hq_llm::copilot_usage::{CopilotQuota, fetch_quota};

/// The longest burn window, so every window has its samples.
const HISTORY_HOURS: i64 = 24;

pub const NOT_METERED_NOTE: &str = "This token has no metered credit allowance (for example a personal free account), so it is probably not the Business seat HQ is meant to run on. No balance is shown.";

/// Store a sample when the quota is a real allowance. Returns whether one was written.
pub fn store_sample(db: &Database, q: &CopilotQuota) -> Result<bool> {
    if !q.is_metered() {
        return Ok(false);
    }
    let sample = UsageSample {
        ts: q.fetched_at,
        login: q.login.clone(),
        credits_used: q.credits_used,
        remaining: q.remaining,
        entitlement: q.entitlement,
        reset_at: q.reset_at,
    };
    db.with_conn(|c| {
        store::insert_sample(c, &sample)?;
        store::prune(c, q.fetched_at)?;
        Ok(())
    })?;
    Ok(true)
}

/// Fetch the balance now and store it. Shared by the periodic sampler and the tool.
pub async fn sample_now(db: &Database) -> Result<CopilotQuota> {
    let quota = fetch_quota().await?;
    store_sample(db, &quota)?;
    Ok(quota)
}

pub struct BurnView {
    pub burn: BurnReport,
    /// Evenly thinned points across the history window, for charts.
    pub points: Vec<UsageSample>,
}

pub fn burn_view(
    db: &Database,
    quota: &CopilotQuota,
    now: DateTime<Utc>,
    max_points: usize,
) -> Result<BurnView> {
    let samples =
        db.with_conn(|c| store::list_samples_since(c, now - Duration::hours(HISTORY_HOURS)))?;
    let burn_samples: Vec<BurnSample> = samples
        .iter()
        .map(|s| BurnSample {
            ts: s.ts,
            credits_used: s.credits_used,
        })
        .collect();
    Ok(BurnView {
        burn: compute_burn(&burn_samples, quota, now),
        points: thin(samples, max_points),
    })
}

fn thin(samples: Vec<UsageSample>, max: usize) -> Vec<UsageSample> {
    if max == 0 || samples.len() <= max {
        return samples;
    }
    let last = samples.len() - 1;
    (0..max)
        .map(|i| samples[i * last / (max - 1).max(1)].clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quota(metered: bool, used: f64, at: DateTime<Utc>) -> CopilotQuota {
        CopilotQuota {
            login: Some("seat".into()),
            plan: Some("business".into()),
            sku: None,
            token_based_billing: true,
            entitlement: if metered { 1000.0 } else { 0.0 },
            credits_used: used,
            remaining: 1000.0 - used,
            percent_remaining: 0.0,
            overage_permitted: false,
            unlimited: false,
            has_quota: metered,
            reset_at: Some(at + Duration::days(1)),
            fetched_at: at,
        }
    }

    #[test]
    fn only_metered_quotas_are_stored_and_feed_the_burn_view() {
        let db = Database::open_memory().unwrap();
        let now = Utc::now();
        assert!(!store_sample(&db, &quota(false, 0.0, now)).unwrap());
        for i in 0..4 {
            let at = now - Duration::minutes(90 - i * 30);
            assert!(store_sample(&db, &quota(true, 100.0 * i as f64, at)).unwrap());
        }
        let view = burn_view(&db, &quota(true, 300.0, now), now, 48).unwrap();
        assert_eq!(view.points.len(), 4);
        let one_hour = &view.burn.windows[0];
        assert!(one_hour.per_hour.unwrap() > 0.0);
    }

    #[test]
    fn thinning_keeps_first_and_last() {
        let now = Utc::now();
        let s: Vec<_> = (0..100)
            .map(|i| UsageSample {
                ts: now + Duration::minutes(i),
                login: None,
                credits_used: i as f64,
                remaining: 0.0,
                entitlement: 0.0,
                reset_at: None,
            })
            .collect();
        let t = thin(s, 48);
        assert_eq!(t.len(), 48);
        assert_eq!((t[0].credits_used, t[47].credits_used), (0.0, 99.0));
    }
}
