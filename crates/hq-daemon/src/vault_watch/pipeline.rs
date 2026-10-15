//! Runs the watcher's events through the queue and into the search index.
//!
//! The shell around [`Engine`](super::queue::Engine): it waits for events or the next deadline,
//! hands whatever is ready to a [`Target`] on a blocking thread, and feeds failures back for
//! retry. A watcher that cannot start leaves the scheduled scan as the only refresh; a watcher
//! that loses events raises a flag and gets a full scan.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use hq_agent::shutdown::ShutdownSignal;
use hq_core::config::VaultWatchConfig;
use hq_db::Database;
use hq_db::search::{
    NOTEBOOKS_DIR, PathOutcome, SyncReport, reconcile_path, sync_index, sync_subtree,
};
use tokio::sync::mpsc::Receiver;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::normalize::WatchEvent;
use super::queue::{Engine, EngineConfig, Retry, Work};
use super::watcher::{VaultWatcher, WatchBackend, WatchStats};

/// Events the watcher may have queued ahead of the pipeline before it starts dropping them.
const CHANNEL_CAPACITY: usize = 4096;

/// Events taken from the channel in one go, so a burst is handled together.
const DRAIN_BATCH: usize = 1024;

/// Wait before trying to start the watcher again after a failure.
const RESTART_DELAY: Duration = Duration::from_secs(60);

/// What one piece of work changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub indexed: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub failed: usize,
}

impl From<PathOutcome> for Applied {
    fn from(outcome: PathOutcome) -> Self {
        let mut applied = Self::default();
        match outcome {
            PathOutcome::Indexed => applied.indexed = 1,
            PathOutcome::Removed => applied.removed = 1,
            PathOutcome::Unchanged | PathOutcome::Ignored => applied.unchanged = 1,
        }
        applied
    }
}

impl From<SyncReport> for Applied {
    fn from(report: SyncReport) -> Self {
        Self {
            indexed: report.indexed,
            removed: report.removed,
            unchanged: report.unchanged,
            failed: report.failed,
        }
    }
}

/// Whatever brings the index in step with the files. An `Err` is a failure worth retrying (the
/// database was busy); a file that cannot be read is counted in `Applied::failed`, not an error.
pub trait Target: Send + Sync + 'static {
    fn apply(&self, work: &Work) -> Result<Applied>;
}

/// The real target: the search index in the vault database.
pub struct DbTarget {
    db: Arc<Database>,
    vault_path: PathBuf,
}

impl DbTarget {
    pub fn new(db: Arc<Database>, vault_path: PathBuf) -> Self {
        Self { db, vault_path }
    }
}

impl Target for DbTarget {
    fn apply(&self, work: &Work) -> Result<Applied> {
        let vault = self.vault_path.clone();
        match work {
            Work::Note(rel) => {
                let rel = rel.clone();
                self.db
                    .with_conn(move |conn| reconcile_path(conn, &vault, &rel))
                    .map(Applied::from)
            }
            Work::Folder(rel) => {
                let rel = rel.clone();
                self.db
                    .with_conn(move |conn| sync_subtree(conn, &vault, &rel))
                    .map(Applied::from)
            }
            Work::Everything => self
                .db
                .with_conn(move |conn| sync_index(conn, &vault.join(NOTEBOOKS_DIR)))
                .map(Applied::from),
        }
    }
}

/// Counters for the health report. Written by the pipeline, read by anyone.
#[derive(Debug, Default)]
pub struct PipelineStats {
    pub events_in: AtomicU64,
    /// Events merged into a path that was already waiting.
    pub coalesced: AtomicU64,
    /// Times the queue filled and was replaced by a full scan.
    pub queue_overflows: AtomicU64,
    pub notes_indexed: AtomicU64,
    pub notes_removed: AtomicU64,
    pub notes_unchanged: AtomicU64,
    /// Files that exist but could not be read; their rows were left alone.
    pub files_failed: AtomicU64,
    pub folder_syncs: AtomicU64,
    pub full_scans: AtomicU64,
    pub retries: AtomicU64,
    /// Work given up on after its retries; the scheduled scan is the floor for these.
    pub abandoned: AtomicU64,
    /// Paths and folders waiting right now.
    pub pending: AtomicU64,
    /// Wall-clock milliseconds since the Unix epoch, 0 for never.
    pub last_scan_started_ms: AtomicI64,
    pub last_scan_finished_ms: AtomicI64,
    pub last_error: Mutex<Option<String>>,
    /// Signalled whenever notes were indexed, so a consumer such as the embedding worker can run
    /// without waiting for its timer. A signal sent while nobody waits is kept.
    pub index_changed: Arc<tokio::sync::Notify>,
    /// The running watcher, if one has started.
    pub watcher: Mutex<Option<(WatchBackend, Arc<WatchStats>)>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl PipelineStats {
    fn record(&self, work: &Work, applied: Applied) {
        let add = |counter: &AtomicU64, n: usize| counter.fetch_add(n as u64, Ordering::Relaxed);
        add(&self.notes_indexed, applied.indexed);
        add(&self.notes_removed, applied.removed);
        add(&self.notes_unchanged, applied.unchanged);
        add(&self.files_failed, applied.failed);
        match work {
            Work::Folder(_) => {
                add(&self.folder_syncs, 1);
            }
            Work::Everything => {
                add(&self.full_scans, 1);
                self.last_scan_finished_ms
                    .store(now_ms(), Ordering::Relaxed);
            }
            Work::Note(_) => {}
        }
    }

    fn record_error(&self, error: &anyhow::Error) {
        if let Ok(mut last) = self.last_error.lock() {
            *last = Some(error.to_string());
        }
    }
}

fn ingest(engine: &mut Engine, stats: &PipelineStats, event: WatchEvent) {
    let result = engine.ingest(event, Instant::now());
    stats.events_in.fetch_add(1, Ordering::Relaxed);
    stats
        .coalesced
        .fetch_add(u64::from(result.coalesced), Ordering::Relaxed);
    if result.queue_overflow {
        stats.queue_overflows.fetch_add(1, Ordering::Relaxed);
    }
}

/// Run what is ready on a blocking thread (the database is synchronous), then record the
/// outcome and queue failures for retry.
async fn run_due(
    engine: &mut Engine,
    target: &Arc<dyn Target>,
    stats: &PipelineStats,
    due: Vec<(Work, u32)>,
) {
    if due.iter().any(|(work, _)| *work == Work::Everything) {
        stats
            .last_scan_started_ms
            .store(now_ms(), Ordering::Relaxed);
    }
    let target = target.clone();
    let results = tokio::task::spawn_blocking(move || {
        due.into_iter()
            .map(|(work, attempts)| {
                let result = target.apply(&work);
                (work, attempts, result)
            })
            .collect::<Vec<_>>()
    })
    .await;
    let Ok(results) = results else {
        warn!("vault watch: a reconcile task panicked; scanning again");
        engine.request_scan();
        return;
    };

    for (work, attempts, result) in results {
        match result {
            Ok(applied) => {
                debug!(?work, ?applied, "vault watch: reconciled");
                if work == Work::Everything {
                    engine.scan_succeeded();
                }
                stats.record(&work, applied);
                if applied.indexed > 0 {
                    stats.index_changed.notify_one();
                }
            }
            Err(e) => {
                warn!(?work, error = %e, "vault watch: reconcile failed");
                stats.record_error(&e);
                match engine.fail(work, attempts, Instant::now()) {
                    Retry::At(_) => stats.retries.fetch_add(1, Ordering::Relaxed),
                    Retry::GaveUp => stats.abandoned.fetch_add(1, Ordering::Relaxed),
                };
            }
        }
    }
}

/// Consume `rx` until it closes or `shutdown` fires.
///
/// With `initial_scan` the first thing done is a full scan. The watcher is already registered by
/// then, so anything changed before it started is found by the scan and anything changed after
/// arrives as an event; nothing falls in between.
pub async fn run_pipeline(
    mut rx: Receiver<WatchEvent>,
    watch: Arc<WatchStats>,
    target: Arc<dyn Target>,
    config: EngineConfig,
    stats: Arc<PipelineStats>,
    shutdown: ShutdownSignal,
    initial_scan: bool,
) {
    let mut engine = Engine::new(config);
    if initial_scan {
        engine.request_scan();
    }
    loop {
        if shutdown.is_triggered() {
            break;
        }
        let wake = engine.next_wakeup(Instant::now());
        let deadline = async {
            match wake {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = shutdown.wait() => break,
            event = rx.recv() => {
                let Some(event) = event else { break };
                ingest(&mut engine, &stats, event);
                for _ in 0..DRAIN_BATCH {
                    match rx.try_recv() {
                        Ok(event) => ingest(&mut engine, &stats, event),
                        Err(_) => break,
                    }
                }
            }
            _ = deadline => {}
        }
        if watch.take_rescan() {
            engine.request_scan();
        }
        let due = engine.take_due(Instant::now());
        if !due.is_empty() {
            run_due(&mut engine, &target, &stats, due).await;
        }
        stats
            .pending
            .store(engine.pending() as u64, Ordering::Relaxed);
    }
}

/// Start the watcher and its pipeline for the vault, and keep them running.
///
/// Returns at once. If the watcher cannot start (`Notebooks/` does not exist yet, the watch
/// limit is hit in `native` mode) it logs, runs one scan so the index is not left waiting for
/// the scheduled task, and tries again after a minute.
pub fn spawn(
    vault_path: PathBuf,
    db: Arc<Database>,
    config: VaultWatchConfig,
    shutdown: ShutdownSignal,
) -> Arc<PipelineStats> {
    let stats = Arc::new(PipelineStats::default());
    tokio::spawn(supervise(vault_path, db, config, shutdown, stats.clone()));
    stats
}

async fn supervise(
    vault_path: PathBuf,
    db: Arc<Database>,
    config: VaultWatchConfig,
    shutdown: ShutdownSignal,
    stats: Arc<PipelineStats>,
) {
    let target: Arc<dyn Target> = Arc::new(DbTarget::new(db, vault_path.clone()));
    let engine_config = EngineConfig::with_debounce(config.debounce());
    let mut caught_up_without_watcher = false;

    while !shutdown.is_triggered() {
        let (tx, rx) = tokio::sync::mpsc::channel(CHANNEL_CAPACITY);
        match VaultWatcher::start(&vault_path, &config, tx) {
            Ok(watcher) => {
                info!(backend = ?watcher.backend, "vault watch: watching Notebooks/ for edits");
                if let Ok(mut slot) = stats.watcher.lock() {
                    *slot = Some((watcher.backend, watcher.stats.clone()));
                }
                run_pipeline(
                    rx,
                    watcher.stats.clone(),
                    target.clone(),
                    engine_config,
                    stats.clone(),
                    shutdown.clone(),
                    true,
                )
                .await;
                if let Ok(mut slot) = stats.watcher.lock() {
                    *slot = None;
                }
                drop(watcher);
                if shutdown.is_triggered() {
                    break;
                }
                warn!("vault watch: pipeline stopped unexpectedly; restarting");
            }
            Err(e) => {
                warn!(error = %e, "vault watch: not watching; the scheduled scan keeps the index current");
                stats.record_error(&e);
                if !caught_up_without_watcher {
                    caught_up_without_watcher = true;
                    let target = target.clone();
                    let scan =
                        tokio::task::spawn_blocking(move || target.apply(&Work::Everything)).await;
                    if let Ok(Ok(applied)) = scan {
                        stats.record(&Work::Everything, applied);
                    }
                }
            }
        }
        tokio::select! {
            _ = shutdown.wait() => break,
            _ = tokio::time::sleep(RESTART_DELAY) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use hq_core::config::VaultWatchConfig;
    use tokio::sync::mpsc::Sender;

    use super::*;

    #[derive(Default)]
    struct FakeTarget {
        calls: Mutex<Vec<Work>>,
        fail_next: AtomicU32,
    }

    impl FakeTarget {
        fn calls(&self) -> Vec<Work> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Target for FakeTarget {
        fn apply(&self, work: &Work) -> Result<Applied> {
            self.calls.lock().unwrap().push(work.clone());
            if self.fail_next.load(Ordering::SeqCst) > 0 {
                self.fail_next.fetch_sub(1, Ordering::SeqCst);
                anyhow::bail!("database is busy");
            }
            Ok(Applied {
                indexed: 1,
                ..Applied::default()
            })
        }
    }

    struct Rig {
        tx: Sender<WatchEvent>,
        watch: Arc<WatchStats>,
        stats: Arc<PipelineStats>,
        target: Arc<FakeTarget>,
        shutdown: ShutdownSignal,
        task: tokio::task::JoinHandle<()>,
    }

    fn rig(initial_scan: bool) -> Rig {
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let watch = Arc::new(WatchStats::default());
        let stats = Arc::new(PipelineStats::default());
        let target = Arc::new(FakeTarget::default());
        let shutdown = ShutdownSignal::new();
        let task = tokio::spawn(run_pipeline(
            rx,
            watch.clone(),
            target.clone(),
            EngineConfig::with_debounce(Duration::from_millis(300)),
            stats.clone(),
            shutdown.clone(),
            initial_scan,
        ));
        Rig {
            tx,
            watch,
            stats,
            target,
            shutdown,
            task,
        }
    }

    /// Poll a condition, advancing the (paused) clock, for at most 100 virtual seconds.
    async fn wait_until(condition: impl FnMut() -> bool) {
        wait_until_describing(condition, String::new).await
    }

    /// Like `wait_until`, but a timeout reports what the condition was looking at.
    async fn wait_until_describing(
        mut condition: impl FnMut() -> bool,
        describe: impl Fn() -> String,
    ) {
        for _ in 0..10_000 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition never became true: {}", describe());
    }

    fn note(p: &str) -> Work {
        Work::Note(p.into())
    }

    fn changed(p: &str) -> WatchEvent {
        WatchEvent::Changed(p.into())
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_saves_is_one_reconcile() {
        let rig = rig(false);
        for _ in 0..50 {
            rig.tx.send(changed("Notebooks/a.md")).await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        wait_until(|| !rig.target.calls().is_empty()).await;
        tokio::time::sleep(Duration::from_secs(10)).await;

        assert_eq!(rig.target.calls(), vec![note("Notebooks/a.md")]);
        assert_eq!(rig.stats.events_in.load(Ordering::Relaxed), 50);
        assert_eq!(rig.stats.coalesced.load(Ordering::Relaxed), 49);
        assert_eq!(rig.stats.notes_indexed.load(Ordering::Relaxed), 1);
        rig.shutdown.trigger();
        rig.task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn indexing_a_note_signals_the_embedding_consumer() {
        let rig = rig(false);
        rig.tx.send(changed("Notebooks/a.md")).await.unwrap();
        // The signal is kept until someone waits for it, so there is no race with the worker.
        tokio::time::timeout(Duration::from_secs(60), rig.stats.index_changed.notified())
            .await
            .expect("no signal after a note was indexed");
        rig.shutdown.trigger();
        rig.task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_thing_after_start_is_a_full_scan() {
        let rig = rig(true);
        wait_until(|| !rig.target.calls().is_empty()).await;
        assert_eq!(rig.target.calls(), vec![Work::Everything]);
        assert_eq!(rig.stats.full_scans.load(Ordering::Relaxed), 1);
        rig.shutdown.trigger();
        rig.task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn lost_events_are_repaired_by_a_scan() {
        let rig = rig(false);
        // The watcher dropped events and says so; the next thing to arrive wakes the pipeline.
        rig.watch.request_rescan();
        rig.tx.send(changed("Notebooks/x.md")).await.unwrap();
        wait_until(|| !rig.target.calls().is_empty()).await;
        tokio::time::sleep(Duration::from_secs(10)).await;

        assert_eq!(rig.target.calls(), vec![Work::Everything]);
        rig.shutdown.trigger();
        rig.task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_busy_database_is_retried_after_a_delay() {
        let rig = rig(false);
        rig.target.fail_next.store(1, Ordering::SeqCst);
        rig.tx.send(changed("Notebooks/a.md")).await.unwrap();
        wait_until(|| rig.target.calls().len() >= 2).await;

        assert_eq!(
            rig.target.calls(),
            vec![note("Notebooks/a.md"), note("Notebooks/a.md")]
        );
        assert_eq!(rig.stats.retries.load(Ordering::Relaxed), 1);
        wait_until(|| rig.stats.notes_indexed.load(Ordering::Relaxed) == 1).await;
        assert!(rig.stats.last_error.lock().unwrap().is_some());
        rig.shutdown.trigger();
        rig.task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn work_that_keeps_failing_is_given_up_on_not_retried_forever() {
        let rig = rig(false);
        rig.target.fail_next.store(1_000, Ordering::SeqCst);
        rig.tx.send(changed("Notebooks/a.md")).await.unwrap();
        wait_until(|| rig.stats.abandoned.load(Ordering::Relaxed) == 1).await;
        tokio::time::sleep(Duration::from_secs(120)).await;

        assert_eq!(
            rig.target.calls().len(),
            4,
            "four tries, then left to the scan"
        );
        assert_eq!(rig.stats.retries.load(Ordering::Relaxed), 3);
        rig.shutdown.trigger();
        rig.task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn the_pipeline_ends_when_the_watcher_goes_away_or_on_shutdown() {
        let rig_closed = rig(false);
        drop(rig_closed.tx);
        rig_closed.task.await.unwrap();

        let rig_shutdown = rig(false);
        rig_shutdown.shutdown.trigger();
        rig_shutdown.task.await.unwrap();
    }

    mod against_the_real_index {
        use super::*;

        struct Vault {
            dir: tempfile::TempDir,
            db: Arc<Database>,
        }

        impl Vault {
            fn new() -> Self {
                let dir = tempfile::tempdir().unwrap();
                std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
                Self {
                    dir,
                    db: Arc::new(Database::open_memory().unwrap()),
                }
            }

            /// Like `new`, with the database where production keeps it, under `_data/` beside
            /// `Notebooks/`, so the index's own writes happen on the real file system.
            fn on_disk() -> Self {
                let dir = tempfile::tempdir().unwrap();
                std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
                std::fs::create_dir_all(dir.path().join("_data")).unwrap();
                let db = Database::open(&dir.path().join("_data/vault.db")).unwrap();
                Self {
                    dir,
                    db: Arc::new(db),
                }
            }

            fn write(&self, rel: &str, body: &str) {
                let path = self.dir.path().join(rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, body).unwrap();
            }

            fn found(&self, word: &str) -> usize {
                let word = word.to_string();
                self.db
                    .with_conn(move |c| hq_db::search::keyword_search(c, &word, 10))
                    .unwrap()
                    .len()
            }

            fn start(&self) -> Rig2 {
                self.start_with(
                    EngineConfig::with_debounce(Duration::from_millis(300)),
                    true,
                )
            }

            fn start_with(&self, config: EngineConfig, initial_scan: bool) -> Rig2 {
                let (tx, rx) = tokio::sync::mpsc::channel(256);
                let target: Arc<dyn Target> = Arc::new(DbTarget::new(
                    self.db.clone(),
                    self.dir.path().to_path_buf(),
                ));
                let shutdown = ShutdownSignal::new();
                let stats = Arc::new(PipelineStats::default());
                let task = tokio::spawn(run_pipeline(
                    rx,
                    Arc::new(WatchStats::default()),
                    target,
                    config,
                    stats.clone(),
                    shutdown.clone(),
                    initial_scan,
                ));
                Rig2 {
                    tx,
                    stats,
                    shutdown,
                    task,
                }
            }
        }

        struct Rig2 {
            tx: Sender<WatchEvent>,
            stats: Arc<PipelineStats>,
            shutdown: ShutdownSignal,
            task: tokio::task::JoinHandle<()>,
        }

        #[tokio::test(start_paused = true)]
        async fn the_first_scan_catches_up_and_events_keep_the_index_current() {
            let vault = Vault::new();
            vault.write("Notebooks/old.md", "written while the daemon was down");
            let rig = vault.start();
            wait_until(|| vault.found("daemon") == 1).await;

            // An edit, a new note and a delete, each announced only as a hint.
            vault.write("Notebooks/old.md", "rewritten with zebra");
            vault.write("Notebooks/new.md", "brand new giraffe");
            rig.tx.send(changed("Notebooks/old.md")).await.unwrap();
            rig.tx
                .send(WatchEvent::Created("Notebooks/new.md".into()))
                .await
                .unwrap();
            wait_until(|| vault.found("zebra") == 1 && vault.found("giraffe") == 1).await;
            assert_eq!(vault.found("daemon"), 0, "the old text is gone");

            std::fs::remove_file(vault.dir.path().join("Notebooks/new.md")).unwrap();
            rig.tx
                .send(WatchEvent::Deleted("Notebooks/new.md".into()))
                .await
                .unwrap();
            wait_until(|| vault.found("giraffe") == 0).await;

            rig.shutdown.trigger();
            rig.task.await.unwrap();
            assert!(rig.stats.notes_indexed.load(Ordering::Relaxed) >= 3);
        }

        #[tokio::test(start_paused = true)]
        async fn a_moved_folder_is_followed_from_the_folder_event_alone() {
            let vault = Vault::new();
            vault.write("Notebooks/A/n.md", "alpaca inside a folder");
            let rig = vault.start();
            wait_until(|| vault.found("alpaca") == 1).await;

            std::fs::rename(
                vault.dir.path().join("Notebooks/A"),
                vault.dir.path().join("Notebooks/B"),
            )
            .unwrap();
            rig.tx
                .send(WatchEvent::Folder("Notebooks/A".into()))
                .await
                .unwrap();
            rig.tx
                .send(WatchEvent::Folder("Notebooks/B".into()))
                .await
                .unwrap();
            let moved = || {
                vault
                    .db
                    .with_conn(|c| hq_db::search::keyword_search(c, "alpaca", 10))
                    .map(|hits| hits.into_iter().map(|h| h.note_path).collect::<Vec<_>>())
            };
            wait_until_describing(
                || moved().is_ok_and(|paths| paths == ["Notebooks/B/n.md"]),
                || {
                    format!(
                        "index has {:?}; last error {:?}; folder syncs {}",
                        moved(),
                        rig.stats.last_error.lock().unwrap(),
                        rig.stats.folder_syncs.load(Ordering::Relaxed)
                    )
                },
            )
            .await;

            rig.shutdown.trigger();
            rig.task.await.unwrap();
        }

        #[tokio::test(start_paused = true)]
        async fn a_flood_larger_than_the_queue_is_repaired_by_a_scan() {
            let vault = Vault::new();
            let mut config = EngineConfig::with_debounce(Duration::from_millis(300));
            config.capacity = 2;
            let rig = vault.start_with(config, false);

            let words = ["ibis", "kiwi", "lynx", "mole", "newt"];
            for (i, word) in words.iter().enumerate() {
                vault.write(&format!("Notebooks/n{i}.md"), &format!("note about {word}"));
                rig.tx
                    .send(WatchEvent::Created(format!("Notebooks/n{i}.md")))
                    .await
                    .unwrap();
            }
            // Only two paths fit; the rest overflow into one full scan that finds all five.
            wait_until_describing(
                || words.iter().all(|w| vault.found(w) == 1),
                || {
                    format!(
                        "overflows {}",
                        rig.stats.queue_overflows.load(Ordering::Relaxed)
                    )
                },
            )
            .await;
            assert!(rig.stats.queue_overflows.load(Ordering::Relaxed) >= 1);
            assert!(rig.stats.full_scans.load(Ordering::Relaxed) >= 1);

            rig.shutdown.trigger();
            rig.task.await.unwrap();
        }

        /// The whole chain with nothing faked: real inotify, the real debounce, the real index.
        /// Prints how long each kind of edit took to become searchable. Run:
        /// `cargo test -p hq-daemon live_edits -- --ignored --nocapture`
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "uses real filesystem events and timing; run with --ignored"]
        async fn live_edits_reach_search_within_seconds() {
            let vault = Vault::on_disk();
            vault.write(
                "Notebooks/seed.md",
                "seed text from before the daemon started",
            );
            let shutdown = ShutdownSignal::new();
            let stats = spawn(
                vault.dir.path().to_path_buf(),
                vault.db.clone(),
                VaultWatchConfig::default(),
                shutdown.clone(),
            );
            wait_until(|| vault.found("seed") == 1).await;
            wait_until(|| stats.watcher.lock().unwrap().is_some()).await;

            let started = std::time::Instant::now();
            vault.write("Notebooks/Sub/new.md", "a fresh pelican note");
            wait_until(|| vault.found("pelican") == 1).await;
            println!("new note searchable after {:?}", started.elapsed());

            // A save by replacement, the way editors and git write.
            let started = std::time::Instant::now();
            let note = vault.dir.path().join("Notebooks/Sub/new.md");
            let temp = vault.dir.path().join("Notebooks/Sub/.new.md.tmp");
            std::fs::write(&temp, "rewritten about a heron").unwrap();
            std::fs::rename(&temp, &note).unwrap();
            wait_until(|| vault.found("heron") == 1 && vault.found("pelican") == 0).await;
            println!("replacement save searchable after {:?}", started.elapsed());

            // Moving a folder reports the folder, not the notes inside it.
            let started = std::time::Instant::now();
            std::fs::rename(
                vault.dir.path().join("Notebooks/Sub"),
                vault.dir.path().join("Notebooks/Moved"),
            )
            .unwrap();
            let moved = || {
                vault
                    .db
                    .with_conn(|c| hq_db::search::keyword_search(c, "heron", 10))
                    .map(|hits| hits.into_iter().map(|h| h.note_path).collect::<Vec<_>>())
            };
            wait_until(|| moved().is_ok_and(|paths| paths == ["Notebooks/Moved/new.md"])).await;
            println!("folder move reflected after {:?}", started.elapsed());

            let started = std::time::Instant::now();
            std::fs::remove_file(vault.dir.path().join("Notebooks/Moved/new.md")).unwrap();
            wait_until(|| vault.found("heron") == 0).await;
            println!("delete reflected after {:?}", started.elapsed());

            // 50 quick saves of one note are one index, not fifty.
            let before = stats.notes_indexed.load(Ordering::Relaxed);
            for i in 0..50 {
                vault.write("Notebooks/burst.md", &format!("burst version {i} flamingo"));
            }
            wait_until(|| vault.found("flamingo") == 1).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let indexed = stats.notes_indexed.load(Ordering::Relaxed) - before;
            println!("50 saves caused {indexed} index operation(s)");
            assert!(indexed <= 3, "a burst should coalesce, saw {indexed}");

            // The index writes its own database and cache on every change. Those writes must not
            // come back as events: once the edits stop, the watcher goes quiet.
            tokio::time::sleep(Duration::from_secs(1)).await;
            let notifications = || {
                let slot = stats.watcher.lock().unwrap();
                slot.as_ref()
                    .unwrap()
                    .1
                    .notifications
                    .load(Ordering::Relaxed)
            };
            let settled = notifications();
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert_eq!(
                notifications(),
                settled,
                "the pipeline's own writes are feeding back into the watcher"
            );
            println!("no feedback after {settled} notifications");

            shutdown.trigger();
        }

        #[tokio::test(start_paused = true)]
        async fn a_save_that_only_touches_the_file_costs_nothing() {
            let vault = Vault::new();
            vault.write("Notebooks/a.md", "unchanging llama");
            let rig = vault.start();
            wait_until(|| vault.found("llama") == 1).await;
            let indexed = rig.stats.notes_indexed.load(Ordering::Relaxed);

            // Same bytes written again: an event arrives, the hash says nothing changed.
            vault.write("Notebooks/a.md", "unchanging llama");
            rig.tx.send(changed("Notebooks/a.md")).await.unwrap();
            wait_until(|| rig.stats.notes_unchanged.load(Ordering::Relaxed) >= 1).await;
            assert_eq!(rig.stats.notes_indexed.load(Ordering::Relaxed), indexed);

            rig.shutdown.trigger();
            rig.task.await.unwrap();
        }
    }
}
