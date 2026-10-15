//! The operating system side: registers a watch on `Notebooks/`, normalizes what it reports and
//! hands it to the pipeline over a bounded channel.
//!
//! Events are hints. When one cannot be delivered or understood (a full channel, a watcher
//! error, a kernel overflow) the watcher raises a rescan flag instead of losing the fact that
//! something happened, and the pipeline answers with a scan.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use hq_core::config::{VaultWatchConfig, VaultWatchMode};
use hq_db::search::NOTEBOOKS_DIR;
use notify::{Config, Event, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{Sender, error::TrySendError};
use tracing::{info, warn};

use super::normalize::{WatchEvent, normalize};

/// How changes are being detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchBackend {
    /// The operating system's file events.
    Native,
    /// Periodic scans of file times.
    Poll,
}

/// Counters the health report reads. All fields are written from the watcher's own thread.
#[derive(Debug, Default)]
pub struct WatchStats {
    pub notifications: AtomicU64,
    pub paths_ignored: AtomicU64,
    pub events_sent: AtomicU64,
    pub overflows: AtomicU64,
    /// Events that could not be queued because the channel was full.
    pub dropped: AtomicU64,
    pub errors: AtomicU64,
    /// Wall-clock milliseconds since the Unix epoch of the latest notification, 0 for none.
    pub last_notification_ms: AtomicI64,
    rescan_needed: AtomicBool,
}

impl WatchStats {
    /// Ask the pipeline for a full scan: something was lost or cannot be understood.
    pub fn request_rescan(&self) {
        self.rescan_needed.store(true, Ordering::SeqCst);
    }

    /// Whether a scan was requested since the last call, clearing the request.
    pub fn take_rescan(&self) -> bool {
        self.rescan_needed.swap(false, Ordering::SeqCst)
    }
}

enum Held {
    Native(#[allow(dead_code)] RecommendedWatcher),
    Poll(#[allow(dead_code)] PollWatcher),
}

/// A running watch on the vault's `Notebooks/` folder. Dropping it stops the watch.
pub struct VaultWatcher {
    pub backend: WatchBackend,
    pub stats: Arc<WatchStats>,
    /// The (canonical) vault directory event paths are relative to.
    pub root: PathBuf,
    _held: Held,
}

/// Linux under WSL2 serves Windows drives (`/mnt/c`) over a file system that delivers no events.
pub fn is_wsl_drive_mount(path: &Path, os_release: &str) -> bool {
    if !os_release.to_ascii_lowercase().contains("microsoft") {
        return false;
    }
    let mut parts = path.components().skip(1);
    let first = parts.next().and_then(|c| c.as_os_str().to_str());
    let drive = parts.next().and_then(|c| c.as_os_str().to_str());
    first == Some("mnt")
        && drive.is_some_and(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_alphabetic()))
}

/// Which backend to try first for `root`.
pub fn preferred_backend(mode: VaultWatchMode, root: &Path, os_release: &str) -> WatchBackend {
    match mode {
        VaultWatchMode::Native => WatchBackend::Native,
        VaultWatchMode::Poll => WatchBackend::Poll,
        VaultWatchMode::Auto if is_wsl_drive_mount(root, os_release) => WatchBackend::Poll,
        VaultWatchMode::Auto => WatchBackend::Native,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Queue one event, or raise the rescan flag if it cannot be queued.
fn deliver(tx: &Sender<WatchEvent>, stats: &WatchStats, event: WatchEvent) {
    if event == WatchEvent::Overflow {
        stats.overflows.fetch_add(1, Ordering::Relaxed);
        stats.request_rescan();
    }
    match tx.try_send(event) {
        Ok(()) => {
            stats.events_sent.fetch_add(1, Ordering::Relaxed);
        }
        Err(TrySendError::Full(_)) => {
            stats.dropped.fetch_add(1, Ordering::Relaxed);
            stats.request_rescan();
        }
        // The pipeline is gone (shutdown); nothing left to tell.
        Err(TrySendError::Closed(_)) => {}
    }
}

fn handler(
    root: PathBuf,
    tx: Sender<WatchEvent>,
    stats: Arc<WatchStats>,
) -> impl FnMut(notify::Result<Event>) + Send + 'static {
    move |result| {
        stats.notifications.fetch_add(1, Ordering::Relaxed);
        stats
            .last_notification_ms
            .store(now_ms(), Ordering::Relaxed);
        match result {
            Ok(event) => {
                let normalized = normalize(&root, &event);
                stats
                    .paths_ignored
                    .fetch_add(normalized.ignored, Ordering::Relaxed);
                for event in normalized.events {
                    deliver(&tx, &stats, event);
                }
            }
            Err(e) => {
                // We no longer know what changed.
                stats.errors.fetch_add(1, Ordering::Relaxed);
                warn!(error = %e, "vault watch: watcher reported an error");
                deliver(&tx, &stats, WatchEvent::Overflow);
            }
        }
    }
}

impl VaultWatcher {
    /// Watch `<vault_path>/Notebooks` and send what happens there to `tx`.
    ///
    /// In `auto` mode a native watch that cannot be set up (the Linux inotify limit, an
    /// unsupported file system) falls back to polling instead of failing.
    pub fn start(
        vault_path: &Path,
        config: &VaultWatchConfig,
        tx: Sender<WatchEvent>,
    ) -> Result<Self> {
        let root = std::fs::canonicalize(vault_path)
            .with_context(|| format!("vault directory {} is not readable", vault_path.display()))?;
        let notebooks = root.join(NOTEBOOKS_DIR);
        if !notebooks.is_dir() {
            bail!("{} does not exist yet", notebooks.display());
        }
        let stats = Arc::new(WatchStats::default());
        let os_release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        let first = preferred_backend(config.mode, &root, &os_release);

        let new_handler = || handler(root.clone(), tx.clone(), stats.clone());
        let held = match first {
            WatchBackend::Poll => {
                info!(
                    interval_secs = config.poll_interval().as_secs(),
                    "vault watch: polling for changes"
                );
                Held::Poll(start_poll(&notebooks, config, new_handler())?)
            }
            WatchBackend::Native => match start_native(&notebooks, new_handler()) {
                Ok(watcher) => Held::Native(watcher),
                Err(e) if config.mode == VaultWatchMode::Auto => {
                    warn!(error = %e, "vault watch: file events unavailable, polling instead");
                    Held::Poll(start_poll(&notebooks, config, new_handler())?)
                }
                Err(e) => return Err(e).context("could not start the native file watcher"),
            },
        };
        let backend = match held {
            Held::Native(_) => WatchBackend::Native,
            Held::Poll(_) => WatchBackend::Poll,
        };
        Ok(Self {
            backend,
            stats,
            root,
            _held: held,
        })
    }
}

fn start_native(
    notebooks: &Path,
    handler: impl FnMut(notify::Result<Event>) + Send + 'static,
) -> Result<RecommendedWatcher> {
    let mut watcher = RecommendedWatcher::new(handler, Config::default())?;
    watcher.watch(notebooks, RecursiveMode::Recursive)?;
    Ok(watcher)
}

fn start_poll(
    notebooks: &Path,
    config: &VaultWatchConfig,
    handler: impl FnMut(notify::Result<Event>) + Send + 'static,
) -> Result<PollWatcher> {
    let settings = Config::default()
        .with_poll_interval(config.poll_interval())
        .with_compare_contents(false);
    let mut watcher = PollWatcher::new(handler, settings)?;
    watcher.watch(notebooks, RecursiveMode::Recursive)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_windows_drive_under_wsl_is_polled() {
        let wsl = "5.15.153.1-microsoft-standard-WSL2";
        assert!(is_wsl_drive_mount(Path::new("/mnt/c/Users/me/vault"), wsl));
        assert!(is_wsl_drive_mount(Path::new("/mnt/D/vault"), wsl));
        assert!(!is_wsl_drive_mount(Path::new("/home/me/vault"), wsl));
        assert!(!is_wsl_drive_mount(Path::new("/mnt/data/vault"), wsl));
        assert!(!is_wsl_drive_mount(Path::new("/mnt"), wsl));
        assert!(!is_wsl_drive_mount(
            Path::new("/mnt/c/vault"),
            "6.1.0-generic"
        ));
    }

    #[test]
    fn auto_mode_prefers_native_except_where_events_never_arrive() {
        let wsl = "5.15-microsoft-standard-WSL2";
        let drive = Path::new("/mnt/c/vault");
        let home = Path::new("/home/me/vault");
        assert_eq!(
            preferred_backend(VaultWatchMode::Auto, home, wsl),
            WatchBackend::Native
        );
        assert_eq!(
            preferred_backend(VaultWatchMode::Auto, drive, wsl),
            WatchBackend::Poll
        );
        assert_eq!(
            preferred_backend(VaultWatchMode::Native, drive, wsl),
            WatchBackend::Native
        );
        assert_eq!(
            preferred_backend(VaultWatchMode::Poll, home, ""),
            WatchBackend::Poll
        );
    }

    #[test]
    fn a_full_channel_raises_the_rescan_flag_instead_of_losing_the_fact() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let stats = WatchStats::default();
        deliver(&tx, &stats, WatchEvent::Changed("Notebooks/a.md".into()));
        assert!(!stats.take_rescan());
        deliver(&tx, &stats, WatchEvent::Changed("Notebooks/b.md".into()));

        assert_eq!(stats.dropped.load(Ordering::Relaxed), 1);
        assert!(stats.take_rescan());
        assert!(!stats.take_rescan(), "the flag is cleared once taken");
        assert_eq!(
            rx.try_recv().unwrap(),
            WatchEvent::Changed("Notebooks/a.md".into())
        );
    }

    #[test]
    fn overflow_events_raise_the_flag_even_when_they_can_be_queued() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let stats = WatchStats::default();
        deliver(&tx, &stats, WatchEvent::Overflow);
        assert_eq!(stats.overflows.load(Ordering::Relaxed), 1);
        assert!(stats.take_rescan());
        assert_eq!(rx.try_recv().unwrap(), WatchEvent::Overflow);
    }

    #[test]
    fn a_closed_channel_is_not_an_error() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(rx);
        let stats = WatchStats::default();
        deliver(&tx, &stats, WatchEvent::Changed("Notebooks/a.md".into()));
        assert_eq!(stats.dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn start_fails_clearly_when_notebooks_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let err = VaultWatcher::start(dir.path(), &VaultWatchConfig::default(), tx)
            .err()
            .unwrap();
        assert!(err.to_string().contains("does not exist yet"), "{err}");
    }

    /// Needs a file system that delivers events and a few real seconds, so it is not part of the
    /// default suite. Run: `cargo test -p hq-daemon vault_watch -- --ignored`
    #[tokio::test]
    #[ignore = "uses real filesystem events and timing; run with --ignored"]
    async fn live_native_watch_reports_a_new_note_and_a_save_by_replacement() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks/Sub")).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let config = VaultWatchConfig {
            mode: VaultWatchMode::Native,
            ..VaultWatchConfig::default()
        };
        let watcher = VaultWatcher::start(dir.path(), &config, tx).unwrap();
        assert_eq!(watcher.backend, WatchBackend::Native);

        let notes = dir.path().join("Notebooks/Sub");
        std::fs::write(notes.join("a.md"), "one").unwrap();
        // A save by replacement: write a temp file, rename it over the note.
        std::fs::write(notes.join(".a.md.tmp"), "two").unwrap();
        std::fs::rename(notes.join(".a.md.tmp"), notes.join("a.md")).unwrap();

        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
                Ok(Some(event)) => seen.push(event),
                _ => {
                    if !seen.is_empty() {
                        break;
                    }
                }
            }
        }
        let touched_note = seen.iter().any(|e| {
            matches!(
                e,
                WatchEvent::Created(p) | WatchEvent::Changed(p)
                    if p == "Notebooks/Sub/a.md"
            )
        });
        assert!(touched_note, "no event for the note: {seen:?}");
    }

    #[tokio::test]
    #[ignore = "uses real filesystem timing; run with --ignored"]
    async fn live_poll_watch_reports_a_new_note() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let config = VaultWatchConfig {
            mode: VaultWatchMode::Poll,
            poll_secs: 2,
            ..VaultWatchConfig::default()
        };
        let watcher = VaultWatcher::start(dir.path(), &config, tx).unwrap();
        assert_eq!(watcher.backend, WatchBackend::Poll);

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::fs::write(dir.path().join("Notebooks/p.md"), "polled").unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            while let Some(event) = rx.recv().await {
                if matches!(&event, WatchEvent::Created(p) | WatchEvent::Changed(p) if p == "Notebooks/p.md")
                {
                    return Some(event);
                }
            }
            None
        })
        .await;
        assert!(matches!(event, Ok(Some(_))), "{event:?}");
    }
}
