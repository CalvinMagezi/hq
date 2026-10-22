//! The work behind a sweep (docs/plans/task-verification.md, "Clearing a queue on demand").
//!
//! A sweep is a persisted batch over the tasks waiting for review. Each phase is a status in the
//! database that [`advance_sweeps`] moves a few items at a time, called from the daemon every
//! minute, so a sweep survives a restart and no web request waits for GitHub or a model:
//!
//! 1. `planning`: read each task's evidence and settle what the facts settle. No model, no spend.
//!    Every task lands in a bucket, and the plan shows counts and an estimate.
//! 2. `running` (the owner starts it): verdicts are produced in priority order and recorded, with
//!    the reviewer where one is needed, and nothing moves.
//! 3. `ready`: the owner reads the buckets, not the tasks.
//! 4. `applying` (the owner confirms): the policy is applied to the buckets, each task checked
//!    again first because the work may have changed since.
//!
//! Undo restores every task a sweep moved that has not changed since, and reports the rest.

use anyhow::Result;
use hq_db::tasks as t;
use serde_json::{Value, json};

use super::dispatch::review_allowed;
use super::pipeline::{
    Currency, Disposition, Env, Evidence, Finished, Gate, Resolution, WaitingFor, autonomy_hold,
    bar_changed, clock, completion_note, gate, gather, still_current, verify_task_as,
};
use super::reviewer::Reviewer;

/// Items worked per tick across all sweeps, so one tick ends in bounded time.
pub const BATCH: usize = 20;
/// Times an item that could not be decided is tried before it is shown as undecidable.
const MAX_TRIES: i64 = 3;
/// Seconds before a deferred item is tried again.
const RETRY_SECS: i64 = 600;
/// What one reviewer run is assumed to cost when the ledger has none to learn from.
const FALLBACK_RUN_USD: f64 = 0.05;
/// Seconds one reviewer run is assumed to take.
const RUN_SECS: f64 = 40.0;

/// What one tick of sweep work did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepProgress {
    pub planned: usize,
    pub verified: usize,
    pub applied: usize,
    pub errors: Vec<String>,
}

/// Advances the sweeps that are planning, running or applying by at most `budget` items in all.
pub async fn advance_sweeps(
    env: &Env<'_>,
    reviewer: &dyn Reviewer,
    budget: usize,
) -> Result<SweepProgress> {
    let mut progress = SweepProgress::default();
    let mut left = budget;
    for sweep in env.db.with_conn(t::active_sweeps)? {
        if left == 0 {
            break;
        }
        let worked = match sweep.status.as_str() {
            t::SWEEP_PLANNING => plan_batch(env, &sweep, left).await,
            t::SWEEP_RUNNING => run_batch(env, reviewer, &sweep, left).await,
            t::SWEEP_APPLYING => apply_batch(env, &sweep, left).await,
            _ => Ok((0, 0)),
        };
        match worked {
            Ok((n, _)) => {
                left = left.saturating_sub(n.max(1));
                match sweep.status.as_str() {
                    t::SWEEP_PLANNING => progress.planned += n,
                    t::SWEEP_RUNNING => progress.verified += n,
                    _ => progress.applied += n,
                }
            }
            Err(e) => progress.errors.push(format!("sweep {}: {e:#}", sweep.id)),
        }
    }
    Ok(progress)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The task, if it is still waiting for review as the same submission the sweep looked at.
fn still_waiting(env: &Env<'_>, item: &t::SweepItem) -> Result<Result<t::Task, String>> {
    let item = item.clone();
    env.db.with_conn(move |c| {
        if let Some(why) = t::still_the_same_submission(c, &item)? {
            return Ok(Err(why));
        }
        Ok(t::get_task(c, &item.task_id)?.ok_or_else(|| "the task is gone".to_string()))
    })
}

// ---- planning ----

/// The bucket the facts alone put a task in, and why.
fn provisional(ev: &Evidence, gated: &Gate) -> (&'static str, String) {
    match gated {
        Gate::NeedsReview { .. } => (
            t::BUCKET_NEEDS_REVIEW,
            "needs a reviewer's judgement".into(),
        ),
        Gate::Settled(Resolution::Pass { summary, .. }) => (t::BUCKET_PASSED, summary.clone()),
        Gate::Settled(Resolution::Fail { reasons, .. }) => (t::BUCKET_FAILED, reasons.clone()),
        Gate::Settled(Resolution::Inconclusive {
            reasons,
            waiting_for,
        }) => (
            match waiting_for {
                WaitingFor::Worker if ev.artifacts.is_empty() => t::BUCKET_NO_ARTIFACT,
                WaitingFor::Worker => t::BUCKET_FAILED,
                WaitingFor::Owner => t::BUCKET_NEEDS_YOU,
                _ => t::BUCKET_UNDECIDABLE,
            },
            reasons.clone(),
        ),
        Gate::Settled(Resolution::Defer { reason, .. }) => (t::BUCKET_UNDECIDABLE, reason.clone()),
    }
}

async fn plan_batch(env: &Env<'_>, sweep: &t::Sweep, limit: usize) -> Result<(usize, usize)> {
    let id = sweep.id.clone();
    let items = env
        .db
        .with_conn(move |c| t::items_to_work(c, &id, t::ITEM_UNPLANNED, now(), limit))?;
    let mut done = 0;
    for chunk in items.chunks(env.cfg.parallelism()) {
        let looks = chunk.iter().map(|item| async move {
            match still_waiting(env, item)? {
                Err(why) => Ok::<_, anyhow::Error>((item, Err(why))),
                Ok(task) => {
                    let ev = gather(env, &task).await?;
                    let gated = gate(&ev, env.cfg);
                    let (bucket, finding) = provisional(&ev, &gated);
                    Ok((item, Ok((bucket, finding, ev.currency.to_json()))))
                }
            }
        });
        for result in futures::future::join_all(looks).await {
            let (item, outcome) = match result {
                Ok(v) => v,
                Err(e) => {
                    // Not being able to read one task leaves it unplanned for the next tick.
                    tracing::warn!(error = %format!("{e:#}"), "sweep: a task could not be read");
                    continue;
                }
            };
            let (sweep_id, task_id) = (item.sweep_id.clone(), item.task_id.clone());
            env.db.with_conn(move |c| match outcome {
                Ok((bucket, finding, currency)) => {
                    t::plan_item(c, &sweep_id, &task_id, bucket, &finding, &currency)
                }
                Err(why) => t::set_aside(
                    c,
                    &sweep_id,
                    &task_id,
                    t::BUCKET_UNDECIDABLE,
                    t::ITEM_KEPT,
                    &format!("not planned: {why}"),
                ),
            })?;
            done += 1;
        }
    }
    let id = sweep.id.clone();
    let finished = env
        .db
        .with_conn(move |c| Ok(!t::any_items_in(c, &id, t::ITEM_UNPLANNED)?))?;
    if finished {
        let (id, cfg) = (sweep.id.clone(), env.cfg.clone());
        env.db.with_conn(move |c| {
            t::in_write_tx(c, |c| {
                let plan = plan_summary(c, &cfg, &id)?;
                t::set_sweep_plan(c, &id, &plan, "")?;
                t::set_sweep_status(c, &id, &[t::SWEEP_PLANNING], t::SWEEP_PLANNED).map(|_| ())
            })
        })?;
    }
    Ok((done, 0))
}

/// Counts per bucket, an estimate of what the reviewer runs would cost, and how many tasks would
/// wait on the owner.
pub fn plan_summary(
    conn: &rusqlite::Connection,
    cfg: &hq_core::config::VerificationConfig,
    sweep_id: &str,
) -> Result<Value> {
    let counts = t::sweep_summary(conn, sweep_id)?;
    let items = t::sweep_items(conn, sweep_id)?;
    let needs_review: Vec<&t::SweepItem> = items
        .iter()
        .filter(|i| i.bucket == t::BUCKET_NEEDS_REVIEW)
        .collect();
    // A tier 2 task needs two reviewers.
    let runs: usize = needs_review
        .iter()
        .map(|i| if i.tier >= t::MAX_TIER { 2 } else { 1 })
        .sum();
    let (spent, recorded) = (
        t::reviewer_spend_this_month(conn)?,
        t::reviewer_runs_recorded(conn)?,
    );
    let per_run = if recorded > 0 && spent > 0.0 {
        spent / recorded as f64
    } else {
        FALLBACK_RUN_USD
    };
    let basis = if recorded > 0 && spent > 0.0 {
        format!("the average of {recorded} reviewer runs this month")
    } else {
        format!("an assumed ${FALLBACK_RUN_USD:.2} a run: no reviewer runs to learn from yet")
    };
    let tier2 = items.iter().filter(|i| i.tier >= t::MAX_TIER).count();
    let waits_on_you = items
        .iter()
        .filter(|i| i.bucket == t::BUCKET_NEEDS_YOU)
        .count()
        + items
            .iter()
            .filter(|i| {
                i.tier >= t::MAX_TIER
                    && i.bucket != t::BUCKET_NEEDS_YOU
                    && matches!(i.bucket.as_str(), t::BUCKET_PASSED | t::BUCKET_NEEDS_REVIEW)
            })
            .count();
    let ceiling = cfg.reviewer_monthly_ceiling_usd;
    Ok(json!({
        "total": counts["total"],
        "buckets": counts["buckets"],
        "tier2": tier2,
        "would_wait_on_you": waits_on_you,
        "estimate": {
            "reviewer_runs": runs,
            "usd": (runs as f64 * per_run * 100.0).round() / 100.0,
            "minutes": ((runs as f64 * RUN_SECS) / cfg.parallelism() as f64 / 60.0).ceil(),
            "basis": basis,
            "reviewer_enabled": cfg.reviewer_enabled(),
            "monthly_ceiling_usd": ceiling,
            "spent_this_month_usd": (spent * 100.0).round() / 100.0,
        },
    }))
}

// ---- running ----

/// The bucket a recorded verdict puts a task in.
fn final_bucket(v: &t::Verdict, cfg: &hq_core::config::VerificationConfig) -> &'static str {
    match v.verdict.as_str() {
        t::VERDICT_PASS => {
            if v.waiting_for.as_deref() == Some(t::WAIT_OWNER)
                || (v.tier >= t::MAX_TIER && cfg.veto_hours > 0)
            {
                t::BUCKET_NEEDS_YOU
            } else {
                t::BUCKET_PASSED
            }
        }
        t::VERDICT_FAIL => t::BUCKET_FAILED,
        _ => match v.waiting_for.as_deref() {
            Some(t::WAIT_WORKER) if v.artifact == "none" => t::BUCKET_NO_ARTIFACT,
            Some(t::WAIT_WORKER) => t::BUCKET_FAILED,
            Some(t::WAIT_OWNER) => t::BUCKET_NEEDS_YOU,
            _ => t::BUCKET_UNDECIDABLE,
        },
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

async fn run_batch(
    env: &Env<'_>,
    reviewer: &dyn Reviewer,
    sweep: &t::Sweep,
    limit: usize,
) -> Result<(usize, usize)> {
    let id = sweep.id.clone();
    let items = env
        .db
        .with_conn(move |c| t::items_to_work(c, &id, t::ITEM_PLANNED, now(), limit))?;
    let mut done = 0;
    for item in items {
        // Pausing or cancelling takes effect between tasks.
        let id = sweep.id.clone();
        let current = env.db.with_conn(move |c| t::get_sweep(c, &id))?;
        if current.is_none_or(|s| s.status != t::SWEEP_RUNNING) {
            break;
        }
        let task = match still_waiting(env, &item)? {
            Ok(task) => task,
            Err(why) => {
                let (sweep_id, task_id) = (item.sweep_id.clone(), item.task_id.clone());
                env.db.with_conn(move |c| {
                    t::set_aside(
                        c,
                        &sweep_id,
                        &task_id,
                        t::BUCKET_UNDECIDABLE,
                        t::ITEM_KEPT,
                        &format!("not verified: {why}"),
                    )
                })?;
                done += 1;
                continue;
            }
        };
        let allow_review = review_allowed(env)?;
        let result = verify_task_as(env, reviewer, &task, allow_review, Disposition::Record).await;
        let (sweep_id, task_id, cfg) =
            (item.sweep_id.clone(), item.task_id.clone(), env.cfg.clone());
        match result {
            Ok(Finished::Recorded { verdict, .. }) => {
                let bucket = final_bucket(&verdict, &cfg);
                let finding = first_line(&verdict.reasons);
                let currency = Currency::from_observations(&verdict.observations).to_json();
                env.db.with_conn(move |c| {
                    t::in_write_tx(c, |c| {
                        t::record_item_verdict(
                            c,
                            &sweep_id,
                            &task_id,
                            Some(&verdict.id),
                            bucket,
                            &finding,
                            &currency,
                        )
                    })
                })?;
            }
            Ok(Finished::Deferred { reason, .. }) => defer(env, &item, &reason)?,
            Ok(Finished::Voided { reason }) => {
                env.db.with_conn(move |c| {
                    t::set_aside(
                        c,
                        &sweep_id,
                        &task_id,
                        t::BUCKET_UNDECIDABLE,
                        t::ITEM_KEPT,
                        &format!("not verified: {reason}"),
                    )
                })?;
            }
            Err(e) => defer(env, &item, &format!("{e:#}"))?,
        }
        done += 1;
    }
    let id = sweep.id.clone();
    let finished = env
        .db
        .with_conn(move |c| Ok(!t::any_items_in(c, &id, t::ITEM_PLANNED)?))?;
    if finished {
        let (id, cfg) = (sweep.id.clone(), env.cfg.clone());
        env.db.with_conn(move |c| {
            t::in_write_tx(c, |c| {
                let plan = plan_summary(c, &cfg, &id)?;
                t::set_sweep_plan(c, &id, &plan, "")?;
                // A pause or cancel that landed meanwhile is respected.
                match t::set_sweep_status(c, &id, &[t::SWEEP_RUNNING], t::SWEEP_READY) {
                    Ok(_) | Err(_) => Ok(()),
                }
            })
        })?;
    }
    Ok((done, 0))
}

/// Puts an item back for a later try; after [`MAX_TRIES`] it is shown as undecidable with why.
fn defer(env: &Env<'_>, item: &t::SweepItem, reason: &str) -> Result<()> {
    let (sweep_id, task_id, reason) = (
        item.sweep_id.clone(),
        item.task_id.clone(),
        reason.to_string(),
    );
    env.db.with_conn(move |c| {
        t::in_write_tx(c, |c| {
            let tries = t::defer_item(c, &sweep_id, &task_id, now() + RETRY_SECS, &reason)?;
            if tries >= MAX_TRIES {
                t::record_item_verdict(
                    c,
                    &sweep_id,
                    &task_id,
                    None,
                    t::BUCKET_UNDECIDABLE,
                    &format!(
                        "could not be decided after {tries} tries: {}",
                        first_line(&reason)
                    ),
                    &json!({}),
                )?;
            }
            Ok(())
        })
    })
}

// ---- applying ----

async fn apply_batch(env: &Env<'_>, sweep: &t::Sweep, limit: usize) -> Result<(usize, usize)> {
    let id = sweep.id.clone();
    let items = env
        .db
        .with_conn(move |c| t::items_to_work(c, &id, t::ITEM_VERDICT, i64::MAX, limit))?;
    let mut done = 0;
    for item in items {
        let outcome = apply_item(env, sweep, &item).await;
        let (sweep_id, task_id) = (item.sweep_id.clone(), item.task_id.clone());
        match outcome {
            Ok(Applied::Moved {
                status,
                updated_at,
                detail,
            }) => env.db.with_conn(move |c| {
                t::mark_item(
                    c,
                    &sweep_id,
                    &task_id,
                    t::ITEM_APPLIED,
                    &detail,
                    Some((&status, &updated_at)),
                )
            })?,
            Ok(Applied::Kept(why)) => env.db.with_conn(move |c| {
                t::mark_item(c, &sweep_id, &task_id, t::ITEM_KEPT, &why, None)
            })?,
            Ok(Applied::Stale(why)) => env.db.with_conn(move |c| {
                t::mark_item(c, &sweep_id, &task_id, t::ITEM_STALE, &why, None)
            })?,
            Err(e) => env.db.with_conn(move |c| {
                t::mark_item(
                    c,
                    &sweep_id,
                    &task_id,
                    t::ITEM_KEPT,
                    &format!("could not be applied: {e:#}"),
                    None,
                )
            })?,
        }
        done += 1;
    }
    let id = sweep.id.clone();
    let finished = env
        .db
        .with_conn(move |c| Ok(!t::any_items_in(c, &id, t::ITEM_VERDICT)?))?;
    if finished {
        let id = sweep.id.clone();
        env.db.with_conn(move |c| {
            t::set_sweep_status(c, &id, &[t::SWEEP_APPLYING], t::SWEEP_APPLIED).map(|_| ())
        })?;
    }
    Ok((done, 0))
}

enum Applied {
    Moved {
        status: String,
        updated_at: String,
        detail: String,
    },
    /// Left where it is, with why.
    Kept(String),
    /// The work changed since the verdict, so it was not applied.
    Stale(String),
}

async fn apply_item(env: &Env<'_>, sweep: &t::Sweep, item: &t::SweepItem) -> Result<Applied> {
    let task = match still_waiting(env, item)? {
        Ok(task) => task,
        Err(why) => return Ok(Applied::Stale(format!("not applied: {why}"))),
    };
    let Some(verdict_id) = item.verdict_id.clone() else {
        return Ok(Applied::Kept("left where it is".into()));
    };
    let Some(verdict) = env.db.with_conn(move |c| t::get_verdict(c, &verdict_id))? else {
        return Ok(Applied::Kept("its verdict is gone".into()));
    };
    // The verdict's tier is the one that counts: a diff scan can raise a task to tier 2 only once
    // it is reviewed, after the plan was made.
    let held_pass = item.bucket == t::BUCKET_NEEDS_YOU
        && verdict.verdict == t::VERDICT_PASS
        && verdict.tier >= t::MAX_TIER;
    let moves = matches!(
        item.bucket.as_str(),
        t::BUCKET_PASSED | t::BUCKET_FAILED | t::BUCKET_NO_ARTIFACT
    ) || held_pass;
    if !moves {
        return Ok(Applied::Kept("left where it is".into()));
    }
    let verifier = format!("verify:sweep-{}", sweep.id);
    let (max_attempts, veto_hours, route_tags, vault_path) = (
        env.cfg.max_attempts,
        env.cfg.veto_hours,
        env.route_tags,
        env.vault_path,
    );

    if verdict.verdict == t::VERDICT_PASS {
        if !verdict.applied {
            return Ok(Applied::Kept(
                "its verdict was not approved for completion (verification was not enforcing, or the tier was held for you)".into(),
            ));
        }
        let currency = Currency::from_observations(&verdict.observations);
        if let Some(why) = still_current(env, &currency).await {
            return Ok(Applied::Stale(format!("not applied: {why}")));
        }
        if let Some(why) = bar_changed(env, &task, &verdict)? {
            return Ok(Applied::Stale(format!("not applied: {why}")));
        }
        if let Some(why) = autonomy_hold(env, verdict.tier) {
            return Ok(Applied::Kept(format!(
                "held for you: the verifier is not trusted to complete this tier on its own right now ({why})"
            )));
        }
        let (sweep_id, hold) = (
            sweep.id.clone(),
            verdict.tier >= t::MAX_TIER && veto_hours > 0,
        );
        let task_id = task.id.clone();
        let outcome = env.db.with_conn(move |c| {
            t::in_write_tx(c, |c| {
                if hold {
                    let at = now() + i64::from(veto_hours) * 3600;
                    t::create_hold(c, &task_id, &verdict.id, at, &currency.to_json())?;
                    t::add_comment(
                        c,
                        &task_id,
                        "verifier",
                        &format!(
                            "Sweep {sweep_id}: verified ({} evidence, tier {}). Because this is tier 2 it \
                             completes at {} UTC unless you veto it: `hq verify veto {}`.",
                            verdict.level,
                            verdict.tier,
                            clock(at),
                            t::get_task(c, &task_id)?.map(|t| t.display_id).unwrap_or_default()
                        ),
                        None,
                    )?;
                    let task = t::get_task(c, &task_id)?
                        .ok_or_else(|| anyhow::anyhow!("task vanished"))?;
                    return Ok((task, Vec::new(), Some(at)));
                }
                let ctx = t::WriteCtx {
                    actor: Some(&verifier),
                    verdict_id: Some(&verdict.id),
                    ..Default::default()
                };
                let done = t::update_task_as(
                    c,
                    &task_id,
                    &t::TaskPatch {
                        status: Some(t::STATUS_COMPLETE.to_string()),
                        ..Default::default()
                    },
                    Some(t::STATUS_READY_FOR_REVIEW),
                    &ctx,
                )?;
                t::add_comment(
                    c,
                    &task_id,
                    "verifier",
                    &format!("Sweep {sweep_id}. {}", completion_note(&verdict)),
                    None,
                )?;
                let unblocked = crate::tasks::unblocked_by_transition(
                    c,
                    Some(t::STATUS_READY_FOR_REVIEW),
                    &done,
                )?;
                let done = t::get_task(c, &task_id)?.unwrap_or(done);
                Ok((done, unblocked, None))
            })
        })?;
        let (done, unblocked, window) = outcome;
        if let Some(at) = window {
            crate::tasks::notify_veto_window(vault_path.to_path_buf(), done.clone(), clock(at))
                .await;
        } else {
            crate::tasks::notify_unblocked(vault_path, &done, &unblocked, route_tags);
        }
        return Ok(Applied::Moved {
            status: done.status.clone(),
            updated_at: done.updated_at.clone(),
            detail: if window.is_some() {
                "inside its veto window".into()
            } else {
                "completed".into()
            },
        });
    }

    // Failed, nothing to check: back to the worker with the findings.
    let (task_id, reasons, sweep_verifier) =
        (task.id.clone(), verdict.reasons.clone(), verifier.clone());
    let result = env.db.with_conn(move |c| {
        t::in_write_tx(c, |c| {
            let moved = t::send_back(c, &task_id, &sweep_verifier, &reasons, max_attempts)?;
            let task = t::get_task(c, &task_id)?.ok_or_else(|| anyhow::anyhow!("task vanished"))?;
            Ok((moved, task))
        })
    })?;
    let (moved, task) = result;
    Ok(Applied::Moved {
        status: task.status.clone(),
        updated_at: task.updated_at.clone(),
        detail: match moved {
            t::Reworked::Returned { .. } => "returned to to_do with the findings".into(),
            t::Reworked::Escalated { .. } => "blocked on you after repeated returns".into(),
        },
    })
}

// ---- undo ----

/// What undoing a sweep did.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UndoReport {
    pub restored: usize,
    /// Tasks left alone because something changed them after the sweep, with why.
    pub left_alone: Vec<(String, String)>,
}

/// Restores every task the sweep moved that has not changed since, and reports the ones it left
/// alone. A restored task is back in review and is not verified automatically until it is
/// submitted again, so the undo is not simply redone.
pub fn undo_sweep(conn: &rusqlite::Connection, sweep_id: &str) -> Result<UndoReport> {
    t::in_write_tx(conn, |conn| {
        let sweep = t::get_sweep(conn, sweep_id)?
            .ok_or_else(|| anyhow::anyhow!("no sweep '{sweep_id}'"))?;
        if sweep.status != t::SWEEP_APPLIED {
            anyhow::bail!(
                "sweep {sweep_id} is {}: only an applied sweep can be undone",
                sweep.status
            );
        }
        let mut report = UndoReport::default();
        for item in t::sweep_items(conn, sweep_id)? {
            if item.state != t::ITEM_APPLIED {
                continue;
            }
            let Some(task) = t::get_task(conn, &item.task_id)? else {
                continue;
            };
            let applied = item.applied_status.clone().unwrap_or_default();
            let unchanged = task.status == applied
                && Some(task.updated_at.as_str()) == item.applied_updated_at.as_deref();
            if !unchanged {
                let why = format!("it is {} now and was changed after the sweep", task.status);
                t::mark_item(conn, sweep_id, &task.id, t::ITEM_LEFT_ALONE, &why, None)?;
                report.left_alone.push((task.display_id.clone(), why));
                continue;
            }
            if task.status != t::STATUS_READY_FOR_REVIEW {
                let ctx = t::WriteCtx {
                    actor: Some("owner:sweep-undo"),
                    owner: true,
                    ..Default::default()
                };
                t::update_task_as(
                    conn,
                    &task.id,
                    &t::TaskPatch {
                        status: Some(t::STATUS_READY_FOR_REVIEW.to_string()),
                        ..Default::default()
                    },
                    Some(&applied),
                    &ctx,
                )?;
            } else {
                // A veto window the sweep opened.
                t::clear_hold(conn, &task.id)?;
            }
            t::skip_verification(conn, &task.id, &format!("sweep {sweep_id} was undone"))?;
            t::add_comment(
                conn,
                &task.id,
                "audit",
                &format!(
                    "Sweep {sweep_id} was undone by the owner. This task is back in review and is not \
                     verified automatically until it is submitted again."
                ),
                None,
            )?;
            t::mark_item(conn, sweep_id, &task.id, t::ITEM_UNDONE, "restored", None)?;
            report.restored += 1;
        }
        t::set_sweep_status(conn, sweep_id, &[t::SWEEP_APPLIED], t::SWEEP_UNDONE)?;
        Ok(report)
    })
}

// ---- what the owner can ask for (shared by `hq verify sweep` and the web API) ----

/// Starts a sweep over `scope`. The daemon plans it within a minute. Refused while verification
/// is off, since a sweep that cannot be run is only a list.
pub fn start_sweep(
    conn: &rusqlite::Connection,
    cfg: &hq_core::config::VerificationConfig,
    scope: &t::SweepScope,
) -> Result<t::Sweep> {
    if cfg.mode == hq_core::config::VerificationMode::Off {
        anyhow::bail!(
            "task verification is off: set tasks.verification.mode to shadow (record only) or \
             enforce before sweeping"
        );
    }
    t::plan_sweep_items(conn, scope, cfg)
}

/// One control on a sweep: `run`, `resume`, `pause`, `cancel` or `apply` (with `confirmed` once it
/// moves more than `sweep_confirm_over` tasks). Undo is [`undo_sweep`].
pub fn sweep_action(
    conn: &rusqlite::Connection,
    cfg: &hq_core::config::VerificationConfig,
    id: &str,
    action: &str,
    confirmed: bool,
) -> Result<t::Sweep> {
    match action {
        "run" | "resume" => t::set_sweep_status(
            conn,
            id,
            &[t::SWEEP_PLANNED, t::SWEEP_PAUSED],
            t::SWEEP_RUNNING,
        ),
        "pause" => t::set_sweep_status(conn, id, &[t::SWEEP_RUNNING], t::SWEEP_PAUSED),
        "cancel" => t::set_sweep_status(
            conn,
            id,
            &[
                t::SWEEP_PLANNING,
                t::SWEEP_PLANNED,
                t::SWEEP_RUNNING,
                t::SWEEP_PAUSED,
                t::SWEEP_READY,
            ],
            t::SWEEP_CANCELLED,
        ),
        "apply" => t::request_apply(conn, cfg, id, confirmed),
        other => anyhow::bail!(
            "a sweep can be run, resumed, paused, cancelled or applied, not '{other}'"
        ),
    }
}
