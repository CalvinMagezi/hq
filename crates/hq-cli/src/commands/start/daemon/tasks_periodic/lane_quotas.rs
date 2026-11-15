//! Samples every host's lane quota readings into hq-db so the last good reading outlives a host
//! that goes away, and so pace has a history to come from.

use anyhow::Result;
use chrono::{Duration, Utc};
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_tools::lane_quotas::{self, Options};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tracing::{debug, warn};

const MIN_INTERVAL_MINUTES: i64 = 1;

/// Unix seconds before which the sampler does not ask again. Set after every attempt, whether or
/// not a host answered, so an unreachable host is tried once per interval and not once a minute.
static NEXT_AT: AtomicI64 = AtomicI64::new(0);

/// Runs every minute from the scheduler and samples once `quota.interval_minutes` has passed.
pub async fn run_lane_quotas(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.quota.enabled {
        return Ok(());
    }
    let now = Utc::now();
    if now.timestamp() < NEXT_AT.load(Ordering::Relaxed) {
        return Ok(());
    }
    let interval =
        Duration::minutes((config.quota.interval_minutes as i64).max(MIN_INTERVAL_MINUTES));
    NEXT_AT.store((now + interval).timestamp(), Ordering::Relaxed);

    let (db, cfg) = (Arc::new(db.clone()), config.clone());
    let opts = Options {
        refresh: true,
        ..Options::default()
    };
    let read = tokio::task::spawn_blocking(move || {
        let report = lane_quotas::read_all(&db, &cfg, &opts);
        let now = Utc::now();
        lane_quotas::prune(&db, now);
        let raised = report
            .as_ref()
            .map(|r| lane_quotas::alerts::raise(&db, r, now))
            .unwrap_or(0);
        lane_quotas::alerts::prune(&db, now);
        report.map(|r| (r, raised))
    })
    .await;
    match read {
        Ok(Ok((report, raised))) => {
            let silent = report.hosts.iter().filter(|h| !h.reachable).count();
            debug!(
                lanes = report.lanes.len(),
                unreachable_hosts = silent,
                alerts = raised,
                "lane quotas sampled"
            );
        }
        Ok(Err(e)) => warn!(error = %e, "lane-quotas: could not read the hosts"),
        Err(e) => warn!(error = %e, "lane-quotas: the reading was cancelled"),
    }
    Ok(())
}
