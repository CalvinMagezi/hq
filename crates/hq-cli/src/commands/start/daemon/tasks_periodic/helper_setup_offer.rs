//! `helper-setup-offer`: every half hour, checks whether a computer is connected that has none of
//! the free coding helpers, and if so leaves one item in the owner's inbox offering to set them
//! up. Approving the item starts the install and the sign-ins; this task installs nothing
//! (`docs/HELPER_SETUP.md`). Off with `daemon.helper_setup_offer: false`.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use tracing::info;

pub async fn run_helper_setup_offer(db: &Database, config: &HqConfig) -> Result<()> {
    if !config.daemon.helper_setup_offer {
        return Ok(());
    }
    let job_db = db.clone();
    // Asking the hosts blocks on a socket or a subprocess.
    let item =
        tokio::task::spawn_blocking(move || hq_tools::helper_setup::offer(&job_db)).await??;
    if let Some(item) = item {
        hq_db::value_items::emit(db, &item)?;
        info!("helper-setup-offer: offered to set up coding helpers");
    }
    Ok(())
}
