//! Quota window readers and threshold-crossing inbox alerts.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use hq_core::config::QuotaAlertsConfig;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::copilot_usage_samples as store;
use hq_db::value_items;

pub const COPILOT_CREDITS_WINDOW_ID: &str = "copilot:credits";
const QUOTA_ALERT_SOURCE: &str = "quota_alerts";
const PERCENT_SCALE: f64 = 100.0;

#[derive(Debug, Clone, PartialEq)]
pub struct QuotaWindow {
    pub id: String,
    pub percent_used: f64,
    pub resets_at: DateTime<Utc>,
}

#[async_trait]
pub trait QuotaReader: Send + Sync {
    async fn read(&self) -> Result<Vec<QuotaWindow>>;
}

/// Reads the Copilot credit window from stored sampler rows without calling GitHub.
pub struct CopilotQuotaReader {
    db: Database,
}

impl CopilotQuotaReader {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn read_at(&self, now: DateTime<Utc>) -> Result<Vec<QuotaWindow>> {
        let since = now - Duration::days(store::RETENTION_DAYS);
        let samples = self.db.with_conn(|c| store::list_samples_since(c, since))?;
        let Some(latest) = samples.into_iter().last() else {
            return Ok(Vec::new());
        };
        if !latest.entitlement.is_finite()
            || latest.entitlement <= 0.0
            || !latest.credits_used.is_finite()
        {
            return Ok(Vec::new());
        }
        let resets_at = match latest.reset_at {
            // A sample whose reset boundary has passed belongs to the previous cycle.
            Some(r) if r <= now => return Ok(Vec::new()),
            Some(r) => r,
            None => next_month_start(now),
        };
        let percent_used = ((latest.credits_used / latest.entitlement) * PERCENT_SCALE).max(0.0);
        Ok(vec![QuotaWindow {
            id: COPILOT_CREDITS_WINDOW_ID.to_string(),
            percent_used,
            resets_at,
        }])
    }
}

#[async_trait]
impl QuotaReader for CopilotQuotaReader {
    async fn read(&self) -> Result<Vec<QuotaWindow>> {
        self.read_at(Utc::now())
    }
}

fn next_month_start(now: DateTime<Utc>) -> DateTime<Utc> {
    let (year, month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    let Some(date) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return now + Duration::days(30);
    };
    let Some(midnight) = date.and_hms_opt(0, 0, 0) else {
        return now + Duration::days(30);
    };
    Utc.from_utc_datetime(&midnight)
}

pub fn quota_dedup_key(window_id: &str, threshold: u8, resets_at: DateTime<Utc>) -> String {
    format!("quota:{window_id}:{threshold}:{}", resets_at.timestamp())
}

fn build_alert_item(window: &QuotaWindow, threshold: u8, dedup_key: String) -> ValueItem {
    let title = format!("Quota {} reached {threshold}%", window.id);
    let body = format!(
        "{} is at {:.0}% used (crossed the {threshold}% alert). Resets on {}.",
        window.id,
        window.percent_used,
        window.resets_at.format("%Y-%m-%d %H:%M UTC"),
    );
    ValueItem::new(QUOTA_ALERT_SOURCE, ValueKind::Fyi, title, body).with_dedup_key(dedup_key)
}

/// Evaluates `reader` against `cfg` and emits at most one Fyi inbox item per crossed threshold
/// and reset boundary. Returns the number of newly emitted items.
pub async fn check_quota_alerts(
    db: &Database,
    cfg: &QuotaAlertsConfig,
    reader: &dyn QuotaReader,
) -> Result<usize> {
    if !cfg.is_active() {
        return Ok(0);
    }
    let windows = reader.read().await?;
    let mut emitted = 0;
    for window in &windows {
        for threshold in cfg.thresholds_for(&window.id) {
            if window.percent_used < f64::from(threshold) {
                continue;
            }
            let dedup_key = quota_dedup_key(&window.id, threshold, window.resets_at);
            if value_items::find_active_by_dedup(db, ValueKind::Fyi, &dedup_key)?.is_some() {
                continue;
            }
            let item = build_alert_item(window, threshold, dedup_key);
            value_items::emit(db, &item)?;
            emitted += 1;
        }
    }
    Ok(emitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::ValueState;
    use hq_db::copilot_usage_samples::{UsageSample, insert_sample};
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeReader {
        windows: Mutex<Vec<QuotaWindow>>,
        calls: AtomicUsize,
    }

    impl FakeReader {
        fn new(windows: Vec<QuotaWindow>) -> Self {
            Self {
                windows: Mutex::new(windows),
                calls: AtomicUsize::new(0),
            }
        }

        fn set_windows(&self, windows: Vec<QuotaWindow>) {
            *self.windows.lock().unwrap() = windows;
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl QuotaReader for FakeReader {
        async fn read(&self) -> Result<Vec<QuotaWindow>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.windows.lock().unwrap().clone())
        }
    }

    fn cfg_with(enabled: bool, thresholds: &[(&str, &[u8])]) -> QuotaAlertsConfig {
        let mut map = BTreeMap::new();
        for (id, pcts) in thresholds {
            map.insert((*id).to_string(), pcts.to_vec());
        }
        QuotaAlertsConfig {
            enabled,
            thresholds: map,
        }
    }

    #[tokio::test]
    async fn fake_reader_below_crossing_and_repeat_stay_at_one_item() {
        let db = Database::open_memory().unwrap();
        let cfg = cfg_with(true, &[("copilot:credits", &[80, 95])]);
        let resets_at = Utc::now() + Duration::days(5);
        let reader = FakeReader::new(vec![QuotaWindow {
            id: "copilot:credits".into(),
            percent_used: 79.9,
            resets_at,
        }]);

        assert_eq!(check_quota_alerts(&db, &cfg, &reader).await.unwrap(), 0);
        assert!(
            value_items::list_by_state(&db, ValueState::Pending)
                .unwrap()
                .is_empty()
        );

        reader.set_windows(vec![QuotaWindow {
            id: "copilot:credits".into(),
            percent_used: 82.0,
            resets_at,
        }]);
        assert_eq!(check_quota_alerts(&db, &cfg, &reader).await.unwrap(), 1);
        assert_eq!(check_quota_alerts(&db, &cfg, &reader).await.unwrap(), 0);

        let items = value_items::list_by_state(&db, ValueState::Pending).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].title.contains("copilot:credits"));
        assert!(items[0].title.contains("80%"));
        assert!(items[0].body.contains("82%"));
    }

    #[tokio::test]
    async fn simulated_restart_does_not_repeat_alert() {
        let db = Database::open_memory().unwrap();
        let cfg = cfg_with(true, &[("copilot:credits", &[80])]);
        let resets_at = Utc::now() + Duration::days(5);
        let reader1 = FakeReader::new(vec![QuotaWindow {
            id: "copilot:credits".into(),
            percent_used: 85.0,
            resets_at,
        }]);
        assert_eq!(check_quota_alerts(&db, &cfg, &reader1).await.unwrap(), 1);

        // Fresh reader and config over the same database simulate a daemon restart.
        let reader2 = FakeReader::new(vec![QuotaWindow {
            id: "copilot:credits".into(),
            percent_used: 88.0,
            resets_at,
        }]);
        assert_eq!(check_quota_alerts(&db, &cfg, &reader2).await.unwrap(), 0);
        assert_eq!(
            value_items::list_by_state(&db, ValueState::Pending)
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn new_resets_at_rearms_threshold() {
        let db = Database::open_memory().unwrap();
        let cfg = cfg_with(true, &[("copilot:credits", &[80])]);
        let first_reset = Utc::now() + Duration::days(2);
        let second_reset = first_reset + Duration::days(30);
        let reader = FakeReader::new(vec![QuotaWindow {
            id: "copilot:credits".into(),
            percent_used: 85.0,
            resets_at: first_reset,
        }]);
        assert_eq!(check_quota_alerts(&db, &cfg, &reader).await.unwrap(), 1);

        reader.set_windows(vec![QuotaWindow {
            id: "copilot:credits".into(),
            percent_used: 81.0,
            resets_at: second_reset,
        }]);
        assert_eq!(check_quota_alerts(&db, &cfg, &reader).await.unwrap(), 1);
        assert_eq!(
            value_items::list_by_state(&db, ValueState::Pending)
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn unconfigured_emits_nothing_and_never_calls_reader() {
        let db = Database::open_memory().unwrap();
        let reader = FakeReader::new(vec![QuotaWindow {
            id: "copilot:credits".into(),
            percent_used: 99.0,
            resets_at: Utc::now() + Duration::days(1),
        }]);

        let default_cfg = QuotaAlertsConfig::default();
        assert_eq!(
            check_quota_alerts(&db, &default_cfg, &reader)
                .await
                .unwrap(),
            0
        );
        assert_eq!(reader.call_count(), 0);

        let empty_thresholds = cfg_with(true, &[]);
        assert_eq!(
            check_quota_alerts(&db, &empty_thresholds, &reader)
                .await
                .unwrap(),
            0
        );
        assert_eq!(reader.call_count(), 0);
        assert!(
            value_items::list_by_state(&db, ValueState::Pending)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn copilot_reader_builds_window_from_stored_samples() {
        let db = Database::open_memory().unwrap();
        let now = Utc::now();
        let reset = now + Duration::days(10);
        db.with_conn(|c| {
            insert_sample(
                c,
                &UsageSample {
                    ts: now - Duration::minutes(10),
                    login: Some("seat".into()),
                    credits_used: 400.0,
                    remaining: 600.0,
                    entitlement: 1000.0,
                    reset_at: Some(reset),
                },
            )?;
            insert_sample(
                c,
                &UsageSample {
                    ts: now - Duration::minutes(1),
                    login: Some("seat".into()),
                    credits_used: 850.0,
                    remaining: 150.0,
                    entitlement: 1000.0,
                    reset_at: Some(reset),
                },
            )
        })
        .unwrap();

        let reader = CopilotQuotaReader::new(db);
        let windows = reader.read_at(now).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, COPILOT_CREDITS_WINDOW_ID);
        assert!((windows[0].percent_used - 85.0).abs() < 1e-9);
        assert_eq!(windows[0].resets_at.timestamp(), reset.timestamp());
    }
}
