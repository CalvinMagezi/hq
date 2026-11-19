//! Choosing the first lane of a start from the stored quota readings.
//!
//! A lane at its block line is skipped for the next lane of its `fallback` list. When every lane
//! is at the line, the start goes ahead on the lane the caller asked for with a warning: the
//! vendor is the one to refuse, and HQ must not strand an owner who chose the lane.

use chrono::{DateTime, Utc};
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::lane_quota_samples as store;
use serde_json::{Value, json};

use crate::lane_gate::{self as gate, Verdict};

/// The lanes a start may use, in order, and what the stored readings say about each.
pub(super) struct Plan {
    pub chain: Vec<String>,
    verdicts: Vec<Verdict>,
    /// Where in `chain` the start begins.
    pub begin: usize,
}

impl Plan {
    pub fn new(
        db: &Database,
        cfg: &HqConfig,
        host: &str,
        harness: &str,
        now: DateTime<Utc>,
    ) -> Self {
        let mut chain = vec![harness.to_string()];
        chain.extend(super::fallback::fallbacks_of(cfg, harness));
        let stored = if cfg.quota.enabled {
            db.with_conn(store::latest_per_lane).unwrap_or_default()
        } else {
            Vec::new()
        };
        let verdicts: Vec<Verdict> = chain
            .iter()
            .map(|lane| match gate::lane_for(&cfg.agent_host, lane) {
                Some(quota_lane) => gate::check(&cfg.quota, &stored, host, &quota_lane, now),
                None => Verdict::Clear,
            })
            .collect();
        let begin = verdicts.iter().position(|v| !v.is_block()).unwrap_or(0);
        Self {
            chain,
            verdicts,
            begin,
        }
    }

    /// Whether some lane of the chain is open, so blocked ones are worth skipping.
    pub fn any_open(&self) -> bool {
        self.verdicts.iter().any(|v| !v.is_block())
    }

    pub fn is_blocked(&self, lane: &str) -> bool {
        self.index(lane)
            .is_some_and(|i| self.verdicts[i].is_block())
    }

    fn index(&self, lane: &str) -> Option<usize> {
        self.chain.iter().position(|l| l == lane)
    }

    /// Why the requested lane was passed over, when it was.
    pub fn skipped_reason(&self) -> Option<String> {
        if self.begin == 0 {
            return None;
        }
        self.verdicts[0].notice().map(|n| n.reason())
    }

    /// The `quota_warning` a start on `lane` carries.
    pub fn warning(&self, lane: &str) -> Option<Value> {
        let notice = self.verdicts[self.index(lane)?].notice()?;
        Some(json!(notice.to_json()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use hq_core::config::HarnessProfileConfig;
    use hq_core::quota::{Confidence, LaneQuota, QuotaSource, QuotaWindow, WindowKind};

    const HOST: &str = "laptop";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 15, 12, 0, 0).unwrap()
    }

    /// `claude` falls back to `codex`, which falls back to `cursor`.
    fn cfg() -> HqConfig {
        let mut cfg = HqConfig::default();
        let chain = |base: &str, fallback: &[&str]| HarnessProfileConfig {
            base: base.into(),
            fallback: fallback.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        cfg.agent_host.harness_profiles.insert(
            "claude-code".into(),
            chain("claude-code", &["codex", "cursor"]),
        );
        cfg
    }

    fn store_reading(db: &Database, lane: &str, used: f64, age: Duration) {
        let observed = now() - age;
        let q = LaneQuota {
            lane: lane.into(),
            account: None,
            windows: vec![QuotaWindow {
                label: "week".into(),
                kind: WindowKind::Weekly,
                used_percent: Some(used),
                remaining: None,
                resets_at: Some(now() + Duration::days(2)),
            }],
            source: QuotaSource::Api,
            observed_at: observed,
            confidence: Confidence::Live,
            reason: None,
        };
        db.with_conn(|c| store::insert(c, HOST, &q)).unwrap();
    }

    fn plan(db: &Database, cfg: &HqConfig) -> Plan {
        Plan::new(db, cfg, HOST, "claude-code", now())
    }

    #[test]
    fn a_blocked_lane_starts_on_the_next_open_one_and_says_why() {
        let db = Database::open_memory().unwrap();
        store_reading(&db, "claude", 97.0, Duration::minutes(1));
        let p = plan(&db, &cfg());
        assert_eq!(p.chain[p.begin], "codex");
        assert!(p.is_blocked("claude-code") && !p.is_blocked("codex"));
        let why = p.skipped_reason().unwrap();
        assert!(why.starts_with("quota: week at 97% until "), "{why}");
        assert!(p.warning("codex").is_none());
    }

    #[test]
    fn a_fallback_is_checked_the_same_way() {
        let db = Database::open_memory().unwrap();
        store_reading(&db, "claude", 97.0, Duration::minutes(1));
        store_reading(&db, "codex", 99.0, Duration::minutes(1));
        let p = plan(&db, &cfg());
        assert_eq!(p.chain[p.begin], "cursor");
    }

    #[test]
    fn with_no_usable_fallback_the_lane_still_starts_with_a_warning() {
        let db = Database::open_memory().unwrap();
        for lane in ["claude", "codex", "cursor"] {
            store_reading(&db, lane, 98.0, Duration::minutes(1));
        }
        let p = plan(&db, &cfg());
        assert_eq!((p.begin, p.skipped_reason()), (0, None));
        assert!(!p.any_open());
        assert_eq!(p.warning("claude-code").unwrap()["used_percent"], 98.0);

        let db = Database::open_memory().unwrap();
        store_reading(&db, "claude", 98.0, Duration::minutes(1));
        let alone = HqConfig::default();
        let p = plan(&db, &alone);
        assert_eq!(p.begin, 0);
        assert_eq!(p.warning("claude-code").unwrap()["lane"], "claude");
    }

    #[test]
    fn warn_unknown_stale_and_disabled_never_move_the_start() {
        let db = Database::open_memory().unwrap();
        let p = plan(&db, &cfg());
        assert!(
            p.begin == 0 && p.warning("claude-code").is_none(),
            "no reading"
        );

        store_reading(&db, "claude", 85.0, Duration::minutes(1));
        let p = plan(&db, &cfg());
        assert_eq!(p.begin, 0);
        assert_eq!(p.warning("claude-code").unwrap()["used_percent"], 85.0);

        let db = Database::open_memory().unwrap();
        store_reading(&db, "claude", 99.0, Duration::minutes(30));
        let p = plan(&db, &cfg());
        assert_eq!(p.begin, 0);
        assert_eq!(p.warning("claude-code").unwrap()["stale"], true);

        store_reading(&db, "claude", 99.0, Duration::minutes(1));
        let mut off = cfg();
        off.quota.lanes.insert("claude".into(), false);
        assert_eq!(plan(&db, &off).begin, 0);
        assert!(plan(&db, &off).warning("claude-code").is_none());
        off.quota.enabled = false;
        assert!(plan(&db, &off).warning("claude-code").is_none());
    }
}
