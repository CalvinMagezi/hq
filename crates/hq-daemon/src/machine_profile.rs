//! Keeps `_system/MACHINE.md` current so every system prompt can state what
//! this host actually has installed.

use std::path::PathBuf;
use std::time::Duration;
use tracing::{info, warn};

/// How often the host is re-probed. Installs and `gh auth login` are the
/// events that matter, and neither is frequent.
const REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);

async fn refresh_once(vault_path: PathBuf) {
    // The probe fans out ~30 subprocesses; keep it off the async runtime.
    let result = tokio::task::spawn_blocking(move || hq_core::machine::refresh(&vault_path)).await;
    match result {
        Ok(Ok(profile)) => info!(
            binaries = profile.binaries.len(),
            missing = profile.missing.len(),
            gh_auth = profile.gh_auth.is_some(),
            "machine_profile: refreshed"
        ),
        Ok(Err(e)) => warn!("machine_profile: probe failed: {e}"),
        Err(e) => warn!("machine_profile: probe task panicked: {e}"),
    }
}

/// Probe once at startup, then every 30 minutes.
pub fn spawn_machine_profile_loop(vault_path: PathBuf) {
    tokio::spawn(async move {
        refresh_once(vault_path.clone()).await;

        let mut interval = tokio::time::interval(REFRESH_INTERVAL);
        interval.tick().await; // consume the immediate tick
        loop {
            interval.tick().await;
            refresh_once(vault_path.clone()).await;
        }
    });
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn refresh_once_writes_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        super::refresh_once(dir.path().to_path_buf()).await;
        let md = std::fs::read_to_string(dir.path().join("_system/MACHINE.md")).unwrap();
        assert!(md.starts_with("# Machine Profile"), "{md}");
        assert!(dir.path().join("_system/machine.json").exists());
    }
}
