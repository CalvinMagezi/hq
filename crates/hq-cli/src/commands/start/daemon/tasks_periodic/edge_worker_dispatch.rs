//! `edge-worker-dispatch`: every two minutes, start HQ on the next open `edge-worker` task when
//! `github_write.dispatch_tasks` is on (`hq_tools::github::dispatch`, docs/plans/edge-coding-worker.md).

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_tools::github::dispatch::{Outcome, dispatch};
use std::sync::Arc;

pub async fn run_edge_worker_dispatch(db: &Database, config: &HqConfig) -> Result<()> {
    match dispatch(
        Arc::new(db.clone()),
        config,
        hq_tools::ask::shared_runner_cell(),
    )
    .await?
    {
        Outcome::Started { task, ask_id } => {
            tracing::info!(%task, %ask_id, "edge-worker-dispatch: HQ is working the task");
        }
        Outcome::AlreadyAsked { task, ask_id } => {
            tracing::debug!(%task, %ask_id, "edge-worker-dispatch: already asked, waiting for HQ to claim it");
        }
        Outcome::Busy(task) => {
            tracing::debug!(%task, "edge-worker-dispatch: a task is in progress")
        }
        Outcome::Off | Outcome::Idle => {}
    }
    Ok(())
}
