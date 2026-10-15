//! A point-in-time report of the watcher and its pipeline, for the health surfaces.
//!
//! The daemon writes it into `_system/.daemon-metrics.json` on every tick; `hq doctor` and the
//! MCP health tools read it from there, since they run in other processes.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::pipeline::PipelineStats;
use super::watcher::{WatchBackend, WatchStats};

/// Key under which the report sits in `.daemon-metrics.json`.
pub const METRICS_KEY: &str = "vault_watch";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchState {
    /// Listening for file events and the first scan is done.
    Watching,
    /// Checking file times on a timer; events are not available here.
    Polling,
    /// The watcher is up but the first scan has not finished, so the index may be behind.
    CatchingUp,
    /// No watcher is running. The scheduled scan is the only refresh.
    NotWatching,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultWatchStatus {
    pub state: WatchState,
    /// Why the watcher is not running, or the latest error the pipeline hit.
    pub detail: Option<String>,
    /// `native` or `poll`, when a watcher is running.
    pub backend: Option<String>,
    /// Folders under the vault that are watched.
    pub roots: Vec<String>,
    pub last_notification_at: Option<String>,
    pub notifications: u64,
    pub paths_ignored: u64,
    pub events_queued: u64,
    pub coalesced: u64,
    pub queue_depth: u64,
    /// Times the queue filled and was replaced by a full scan.
    pub queue_overflows: u64,
    /// Times the operating system reported lost events.
    pub watcher_overflows: u64,
    /// Events that did not fit the channel to the pipeline.
    pub dropped_events: u64,
    pub watcher_errors: u64,
    pub notes_indexed: u64,
    pub notes_removed: u64,
    pub notes_unchanged: u64,
    pub files_failed: u64,
    pub folder_syncs: u64,
    pub full_scans: u64,
    pub retries: u64,
    pub abandoned: u64,
    pub last_scan_started_at: Option<String>,
    pub last_scan_finished_at: Option<String>,
    pub updated_at: String,
}

fn when(ms: i64) -> Option<String> {
    (ms > 0)
        .then(|| DateTime::from_timestamp_millis(ms))
        .flatten()
        .map(|t| t.to_rfc3339())
}

impl PipelineStats {
    /// Read every counter into a report.
    pub fn snapshot(&self) -> VaultWatchStatus {
        let load = |c: &std::sync::atomic::AtomicU64| c.load(Ordering::Relaxed);
        let watcher: Option<(WatchBackend, Arc<WatchStats>)> =
            self.watcher.lock().ok().and_then(|slot| (*slot).clone());
        let last_error: Option<String> = self.last_error.lock().ok().and_then(|e| (*e).clone());
        let scanned = self.last_scan_finished_ms.load(Ordering::Relaxed) > 0;

        let state = match &watcher {
            None => WatchState::NotWatching,
            Some(_) if !scanned => WatchState::CatchingUp,
            Some((WatchBackend::Native, _)) => WatchState::Watching,
            Some((WatchBackend::Poll, _)) => WatchState::Polling,
        };
        let backend = watcher.as_ref().map(|(b, _)| match b {
            WatchBackend::Native => "native".to_string(),
            WatchBackend::Poll => "poll".to_string(),
        });
        let (notifications, ignored, overflows, dropped, errors, last_event) = match &watcher {
            Some((_, w)) => (
                load(&w.notifications),
                load(&w.paths_ignored),
                load(&w.overflows),
                load(&w.dropped),
                load(&w.errors),
                w.last_notification_ms.load(Ordering::Relaxed),
            ),
            None => (0, 0, 0, 0, 0, 0),
        };

        VaultWatchStatus {
            state,
            detail: last_error,
            backend,
            roots: vec![hq_db::search::NOTEBOOKS_DIR.to_string()],
            last_notification_at: when(last_event),
            notifications,
            paths_ignored: ignored,
            events_queued: load(&self.events_in),
            coalesced: load(&self.coalesced),
            queue_depth: load(&self.pending),
            queue_overflows: load(&self.queue_overflows),
            watcher_overflows: overflows,
            dropped_events: dropped,
            watcher_errors: errors,
            notes_indexed: load(&self.notes_indexed),
            notes_removed: load(&self.notes_removed),
            notes_unchanged: load(&self.notes_unchanged),
            files_failed: load(&self.files_failed),
            folder_syncs: load(&self.folder_syncs),
            full_scans: load(&self.full_scans),
            retries: load(&self.retries),
            abandoned: load(&self.abandoned),
            last_scan_started_at: when(self.last_scan_started_ms.load(Ordering::Relaxed)),
            last_scan_finished_at: when(self.last_scan_finished_ms.load(Ordering::Relaxed)),
            updated_at: Utc::now().to_rfc3339(),
        }
    }
}

/// The report the daemon last wrote, if any.
pub fn read(vault_path: &Path) -> Option<VaultWatchStatus> {
    let raw = std::fs::read_to_string(vault_path.join("_system/.daemon-metrics.json")).ok()?;
    let metrics: serde_json::Value = serde_json::from_str(&raw).ok()?;
    serde_json::from_value(metrics.get(METRICS_KEY)?.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(backend: WatchBackend) -> PipelineStats {
        let stats = PipelineStats::default();
        *stats.watcher.lock().unwrap() = Some((backend, Arc::new(WatchStats::default())));
        stats
    }

    #[test]
    fn with_no_watcher_the_state_is_not_watching_and_says_why() {
        let stats = PipelineStats::default();
        *stats.last_error.lock().unwrap() = Some("Notebooks does not exist yet".into());
        let status = stats.snapshot();
        assert_eq!(status.state, WatchState::NotWatching);
        assert_eq!(
            status.detail.as_deref(),
            Some("Notebooks does not exist yet")
        );
        assert_eq!(status.backend, None);
    }

    #[test]
    fn the_index_is_not_called_current_until_the_first_scan_has_finished() {
        let stats = running(WatchBackend::Native);
        assert_eq!(stats.snapshot().state, WatchState::CatchingUp);
        stats
            .last_scan_finished_ms
            .store(1_760_000_000_000, Ordering::Relaxed);
        let status = stats.snapshot();
        assert_eq!(status.state, WatchState::Watching);
        assert_eq!(status.backend.as_deref(), Some("native"));
        assert!(status.last_scan_finished_at.is_some());
    }

    #[test]
    fn a_polling_watcher_is_reported_as_polling() {
        let stats = running(WatchBackend::Poll);
        stats
            .last_scan_finished_ms
            .store(1_760_000_000_000, Ordering::Relaxed);
        let status = stats.snapshot();
        assert_eq!(status.state, WatchState::Polling);
        assert_eq!(status.backend.as_deref(), Some("poll"));
    }

    #[test]
    fn counters_from_the_watcher_and_the_pipeline_both_appear() {
        let stats = running(WatchBackend::Native);
        stats.events_in.store(7, Ordering::Relaxed);
        stats.pending.store(2, Ordering::Relaxed);
        stats.abandoned.store(1, Ordering::Relaxed);
        {
            let slot = stats.watcher.lock().unwrap();
            let (_, watch) = slot.as_ref().unwrap();
            watch.overflows.store(3, Ordering::Relaxed);
            watch.dropped.store(5, Ordering::Relaxed);
            watch
                .last_notification_ms
                .store(1_760_000_000_000, Ordering::Relaxed);
        }
        let status = stats.snapshot();
        assert_eq!(
            (status.events_queued, status.queue_depth, status.abandoned),
            (7, 2, 1)
        );
        assert_eq!((status.watcher_overflows, status.dropped_events), (3, 5));
        assert!(status.last_notification_at.is_some());
        assert_eq!(status.roots, vec!["Notebooks".to_string()]);
    }

    #[test]
    fn the_report_survives_the_metrics_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("_system")).unwrap();
        assert_eq!(read(dir.path()), None);

        let status = running(WatchBackend::Native).snapshot();
        let metrics = serde_json::json!({ "tasks": {}, METRICS_KEY: status });
        std::fs::write(
            dir.path().join("_system/.daemon-metrics.json"),
            metrics.to_string(),
        )
        .unwrap();
        assert_eq!(read(dir.path()), Some(status));

        // A metrics file from a daemon that has no watcher yet simply has no report.
        std::fs::write(
            dir.path().join("_system/.daemon-metrics.json"),
            "{\"tasks\":{}}",
        )
        .unwrap();
        assert_eq!(read(dir.path()), None);
    }
}
