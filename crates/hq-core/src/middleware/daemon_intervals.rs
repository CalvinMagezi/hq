//! Default task timeouts derived from scheduler interval tiers.

use std::time::Duration;

/// Default timeout by tier: fast=30s, periodic=2min, scheduled=5min, slow=10min.
#[inline]
pub fn default_timeout_for_interval(interval: &Duration) -> Duration {
    let secs = interval.as_secs();
    if secs <= 60 {
        Duration::from_secs(30)
    } else if secs <= 3600 {
        Duration::from_secs(120)
    } else if secs <= 7200 {
        Duration::from_secs(300)
    } else {
        Duration::from_secs(600)
    }
}
