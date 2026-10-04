//! Harness heartbeat files under `.vault/_system/heartbeats/`: reading them
//! and reclaiming dead ones. Nothing writes new heartbeats any more.

use crate::types::HarnessHeartbeat;
use anyhow::Result;
use chrono::Utc;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Subdirectory for heartbeat files.
const HEARTBEAT_DIR: &str = "_system/heartbeats";

/// Validate that a harness ID is safe to use as a path component.
fn validate_harness_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains('\0')
    {
        anyhow::bail!("invalid harness_id: must not contain path separators or '..'");
    }
    Ok(())
}

/// Path to a harness's heartbeat file.
fn heartbeat_path(vault_path: &Path, harness_id: &str) -> Result<PathBuf> {
    validate_harness_id(harness_id)?;
    Ok(vault_path
        .join(HEARTBEAT_DIR)
        .join(format!("{}.json", harness_id)))
}

/// Remove heartbeat on clean shutdown.
pub fn clear_heartbeat(vault_path: &Path, harness_id: &str) -> Result<()> {
    let path = heartbeat_path(vault_path, harness_id)?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Read all raw heartbeat files in the vault.
pub fn read_all_heartbeats(vault_path: &Path) -> Result<Vec<HarnessHeartbeat>> {
    let dir = vault_path.join(HEARTBEAT_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut heartbeats = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<HarnessHeartbeat>(&content) {
                    Ok(hb) => heartbeats.push(hb),
                    Err(e) => warn!(path = %path.display(), error = %e, "malformed heartbeat"),
                },
                Err(e) => warn!(path = %path.display(), error = %e, "failed to read heartbeat"),
            }
        }
    }

    Ok(heartbeats)
}

/// Check if a process is alive by PID.
pub fn is_pid_alive(pid: u32) -> bool {
    // On Unix, kill(pid, 0) checks if process exists without sending a signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Detect dead harnesses: those with stale heartbeats or dead PIDs.
///
/// Returns list of (harness_id, job_id) pairs that should be reclaimed.
pub fn detect_dead_harnesses(
    vault_path: &Path,
    stale_threshold_secs: u64,
) -> Result<Vec<(String, Option<String>)>> {
    let heartbeats = read_all_heartbeats(vault_path)?;
    let now = Utc::now();
    let threshold = chrono::Duration::seconds(stale_threshold_secs as i64);
    let mut dead = Vec::new();

    let local_device_id = crate::device_id();

    for hb in &heartbeats {
        let is_stale = (now - hb.last_heartbeat) > threshold;
        let is_remote = hb
            .device_id
            .as_deref()
            .is_some_and(|d| d != local_device_id);
        // Remote heartbeats: only stale counts. Local: stale OR PID dead.
        let pid_dead = if is_remote {
            false
        } else {
            !is_pid_alive(hb.pid)
        };

        if is_stale || pid_dead {
            info!(
                harness = %hb.harness_id,
                pid = hb.pid,
                is_stale,
                pid_dead,
                job_id = ?hb.current_job_id,
                "detected dead harness"
            );

            dead.push((hb.harness_id.clone(), hb.current_job_id.clone()));

            // Clean up the heartbeat file
            let path = match heartbeat_path(vault_path, &hb.harness_id) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if let Err(e) = std::fs::remove_file(&path) {
                warn!(error = %e, "failed to clean up dead heartbeat");
            }
        }
    }

    Ok(dead)
}
