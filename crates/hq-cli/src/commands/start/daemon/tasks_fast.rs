//! Fast-cycle daemon tasks (every 1 minute).

use anyhow::Result;
use std::path::Path;
use tracing::info;


/// Expire pending approval files older than 5 minutes.
pub async fn run_expire_approvals(vault_path: &Path) -> Result<()> {
    let pending_dir = vault_path.join("_approvals").join("pending");
    if pending_dir.exists() {
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(300);
        let mut expired = 0u32;
        if let Ok(entries) = std::fs::read_dir(&pending_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata()
                    && let Ok(modified) = meta.modified()
                    && modified < cutoff
                {
                    let _ = std::fs::remove_file(entry.path());
                    expired += 1;
                }
            }
        }
        if expired > 0 {
            info!(
                count = expired,
                "expire-approvals: removed expired approvals"
            );
        }
    }
    Ok(())
}
