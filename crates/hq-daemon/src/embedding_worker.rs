//! Keeps vector embeddings current in the background.
//!
//! Woken when the vault watcher indexes notes, and on a slow timer as a floor, it embeds what is
//! new, edited or from another model a few notes at a time. A burst of edits is one pass, a pass
//! has a budget, and a provider that fails is left alone for longer each time. It is independent
//! of the keyword index: search is already current when this runs, and a failure here changes
//! nothing there.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use hq_agent::shutdown::ShutdownSignal;
use hq_db::Database;
use hq_vault::VaultClient;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tracing::{info, warn};

use crate::embeddings::{Embedder, embed_pending, embedder_for};

/// Key under which the report sits in `.daemon-metrics.json`.
pub const METRICS_KEY: &str = "embedding_worker";

/// How many skipped notes the status names. Enough to find them, few enough to read.
pub const MAX_UNREADABLE_PATHS: usize = 5;

#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    /// Notes per provider batch.
    pub batch: usize,
    /// Notes per wake-up. A large burst of edits is worked through over several wake-ups instead
    /// of in one run, which bounds provider load and spend.
    pub max_per_wake: usize,
    /// Pause between batches in one wake-up.
    pub pace: Duration,
    /// Wait after a change signal, so a burst of edits is one pass.
    pub debounce: Duration,
    /// Look for work this often even with no signal.
    pub idle_check: Duration,
    /// First wait after a failure; doubles on each failure up to `backoff_max`.
    pub backoff_start: Duration,
    pub backoff_max: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            batch: 20,
            max_per_wake: 200,
            pace: Duration::from_millis(500),
            debounce: Duration::from_secs(3),
            idle_check: Duration::from_secs(300),
            backoff_start: Duration::from_secs(30),
            backoff_max: Duration::from_secs(15 * 60),
        }
    }
}

impl WorkerConfig {
    fn next_backoff(&self, current: Duration) -> Duration {
        if current.is_zero() {
            self.backoff_start
        } else {
            (current * 2).min(self.backoff_max)
        }
    }
}

#[derive(Debug, Default)]
pub struct EmbedStats {
    pub embedded: AtomicU64,
    pub failed: AtomicU64,
    pub unreadable: AtomicU64,
    /// Notes the worker is passing over because they could not be read, a few of them, so a
    /// health report can name them. The count above only grows; this is what is skipped now.
    pub unreadable_paths: Mutex<Vec<String>>,
    pub passes: AtomicU64,
    /// Wall-clock milliseconds since the Unix epoch of the latest pass, 0 for none.
    pub last_pass_ms: AtomicI64,
    /// Seconds the worker is currently holding off after a failure, 0 when healthy.
    pub backoff_secs: AtomicU64,
    pub last_error: Mutex<Option<String>>,
}

/// A point-in-time report for the health surfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingWorkerStatus {
    pub embedded: u64,
    pub failed: u64,
    pub unreadable: u64,
    /// Up to [`MAX_UNREADABLE_PATHS`] notes being skipped now, in path order.
    #[serde(default)]
    pub unreadable_paths: Vec<String>,
    pub passes: u64,
    pub last_pass_at: Option<String>,
    pub backoff_secs: u64,
    pub last_error: Option<String>,
    pub updated_at: String,
}

impl EmbedStats {
    pub fn snapshot(&self) -> EmbeddingWorkerStatus {
        let last_pass = self.last_pass_ms.load(Ordering::Relaxed);
        EmbeddingWorkerStatus {
            embedded: self.embedded.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            unreadable: self.unreadable.load(Ordering::Relaxed),
            unreadable_paths: self
                .unreadable_paths
                .lock()
                .map(|paths| paths.clone())
                .unwrap_or_default(),
            passes: self.passes.load(Ordering::Relaxed),
            last_pass_at: (last_pass > 0)
                .then(|| DateTime::from_timestamp_millis(last_pass))
                .flatten()
                .map(|t| t.to_rfc3339()),
            backoff_secs: self.backoff_secs.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|e| (*e).clone()),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    /// Keep the first few of the notes being skipped, in path order.
    fn set_unreadable_paths(&self, skipped: &HashSet<String>) {
        let mut paths: Vec<String> = skipped.iter().cloned().collect();
        paths.sort();
        paths.truncate(MAX_UNREADABLE_PATHS);
        if let Ok(mut slot) = self.unreadable_paths.lock() {
            *slot = paths;
        }
    }

    fn set_error(&self, error: Option<String>) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = error;
        }
    }
}

/// The report the daemon last wrote, if any.
pub fn read(vault_path: &Path) -> Option<EmbeddingWorkerStatus> {
    let raw = std::fs::read_to_string(vault_path.join("_system/.daemon-metrics.json")).ok()?;
    let metrics: serde_json::Value = serde_json::from_str(&raw).ok()?;
    serde_json::from_value(metrics.get(METRICS_KEY)?.clone()).ok()
}

/// Start the worker. `changed` is signalled when notes were indexed (the vault watcher's
/// `PipelineStats::index_changed`); pass a fresh `Notify` when there is no watcher and the worker
/// will rely on its timer. Returns at once.
pub fn spawn(
    vault_path: PathBuf,
    db: Arc<Database>,
    openrouter_api_key: Option<String>,
    changed: Arc<Notify>,
    shutdown: ShutdownSignal,
) -> Arc<EmbedStats> {
    let stats = Arc::new(EmbedStats::default());
    let task_stats = stats.clone();
    tokio::spawn(async move {
        let vault = match VaultClient::new(vault_path) {
            Ok(vault) => vault,
            Err(e) => {
                warn!(error = %e, "embedding worker: cannot open the vault");
                return;
            }
        };
        let (embedder, model) = embedder_for(openrouter_api_key.as_deref());
        info!(model = %model, "embedding worker: running");
        run_worker(
            vault,
            db,
            model,
            embedder,
            WorkerConfig::default(),
            changed,
            task_stats,
            shutdown,
        )
        .await;
    });
    stats
}

/// Sleep, but return early (true) if the daemon is shutting down.
async fn sleep_or_shutdown(shutdown: &ShutdownSignal, duration: Duration) -> bool {
    tokio::select! {
        _ = shutdown.wait() => true,
        _ = tokio::time::sleep(duration) => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_worker(
    vault: VaultClient,
    db: Arc<Database>,
    model: String,
    embedder: Embedder,
    config: WorkerConfig,
    changed: Arc<Notify>,
    stats: Arc<EmbedStats>,
    shutdown: ShutdownSignal,
) {
    let mut skip: HashSet<String> = HashSet::new();
    let mut backoff = Duration::ZERO;

    loop {
        // Wait for a reason to look: a change signal (not while holding off after a failure),
        // the timer, or shutdown. A signal that arrives during a back-off is kept for later.
        let wait = if backoff.is_zero() {
            config.idle_check
        } else {
            backoff
        };
        let signalled = tokio::select! {
            _ = shutdown.wait() => return,
            _ = changed.notified(), if backoff.is_zero() => true,
            _ = tokio::time::sleep(wait) => false,
        };
        if signalled && sleep_or_shutdown(&shutdown, config.debounce).await {
            return;
        }

        let mut budget = config.max_per_wake;
        while budget > 0 {
            if shutdown.is_triggered() {
                return;
            }
            let report = embed_pending(
                &vault,
                &db,
                &model,
                &embedder,
                config.batch.min(budget),
                &mut skip,
            )
            .await;
            stats.passes.fetch_add(1, Ordering::Relaxed);
            stats
                .last_pass_ms
                .store(Utc::now().timestamp_millis(), Ordering::Relaxed);

            let report = match report {
                Ok(report) => report,
                Err(e) => {
                    warn!(error = %e, "embedding worker: pass failed");
                    stats.set_error(Some(e.to_string()));
                    backoff = config.next_backoff(backoff);
                    stats
                        .backoff_secs
                        .store(backoff.as_secs(), Ordering::Relaxed);
                    break;
                }
            };
            stats
                .embedded
                .fetch_add(report.embedded as u64, Ordering::Relaxed);
            stats
                .failed
                .fetch_add(report.failed as u64, Ordering::Relaxed);
            stats
                .unreadable
                .fetch_add(report.unreadable as u64, Ordering::Relaxed);
            stats.set_unreadable_paths(&skip);

            if report.gave_up || (report.failed > 0 && report.embedded == 0) {
                stats.set_error(report.last_error.clone());
                backoff = config.next_backoff(backoff);
                stats
                    .backoff_secs
                    .store(backoff.as_secs(), Ordering::Relaxed);
                break;
            }
            if report.embedded > 0 {
                backoff = Duration::ZERO;
                stats.backoff_secs.store(0, Ordering::Relaxed);
                stats.set_error(None);
            }
            if report.idle() {
                break;
            }
            budget = budget.saturating_sub(report.embedded + report.failed + report.unreadable);
            if budget > 0 && sleep_or_shutdown(&shutdown, config.pace).await {
                return;
            }
        }
        // A pass that found nothing wrong and nothing to do also ends any earlier back-off.
        if backoff.is_zero() {
            stats.backoff_secs.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    struct Fake {
        calls: AtomicUsize,
        failing: std::sync::atomic::AtomicBool,
    }

    impl Fake {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                failing: std::sync::atomic::AtomicBool::new(false),
            })
        }

        fn embedder(self: &Arc<Self>) -> Embedder {
            let fake = self.clone();
            Arc::new(move |text| {
                let fake = fake.clone();
                Box::pin(async move {
                    fake.calls.fetch_add(1, Ordering::SeqCst);
                    if fake.failing.load(Ordering::SeqCst) {
                        anyhow::bail!("rate limited");
                    }
                    Ok(vec![text.len() as f32, 1.0])
                })
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    struct Rig {
        dir: tempfile::TempDir,
        db: Arc<Database>,
        fake: Arc<Fake>,
        changed: Arc<Notify>,
        stats: Arc<EmbedStats>,
        shutdown: ShutdownSignal,
        task: tokio::task::JoinHandle<()>,
    }

    fn rig(config: WorkerConfig) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        let db = Arc::new(Database::open_memory().unwrap());
        let fake = Fake::new();
        let changed = Arc::new(Notify::new());
        let stats = Arc::new(EmbedStats::default());
        let shutdown = ShutdownSignal::new();
        let task = tokio::spawn(run_worker(
            VaultClient::new(dir.path().to_path_buf()).unwrap(),
            db.clone(),
            "m".into(),
            fake.embedder(),
            config,
            changed.clone(),
            stats.clone(),
            shutdown.clone(),
        ));
        Rig {
            dir,
            db,
            fake,
            changed,
            stats,
            shutdown,
            task,
        }
    }

    impl Rig {
        fn add_notes(&self, n: usize) {
            for i in 0..n {
                let path = self.dir.path().join(format!("Notebooks/n{i:02}.md"));
                std::fs::write(path, format!("note number {i}")).unwrap();
            }
            let notebooks = self.dir.path().join("Notebooks");
            self.db
                .with_conn(move |c| hq_db::search::sync_index(c, &notebooks))
                .unwrap();
        }

        fn embedded(&self) -> usize {
            self.db
                .with_conn(|c| Ok(hq_db::search::get_stats(c)?.embedding_count))
                .unwrap()
        }

        async fn finish(self) {
            self.shutdown.trigger();
            self.task.await.unwrap();
        }
    }

    async fn wait_until(mut condition: impl FnMut() -> bool) {
        for _ in 0..10_000 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("condition never became true");
    }

    #[tokio::test(start_paused = true)]
    async fn a_change_signal_embeds_what_is_pending_in_batches() {
        let config = WorkerConfig {
            batch: 2,
            ..WorkerConfig::default()
        };
        let rig = rig(config);
        rig.add_notes(5);
        rig.changed.notify_one();

        wait_until(|| rig.embedded() == 5).await;
        assert_eq!(rig.fake.calls(), 5);
        assert_eq!(rig.stats.embedded.load(Ordering::Relaxed), 5);
        assert!(
            rig.stats.passes.load(Ordering::Relaxed) >= 3,
            "2 + 2 + 1, then idle"
        );
        rig.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn the_timer_finds_work_even_with_no_signal() {
        let rig = rig(WorkerConfig::default());
        rig.add_notes(2);
        // Nothing signals; the idle check (5 virtual minutes) picks the notes up.
        wait_until(|| rig.embedded() == 2).await;
        rig.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn one_wake_up_has_a_budget_and_the_rest_waits_for_the_next() {
        let config = WorkerConfig {
            batch: 2,
            max_per_wake: 3,
            ..WorkerConfig::default()
        };
        let rig = rig(config);
        rig.add_notes(8);
        rig.changed.notify_one();

        wait_until(|| rig.embedded() >= 3).await;
        // The first wake-up spends its budget of three and stops, leaving five for later.
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(rig.embedded(), 3);
        // The timer brings the next wake-up, which spends another three.
        wait_until(|| rig.embedded() == 6).await;
        wait_until(|| rig.embedded() == 8).await;
        rig.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_provider_is_left_alone_for_longer_each_time_then_recovers() {
        let rig = rig(WorkerConfig::default());
        rig.fake.failing.store(true, Ordering::SeqCst);
        rig.add_notes(2);
        rig.changed.notify_one();

        wait_until(|| rig.stats.backoff_secs.load(Ordering::Relaxed) == 30).await;
        assert_eq!(rig.embedded(), 0);
        assert_eq!(
            rig.stats.snapshot().last_error.as_deref(),
            Some("rate limited")
        );

        // Still failing: the wait doubles.
        wait_until(|| rig.stats.backoff_secs.load(Ordering::Relaxed) == 60).await;
        // A change signal during the back-off does not bring the next try forward.
        let calls = rig.fake.calls();
        rig.changed.notify_one();
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(rig.fake.calls(), calls);

        // The provider recovers and the next try after the wait succeeds and clears the error.
        rig.fake.failing.store(false, Ordering::SeqCst);
        wait_until(|| rig.embedded() == 2).await;
        wait_until(|| rig.stats.backoff_secs.load(Ordering::Relaxed) == 0).await;
        assert_eq!(rig.stats.snapshot().last_error, None);
        rig.finish().await;
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_after_failures_stops_growing_at_its_ceiling() {
        let config = WorkerConfig::default();
        let mut wait = Duration::ZERO;
        let mut seen = Vec::new();
        for _ in 0..8 {
            wait = config.next_backoff(wait);
            seen.push(wait.as_secs());
        }
        assert_eq!(seen, vec![30, 60, 120, 240, 480, 900, 900, 900]);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_ends_the_worker_even_while_it_waits() {
        let rig = rig(WorkerConfig::default());
        rig.finish().await;
    }

    #[test]
    fn the_status_names_a_few_skipped_notes_in_path_order() {
        let stats = EmbedStats::default();
        let skipped: HashSet<String> = (0..8).map(|i| format!("Notebooks/n{i}.md")).collect();

        stats.set_unreadable_paths(&skipped);

        let status = stats.snapshot();
        assert_eq!(status.unreadable_paths.len(), MAX_UNREADABLE_PATHS);
        assert_eq!(status.unreadable_paths[0], "Notebooks/n0.md");

        // A metrics file written by the build before this field existed still loads.
        let old = r#"{"embedded":1,"failed":0,"unreadable":0,"passes":1,"last_pass_at":null,
            "backoff_secs":0,"last_error":null,"updated_at":"2026-10-16T00:00:00Z"}"#;
        let parsed: EmbeddingWorkerStatus = serde_json::from_str(old).unwrap();
        assert!(parsed.unreadable_paths.is_empty());
    }

    #[test]
    fn the_report_survives_the_metrics_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("_system")).unwrap();
        assert_eq!(read(dir.path()), None);

        let stats = EmbedStats::default();
        stats.embedded.store(7, Ordering::Relaxed);
        stats.backoff_secs.store(60, Ordering::Relaxed);
        stats.set_error(Some("rate limited".into()));
        let status = stats.snapshot();
        let metrics = serde_json::json!({ METRICS_KEY: status });
        std::fs::write(
            dir.path().join("_system/.daemon-metrics.json"),
            metrics.to_string(),
        )
        .unwrap();
        assert_eq!(read(dir.path()), Some(status));
    }
}
