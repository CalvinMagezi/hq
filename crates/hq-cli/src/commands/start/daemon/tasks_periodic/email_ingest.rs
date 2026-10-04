//! Time-gated fallback for Gmail ingestion. `/hooks/gmail` (Pub/Sub push) is
//! primary; this tick covers companies/environments without push configured.

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_web::gmail_ingest::PollOutcome;
use std::path::Path;
use tracing::info;

use crate::commands::start::daemon::TaskUnconfigured;

pub async fn run_email_poll(vault_path: &Path, config: &HqConfig) -> Result<()> {
    match hq_web::gmail_ingest::poll_and_enqueue(vault_path, config).await? {
        PollOutcome::Polled(n) => {
            if n > 0 {
                info!(enqueued = n, "email-poll: queued triage events");
            }
            Ok(())
        }
        PollOutcome::Unconfigured(reason) => Err(TaskUnconfigured(reason).into()),
    }
}
