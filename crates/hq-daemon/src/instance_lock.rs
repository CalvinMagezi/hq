//! Daemon instance lock — prevents multiple daemon processes from running simultaneously.
//!
//! Uses atomic file creation (create_new) + PID-based liveness to ensure only one
//! daemon fires tasks at a time. Inspired by Claude Code's Kairos scheduler lock
//! (cronTasksLock.ts) and Agent HQ's existing consolidation lock in memory.rs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Lock file location relative to vault root.
const LOCK_FILE: &str = "_system/.daemon-lock";

/// If a lock file is older than this (seconds), consider it stale regardless of PID.
const STALE_THRESHOLD_SECS: u64 = 300; // 5 minutes (heartbeat touches every tick)

#[derive(Serialize, Deserialize)]
struct LockBody {
    pid: u32,
    acquired_at: String,
}

/// A held daemon lock. Dropping it releases the lock (best-effort).
pub struct DaemonLock {
    path: PathBuf,
}

impl DaemonLock {
    /// Try to acquire the daemon lock. Returns:
    /// - `Ok(Some(lock))` if we acquired it
    /// - `Ok(None)` if another live daemon holds the lock
    /// - `Err(...)` on I/O failure
    pub fn try_acquire(vault_path: &Path) -> Result<Option<Self>> {
        let path = vault_path.join(LOCK_FILE);

        // Ensure _system/ exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Check for existing lock
        if path.exists() {
            match Self::check_existing(&path) {
                LockStatus::HeldByLiveProcess(pid) => {
                    debug!(pid, "daemon lock held by live process");
                    return Ok(None);
                }
                LockStatus::Stale(reason) => {
                    info!(reason, "reclaiming stale daemon lock");
                    let _ = std::fs::remove_file(&path);
                }
            }
        }

        // Try atomic creation
        let body = LockBody {
            pid: std::process::id(),
            acquired_at: chrono::Utc::now().to_rfc3339(),
        };
        let body_json = serde_json::to_string_pretty(&body)?;

        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true) // O_EXCL equivalent — fails if file exists
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(body_json.as_bytes())?;
                drop(file);

                // Verify we won the race (re-read and check PID)
                if let Ok(contents) = std::fs::read_to_string(&path)
                    && let Ok(read_back) = serde_json::from_str::<LockBody>(&contents)
                    && read_back.pid == std::process::id()
                {
                    info!(pid = read_back.pid, "daemon lock acquired");
                    return Ok(Some(Self { path }));
                }
                // Lost the race somehow
                Ok(None)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Another process created the file between our check and create
                // Re-check if that process is alive
                match Self::check_existing(&path) {
                    LockStatus::HeldByLiveProcess(pid) => {
                        debug!(pid, "daemon lock acquired by another process during race");
                        Ok(None)
                    }
                    LockStatus::Stale(_) => {
                        // Rare: file appeared but is already stale. Don't recurse — just fail this attempt.
                        Ok(None)
                    }
                }
            }
            Err(e) => Err(e).context("failed to create daemon lock file"),
        }
    }

    /// Touch the lock file's mtime to signal liveness. Called every tick.
    pub fn heartbeat(&self) -> Result<()> {
        let now = filetime::FileTime::now();
        filetime::set_file_mtime(&self.path, now).context("failed to touch daemon lock")?;
        Ok(())
    }

    /// Explicitly release the lock by removing the file.
    pub fn release(self) {
        let _ = std::fs::remove_file(&self.path);
        info!("daemon lock released");
        // Skip Drop since we already removed it
        std::mem::forget(self);
    }

    /// PID of the live daemon holding the lock for this vault, if any.
    pub fn holder_pid(vault_path: &Path) -> Option<u32> {
        match Self::check_existing(&vault_path.join(LOCK_FILE)) {
            LockStatus::HeldByLiveProcess(pid) => Some(pid),
            LockStatus::Stale(_) => None,
        }
    }

    fn check_existing(path: &Path) -> LockStatus {
        // Check age first
        let age_secs = std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs());

        if let Some(age) = age_secs
            && age > STALE_THRESHOLD_SECS
        {
            return LockStatus::Stale(format!(
                "lock file is {age}s old (threshold: {STALE_THRESHOLD_SECS}s)"
            ));
        }

        // Check PID liveness
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(body) = serde_json::from_str::<LockBody>(&contents)
        {
            if is_process_alive(body.pid) {
                return LockStatus::HeldByLiveProcess(body.pid);
            }
            return LockStatus::Stale(format!("holder PID {} is dead", body.pid));
        }

        LockStatus::Stale("lock file unreadable".into())
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        // Best-effort cleanup on drop (e.g., panic unwind)
        if self.path.exists() {
            warn!("daemon lock dropped without explicit release, cleaning up");
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

enum LockStatus {
    HeldByLiveProcess(u32),
    Stale(String),
}

/// Check if a process is alive (Unix: kill(pid, 0)).
pub(crate) fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false // Assume dead on non-Unix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holder_pid_follows_the_lock() {
        let vault = tempfile::tempdir().unwrap();
        assert_eq!(DaemonLock::holder_pid(vault.path()), None);
        let lock = DaemonLock::try_acquire(vault.path()).unwrap().unwrap();
        assert_eq!(DaemonLock::holder_pid(vault.path()), Some(std::process::id()));
        lock.release();
        assert_eq!(DaemonLock::holder_pid(vault.path()), None);
    }
}
