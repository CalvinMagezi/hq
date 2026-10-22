//! `verify-dispatch`: every minute, verify the tasks waiting for review and, once
//! `tasks.verification.mode` is `enforce`, complete the ones that pass
//! (`hq_tools::tasks::verify::dispatch`, docs/plans/task-verification.md). With the mode `off` it
//! does nothing.
//!
//! A pass can run for minutes (a reviewer call can take two), and the scheduler runs its tasks one
//! after another, so the pass runs detached and the tick returns at once. A tick that finds the
//! previous pass still running skips.

use anyhow::Result;
use hq_core::config::{HqConfig, VerificationMode};
use hq_db::Database;
use hq_tools::tasks::verify::{
    DispatchReport, Env, GhCli, LlmReviewer, NoReviewer, Reviewer, advance_sweeps,
    answer_second_opinions, dispatch,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How long one pass may run before the rest of the queue is left to the next one.
const PASS_DEADLINE: Duration = Duration::from_secs(20 * 60);

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Clears the running flag when a pass ends however it ends.
struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

pub async fn run_verify_dispatch(
    vault_path: &Path,
    db: &Database,
    config: &HqConfig,
) -> Result<()> {
    let cfg = &config.tasks.verification;
    // The daemon runs on a configuration it really loaded (a failed reload keeps the old one), so
    // it keeps the shared policy in step with it, turning verification off included. Other
    // processes record theirs only when they too read a file.
    if let Err(e) = db.with_conn(|c| hq_db::tasks::sync_verification_policy(c, cfg).map(|_| ())) {
        tracing::warn!(error = %e, "verify-dispatch: could not record the verification policy");
    }
    if cfg.mode == VerificationMode::Off {
        return Ok(());
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        tracing::debug!("verify-dispatch: the previous pass is still running");
        return Ok(());
    }
    let guard = Running;
    let vault_path: PathBuf = vault_path.to_path_buf();
    let db = db.clone();
    let config = config.clone();
    tokio::spawn(async move {
        let _guard = guard;
        match tokio::time::timeout(PASS_DEADLINE, pass(&vault_path, &db, &config)).await {
            Ok(Ok(report)) => log(&report),
            Ok(Err(e)) => tracing::warn!(error = %format!("{e:#}"), "verify-dispatch: pass failed"),
            Err(_) => tracing::warn!(
                minutes = PASS_DEADLINE.as_secs() / 60,
                "verify-dispatch: pass cut off; the rest waits for the next one"
            ),
        }
    });
    Ok(())
}

async fn pass(vault_path: &Path, db: &Database, config: &HqConfig) -> Result<DispatchReport> {
    let cfg = &config.tasks.verification;
    let gh = GhCli;
    let env = Env {
        db,
        gh: &gh,
        vault_path,
        cfg,
        route_tags: config.tasks.route_tags,
    };
    // The router is built only when a model may be called at all.
    let reviewer: Box<dyn Reviewer> = if cfg.reviewer_enabled() {
        Box::new(LlmReviewer::new(
            Arc::new(hq_llm::router::LlmRouter::from_env()),
            cfg.reviewers(),
            Duration::from_secs(cfg.reviewer_timeout_secs.max(5)),
        ))
    } else {
        Box::new(NoReviewer)
    };
    let report = dispatch(&env, reviewer.as_ref()).await?;
    // The live queue comes first; sweeps use what is left of the pass.
    match advance_sweeps(
        &env,
        reviewer.as_ref(),
        hq_tools::tasks::verify::sweep::BATCH,
    )
    .await
    {
        Ok(progress) => {
            for error in &progress.errors {
                tracing::warn!(%error, "verify-dispatch: a sweep could not advance");
            }
            if progress.planned + progress.verified + progress.applied > 0 {
                tracing::info!(
                    planned = progress.planned,
                    verified = progress.verified,
                    applied = progress.applied,
                    "verify-dispatch: sweeps advanced"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "verify-dispatch: sweeps could not advance")
        }
    }
    match answer_second_opinions(&env, reviewer.as_ref(), 3).await {
        Ok(n) if n > 0 => tracing::info!(answered = n, "verify-dispatch: second opinions answered"),
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "verify-dispatch: second opinions could not be answered")
        }
    }
    Ok(report)
}

fn log(report: &DispatchReport) {
    for error in &report.errors {
        tracing::warn!(%error, "verify-dispatch: a task could not be verified");
    }
    if report.attempted == 0 && report.released == 0 && report.changed_in_window == 0 {
        return;
    }
    tracing::info!(
        queued = report.queued,
        attempted = report.attempted,
        recorded = report.recorded,
        completed = report.completed,
        returned = report.returned,
        escalated = report.escalated,
        held = report.held,
        released = report.released,
        deferred = report.deferred,
        voided = report.voided,
        "verify-dispatch: pass done"
    );
}
