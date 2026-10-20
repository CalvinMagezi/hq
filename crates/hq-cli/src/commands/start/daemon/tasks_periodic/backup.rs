//! Scheduled backups (`backup:` in the config, `docs/BACKUP.md`). Off by default.
//!
//! The task wakes hourly and does nothing unless the newest backup in `backup.dir` is older than
//! `backup.every_hours`. It seals with the key file only, never a passphrase, so no secret sits in
//! the config. After a new backup it keeps the newest `backup.keep` and deletes the rest.
//!
//! The backup runs detached from the scheduler: a big vault can take longer than any task
//! timeout, and a timed-out task would leave its work running while the next hour started a
//! second one. A flag allows one at a time, and the outcome is written to `backup-status.json`
//! beside the backups (`hq backup status`) because nothing else would show a failure.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use hq_backup::status::{self, Status};
use hq_backup::{Argon2Cost, BackupOptions, Secret};
use hq_core::config::{BackupConfig, HqConfig};
use tracing::{error, info, warn};

const SECS_PER_HOUR: u64 = 3600;

/// Set while a scheduled backup is running.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Clears the flag when the job ends, however it ends.
struct Running(&'static AtomicBool);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// How long until the next backup is due, or `None` if one is due now.
fn due_in(
    newest_written: Option<SystemTime>,
    every: Duration,
    now: SystemTime,
) -> Option<Duration> {
    let written = newest_written?;
    let age = now.duration_since(written).unwrap_or(Duration::ZERO);
    every.checked_sub(age).filter(|left| !left.is_zero())
}

fn check(cfg: &BackupConfig) -> Result<(&Path, &Path)> {
    let (Some(dir), Some(key_file)) = (cfg.dir.as_deref(), cfg.key_file.as_deref()) else {
        bail!("backup.enabled needs backup.dir and backup.key_file");
    };
    if cfg.keep == 0 {
        bail!("backup.keep must be at least 1");
    }
    if cfg.every_hours == 0 {
        bail!("backup.every_hours must be at least 1");
    }
    Ok((dir, key_file))
}

fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// One due backup, with everything it needs.
struct Job {
    config: HqConfig,
    dir: PathBuf,
    key_file: PathBuf,
}

struct Done {
    path: PathBuf,
    files: usize,
    bytes: u64,
    left_out: usize,
    /// Files or folders this user could not read: the backup is not a complete copy.
    unreadable: usize,
    /// How many older backups were deleted. None are, while the new one is incomplete.
    pruned: usize,
}

impl Job {
    /// An incomplete backup the last run recorded and left in place, if it is still there.
    fn previous_incomplete(&self) -> Option<PathBuf> {
        let last = status::read(&self.dir)?;
        let path = PathBuf::from(last.path.filter(|_| !last.ok)?);
        let named = path
            .file_name()?
            .to_str()
            .is_some_and(hq_backup::is_backup_name);
        (named && path.parent() == Some(self.dir.as_path()) && path.is_file()).then_some(path)
    }

    fn execute(&self) -> Result<Done> {
        let cfg = &self.config.backup;
        hq_backup::sweep_partials(&self.dir);
        let previous_incomplete = self.previous_incomplete();
        let secret =
            Secret::key_file(&self.key_file).context("the backup key file cannot be used")?;
        let db = self.config.db_path();
        let config_file = cfg.include_config.then(HqConfig::config_read_path);
        let report = hq_backup::create(&BackupOptions {
            vault: &self.config.vault_path,
            db: &db,
            config_file: config_file.as_deref(),
            out: &self.dir,
            secret: &secret,
            hq_version: env!("CARGO_PKG_VERSION"),
            cost: Argon2Cost::default(),
            verify: true,
        })?;
        // A newer backup replaces the incomplete one the last run left, so incomplete ones do not
        // pile up while a file stays unreadable.
        let mut pruned = 0;
        if let Some(old) = previous_incomplete.filter(|old| *old != report.path) {
            pruned += usize::from(std::fs::remove_file(old).is_ok());
        }
        // An incomplete backup must never push out a complete one, or a permissions problem would
        // erase every good backup within `keep` runs.
        if report.unreadable == 0 {
            pruned += hq_backup::prune(&self.dir, cfg.keep, Some(&report.path))?.len();
        }
        Ok(Done {
            path: report.path,
            files: report.files,
            bytes: report.archive_bytes,
            left_out: report.skipped.len(),
            unreadable: report.unreadable,
            pruned,
        })
    }

    /// Runs the backup and records how it went.
    fn run(&self) {
        let result = self.execute();
        let status = match &result {
            Ok(done) if done.unreadable > 0 => Status {
                finished: chrono::Utc::now().to_rfc3339(),
                ok: false,
                message: format!(
                    "wrote {} but it is INCOMPLETE: {} file(s) or folder(s) could not be read by the user HQ runs as, so no complete backup was deleted. Fix who can read them; the next backup replaces this one",
                    done.path.display(),
                    done.unreadable
                ),
                path: Some(done.path.display().to_string()),
                files: done.files,
                left_out: done.left_out,
            },
            Ok(done) => Status {
                finished: chrono::Utc::now().to_rfc3339(),
                ok: true,
                message: format!(
                    "wrote {} ({} files, {} bytes, {} left out), removed {} old",
                    done.path.display(),
                    done.files,
                    done.bytes,
                    done.left_out,
                    done.pruned
                ),
                path: Some(done.path.display().to_string()),
                files: done.files,
                left_out: done.left_out,
            },
            Err(e) => Status {
                finished: chrono::Utc::now().to_rfc3339(),
                ok: false,
                message: format!("{e:#}"),
                path: None,
                files: 0,
                left_out: 0,
            },
        };
        if let Err(e) = status::write(&self.dir, &status) {
            warn!(error = %e, "backup: could not record the outcome");
        }
        match result {
            Ok(done) if done.unreadable > 0 => error!(
                path = %done.path.display(),
                unreadable = done.unreadable,
                "backup: incomplete, some files could not be read; older backups kept"
            ),
            Ok(done) => info!(
                path = %done.path.display(),
                files = done.files,
                bytes = done.bytes,
                left_out = done.left_out,
                pruned = done.pruned,
                "backup: written and checked"
            ),
            Err(e) => error!(error = %format!("{e:#}"), "backup: failed"),
        }
    }
}

/// The backup that is due now, if any.
fn prepare(config: &HqConfig) -> Result<Option<Job>> {
    let cfg = &config.backup;
    if !cfg.enabled {
        return Ok(None);
    }
    let (dir, key_file) = check(cfg)?;
    ensure_private_dir(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let newest = hq_backup::newest(dir)?.map(|(_, written)| written);
    let every = Duration::from_secs(cfg.every_hours * SECS_PER_HOUR);
    if due_in(newest, every, SystemTime::now()).is_some() {
        return Ok(None);
    }
    Ok(Some(Job {
        config: config.clone(),
        dir: dir.to_path_buf(),
        key_file: key_file.to_path_buf(),
    }))
}

/// Records a failure that happened before any backup was tried (a missing key file, a setting that
/// is not allowed), so `hq backup status` shows it too. Needs the backup folder to exist, since the
/// status lives in it; a folder that cannot be made is only logged.
fn record_failure(config: &HqConfig, error: &anyhow::Error) {
    let Some(dir) = config.backup.dir.as_deref().filter(|d| d.is_dir()) else {
        return;
    };
    let status = Status {
        finished: chrono::Utc::now().to_rfc3339(),
        ok: false,
        message: format!("{error:#}"),
        path: None,
        files: 0,
        left_out: 0,
    };
    let _ = status::write(dir, &status);
}

fn start(config: &HqConfig, running: &'static AtomicBool) -> Result<()> {
    let job = match prepare(config) {
        Ok(Some(job)) => job,
        Ok(None) => return Ok(()),
        Err(e) => {
            record_failure(config, &e);
            return Err(e);
        }
    };
    if running.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let guard = Running(running);
    // Detached on purpose; see the module comment.
    drop(tokio::task::spawn_blocking(move || {
        let _guard = guard;
        job.run();
    }));
    Ok(())
}

pub async fn run_backup(config: &HqConfig) -> Result<()> {
    start(config, &RUNNING)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(SECS_PER_HOUR);

    #[test]
    fn a_backup_is_due_when_there_is_none_or_the_newest_is_old_enough() {
        let now = SystemTime::now();
        assert!(due_in(None, 24 * HOUR, now).is_none());
        assert!(due_in(Some(now - 25 * HOUR), 24 * HOUR, now).is_none());
        assert!(due_in(Some(now - 24 * HOUR), 24 * HOUR, now).is_none());
        assert_eq!(due_in(Some(now - 23 * HOUR), 24 * HOUR, now), Some(HOUR));
        // A clock set back is not "due"; the file is simply new.
        assert!(due_in(Some(now + HOUR), 24 * HOUR, now).is_some());
    }

    #[test]
    fn enabling_it_without_a_folder_a_key_or_a_sane_schedule_is_an_error() {
        let ok = BackupConfig {
            enabled: true,
            dir: Some("/tmp/backups".into()),
            key_file: Some("/tmp/key".into()),
            ..BackupConfig::default()
        };
        assert!(check(&ok).is_ok());
        for bad in [
            BackupConfig {
                dir: None,
                ..ok.clone()
            },
            BackupConfig {
                key_file: None,
                ..ok.clone()
            },
            BackupConfig {
                keep: 0,
                ..ok.clone()
            },
            BackupConfig {
                every_hours: 0,
                ..ok.clone()
            },
        ] {
            assert!(check(&bad).is_err());
        }
    }

    #[tokio::test]
    async fn it_does_nothing_unless_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let config = HqConfig {
            backup: BackupConfig {
                enabled: false,
                dir: Some(dir.path().join("never-made")),
                ..BackupConfig::default()
            },
            ..HqConfig::default()
        };
        run_backup(&config).await.unwrap();
        assert!(!dir.path().join("never-made").exists());
    }

    /// A vault with one note that has been quiet for a minute, its database, a key file and a
    /// config that turns scheduled backups on.
    fn setup(root: &Path, keep: usize) -> (HqConfig, PathBuf) {
        let vault = root.join("vault");
        std::fs::create_dir_all(vault.join("Notebooks")).unwrap();
        std::fs::write(vault.join("Notebooks/a.md"), "note").unwrap();
        // Quiet for the stability check: a freshly written file waits out a second.
        std::fs::File::options()
            .write(true)
            .open(vault.join("Notebooks/a.md"))
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        let mut config = HqConfig {
            vault_path: vault,
            ..HqConfig::default()
        };
        drop(hq_db::Database::open(&config.db_path()).unwrap());
        let key = root.join("k.key");
        Secret::generate_key_file(&key).unwrap();
        let backups = root.join("backups");
        config.backup = BackupConfig {
            enabled: true,
            dir: Some(backups.clone()),
            key_file: Some(key),
            every_hours: 24,
            keep,
            include_config: false,
        };
        (config, backups)
    }

    /// The whole path: a real vault and database, a key file, a due backup, retention. Nothing is
    /// due while the newest is fresh, and the oldest of the kept few is deleted when a new one lands.
    #[test]
    fn a_due_backup_is_written_checked_and_old_ones_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let (config, backups) = setup(dir.path(), 2);

        prepare(&config).unwrap().unwrap().run();
        let first = hq_backup::list(&backups).unwrap();
        assert_eq!(first.len(), 1);
        let recorded = status::read(&backups).expect("the outcome is recorded");
        assert!(recorded.ok && recorded.files == 1, "{recorded:?}");
        // Fresh: not due again.
        assert!(prepare(&config).unwrap().is_none());

        // Two older backups already there, as after a few days; the new one makes three, and
        // retention keeps the newest two.
        for stamp in ["20200101T000000Z", "20200102T000000Z"] {
            std::fs::write(backups.join(format!("hq-backup-{stamp}.hqbak")), b"old").unwrap();
        }
        let aged = SystemTime::now() - 30 * HOUR;
        for path in hq_backup::list(&backups).unwrap() {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(aged)
                .unwrap();
        }
        prepare(&config).unwrap().unwrap().run();
        let kept = hq_backup::list(&backups).unwrap();
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert!(
            kept.iter()
                .all(|p| !p.to_str().unwrap().contains("2020010")),
            "{kept:?}"
        );
    }

    #[test]
    fn a_failed_backup_is_recorded_where_hq_backup_status_will_find_it() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, backups) = setup(dir.path(), 2);
        config.backup.key_file = Some(dir.path().join("missing.key"));

        prepare(&config).unwrap().unwrap().run();
        let recorded = status::read(&backups).expect("the failure is recorded");
        assert!(
            !recorded.ok && recorded.message.contains("key file"),
            "{recorded:?}"
        );
        assert!(hq_backup::list(&backups).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn an_incomplete_backup_does_not_delete_the_complete_ones_and_is_reported_as_a_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (config, backups) = setup(dir.path(), 1);
        std::fs::create_dir_all(&backups).unwrap();
        let old = backups.join("hq-backup-20200101T000000Z.hqbak");
        std::fs::write(&old, b"a complete backup").unwrap();
        // Written a while ago, so a new one is due.
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::now() - 30 * HOUR)
            .unwrap();
        let locked = config.vault_path.join("Notebooks/locked.md");
        std::fs::write(&locked, "private").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&locked).is_ok() {
            return; // running as a user that can read anything: nothing to prove here
        }

        prepare(&config).unwrap().unwrap().run();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
        let recorded = status::read(&backups).expect("recorded");
        assert!(
            !recorded.ok && recorded.message.contains("INCOMPLETE"),
            "{recorded:?}"
        );
        assert!(recorded.left_out >= 1);
        assert!(old.exists(), "keep=1, but the older complete backup stays");
        let incomplete = recorded.path.clone().map(PathBuf::from).unwrap();
        assert!(incomplete.exists());
        assert_eq!(hq_backup::list(&backups).unwrap().len(), 2);

        // Fixed, and the next backup is due: it replaces the incomplete one, and retention then
        // applies to complete backups as usual.
        std::fs::File::options()
            .write(true)
            .open(&incomplete)
            .unwrap()
            .set_modified(SystemTime::now() - 30 * HOUR)
            .unwrap();
        prepare(&config).unwrap().unwrap().run();
        let recorded = status::read(&backups).expect("recorded");
        assert!(recorded.ok, "{recorded:?}");
        assert!(!incomplete.exists(), "the incomplete backup is replaced");
        let kept = hq_backup::list(&backups).unwrap();
        assert_eq!(kept.len(), 1, "keep=1 of complete backups: {kept:?}");
    }

    #[test]
    fn a_failure_before_any_backup_is_tried_is_recorded_too() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, backups) = setup(dir.path(), 2);
        std::fs::create_dir_all(&backups).unwrap();
        config.backup.every_hours = 0;
        assert!(prepare(&config).is_err());
        record_failure(&config, &prepare(&config).err().unwrap());
        let recorded = status::read(&backups).expect("recorded");
        assert!(
            !recorded.ok && recorded.message.contains("every_hours"),
            "{recorded:?}"
        );
    }

    static DETACHED: AtomicBool = AtomicBool::new(false);
    static BUSY: AtomicBool = AtomicBool::new(false);
    const WAIT_FOR_JOB: Duration = Duration::from_secs(60);

    /// The scheduler gets control back at once; the work finishes on its own and frees the flag.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_backup_runs_detached_and_one_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let (config, backups) = setup(dir.path(), 2);

        start(&config, &DETACHED).unwrap();
        let began = std::time::Instant::now();
        while status::read(&backups).is_none() || DETACHED.load(Ordering::SeqCst) {
            assert!(began.elapsed() < WAIT_FOR_JOB, "the job never finished");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(status::read(&backups).unwrap().ok);
        assert_eq!(hq_backup::list(&backups).unwrap().len(), 1);

        // A job already running: a due backup does not start a second one.
        let other = tempfile::tempdir().unwrap();
        let (config, backups) = setup(other.path(), 2);
        BUSY.store(true, Ordering::SeqCst);
        start(&config, &BUSY).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(hq_backup::list(&backups).unwrap().is_empty());
        assert!(
            BUSY.load(Ordering::SeqCst),
            "the running job's flag stays set"
        );
    }
}
