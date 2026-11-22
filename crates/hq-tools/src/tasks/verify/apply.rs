//! Turning a resolution into a recorded verdict: plan it, decide what to do with it, and write it
//! in one transaction. `pipeline::finish_as` runs these in order and sends the notifications.

use anyhow::Result;
use hq_core::config::VerificationMode;
use hq_db::tasks as t;
use serde_json::{Value, json};

use super::pipeline::{
    Disposition, Env, Evidence, Finished, Moved, Resolution, WaitingFor, autonomy_hold, clock,
    completion_note,
};

/// A resolution that could not decide yet, and what to record in its place.
pub(super) struct Deferral {
    pub(super) reason: String,
    pub(super) waiting_for: WaitingFor,
    pub(super) retry_after_secs: u64,
}

/// The verdict a resolution comes to, before anything decides what to do with it.
pub(super) struct Planned {
    pub(super) verdict: &'static str,
    pub(super) level: &'static str,
    pub(super) independence: &'static str,
    pub(super) criteria: Value,
    pub(super) reasons: String,
    pub(super) waiting_for: Option<WaitingFor>,
}

/// Turns a resolution into the verdict to record. A resolution that could not decide yet records a
/// wait instead, so it comes back as the deferral.
pub(super) fn plan_verdict(resolution: Resolution, ev: &Evidence) -> Result<Planned, Deferral> {
    Ok(match resolution {
        Resolution::Defer {
            reason,
            waiting_for,
            retry_after_secs,
        } => {
            return Err(Deferral {
                reason,
                waiting_for,
                retry_after_secs,
            });
        }
        Resolution::Pass {
            level,
            independence,
            criteria,
            summary,
        } => {
            let assessed = ev.contract.is_assessed_only();
            let reasons = if assessed {
                format!("assessed, not checked: the task states no acceptance criteria. {summary}")
            } else {
                summary
            };
            Planned {
                verdict: t::VERDICT_PASS,
                level,
                independence,
                criteria,
                reasons,
                waiting_for: None,
            }
        }
        Resolution::Fail { reasons, criteria } => Planned {
            verdict: t::VERDICT_FAIL,
            level: t::LEVEL_NONE,
            independence: "none",
            criteria,
            reasons,
            waiting_for: Some(WaitingFor::Worker),
        },
        Resolution::Inconclusive {
            reasons,
            waiting_for,
        } => Planned {
            verdict: t::VERDICT_INCONCLUSIVE,
            level: t::LEVEL_NONE,
            independence: "none",
            criteria: json!([]),
            reasons,
            waiting_for: Some(waiting_for),
        },
    })
}

/// Stores the wait for a deferral, unless the task was submitted again meanwhile.
pub(super) async fn record_deferral(
    env: &Env<'_>,
    ev: &Evidence,
    wait: Deferral,
) -> Result<Finished> {
    let marker = ev.marker;
    let id = ev.task.id.clone();
    let Deferral {
        reason,
        waiting_for,
        retry_after_secs,
    } = wait;
    let stored = reason.clone();
    let recorded = env.db.with_conn(move |c| {
        t::in_write_tx(c, |c| {
            if t::review_marker(c, &id)? != marker {
                return Ok(false);
            }
            t::record_wait(
                c,
                &id,
                waiting_for.as_str(),
                &stored,
                retry_after_secs,
                chrono::Utc::now().timestamp(),
            )?;
            Ok(true)
        })
    })?;
    if !recorded {
        return Ok(Finished::Voided {
            reason: "the task was submitted again while it was being verified".to_string(),
        });
    }
    Ok(Finished::Deferred {
        reason,
        waiting_for,
        retry_after_secs,
    })
}

/// What to do with a planned verdict: whether it completes the task, is held for the owner, waits
/// out a veto window, or sends the task back.
pub(super) struct Decision {
    pub(super) planned: Planned,
    /// Why measured trust in the verifier does not allow completing now, if it does not.
    pub(super) held: Option<String>,
    /// The verdict is a pass that completes the task (or starts its veto window).
    pub(super) applies: bool,
    /// When a tier 2 task completes unless vetoed, in unix seconds.
    pub(super) release_at: Option<i64>,
    pub(super) sends_back: bool,
}

pub(super) fn decide(
    env: &Env<'_>,
    ev: &Evidence,
    disposition: Disposition,
    planned: Planned,
) -> Decision {
    let enforced = env.cfg.mode == VerificationMode::Enforce && env.cfg.enforces(ev.tier);
    // A pass completes the task only while measured trust in the verifier allows it; otherwise
    // the verdict is kept and the task is left for the owner, who is told why once.
    let held = if planned.verdict == t::VERDICT_PASS && enforced {
        autonomy_hold(env, ev.tier)
    } else {
        None
    };
    let Planned {
        verdict,
        level,
        independence,
        criteria,
        reasons,
        waiting_for,
    } = planned;
    let (reasons, waiting_for) = match &held {
        Some(why) => (
            format!("{reasons}\nHeld for the owner: {why}."),
            Some(WaitingFor::Owner),
        ),
        None => (reasons, waiting_for),
    };
    let applies = verdict == t::VERDICT_PASS && enforced && held.is_none();
    // Tier 2 work is approved now and completed when its veto window closes.
    let hold_for = (applies
        && disposition == Disposition::Act
        && ev.tier >= t::MAX_TIER
        && env.cfg.veto_hours > 0)
        .then(|| i64::from(env.cfg.veto_hours) * 3600);
    let release_at = hold_for.map(|secs| chrono::Utc::now().timestamp().saturating_add(secs));
    let (reasons, waiting_for) = match release_at {
        Some(at) => (
            format!(
                "{reasons}\nTier 2: it completes at {} UTC unless the owner vetoes it first.",
                clock(at)
            ),
            Some(WaitingFor::Owner),
        ),
        None => (reasons, waiting_for),
    };
    let sends_back =
        enforced && disposition == Disposition::Act && waiting_for == Some(WaitingFor::Worker);
    Decision {
        planned: Planned {
            verdict,
            level,
            independence,
            criteria,
            reasons,
            waiting_for,
        },
        held,
        applies,
        release_at,
        sends_back,
    }
}

/// Everything the transaction that records and applies a verdict needs, owned so it can move into
/// the database closure.
pub(super) struct ApplyArgs {
    pub(super) task_id: String,
    pub(super) marker: Option<i64>,
    pub(super) artifact: String,
    pub(super) contract_hash: String,
    pub(super) tier: u8,
    pub(super) verifier: String,
    pub(super) observations: Value,
    pub(super) verifier_config: String,
    pub(super) currency_snapshot: Value,
    pub(super) max_attempts: u32,
    pub(super) disposition: Disposition,
    pub(super) planned: Planned,
    pub(super) applies: bool,
    pub(super) release_at: Option<i64>,
    pub(super) sends_back: bool,
}

/// What the transaction did: the recorded verdict, how it moved the task, and the task (with what
/// it unblocked) when someone is to be told.
pub(super) type Applied = (t::Verdict, Moved, Option<(t::Task, Vec<t::Task>)>);

/// Records the verdict and applies it in one transaction. `Err(reason)` inside means the work was
/// submitted or changed again, so nothing was written.
pub(super) fn apply_verdict(
    c: &rusqlite::Connection,
    a: &ApplyArgs,
) -> Result<std::result::Result<Applied, String>> {
    let ApplyArgs {
        task_id,
        marker,
        artifact,
        contract_hash,
        tier,
        verifier,
        observations,
        verifier_config,
        currency_snapshot,
        max_attempts,
        disposition,
        planned,
        applies,
        release_at,
        sends_back,
    } = a;
    let (applies, sends_back, release_at) = (*applies, *sends_back, *release_at);
    let Planned {
        verdict,
        level,
        independence,
        criteria,
        reasons,
        waiting_for,
    } = planned;
    t::in_write_tx(c, |c| {
        // The work must still be the submission that was checked: a verdict recorded about
        // an earlier one would hide the new submission from the queue.
        if t::review_marker(c, task_id)? != *marker {
            return Ok(Err(
                "the task was submitted again while it was being verified".to_string(),
            ));
        }
        if applies || sends_back {
            let current = t::get_task(c, task_id)?
                .ok_or_else(|| anyhow::anyhow!("task {task_id} vanished"))?;
            if current.status != t::STATUS_READY_FOR_REVIEW {
                return Ok(Err(
                    "the task was changed while it was being verified".to_string()
                ));
            }
        }
        let new = t::NewVerdict {
            task_id,
            artifact,
            contract_hash,
            tier: *tier,
            verdict,
            level,
            verifier,
            independence,
            observations,
            criteria,
            reasons,
            applied: applies,
            canary: false,
            submission: *marker,
            waiting_for: waiting_for.map(WaitingFor::as_str),
            verifier_config,
        };
        let recorded = t::record_verdict(c, &new)?;
        t::clear_wait(c, task_id)?;
        if *disposition == Disposition::Record {
            return Ok(Ok((recorded, Moved::Nothing, None)));
        }
        if let Some(at) = release_at {
            t::create_hold(c, task_id, &recorded.id, at, currency_snapshot)?;
            t::add_comment(
                c,
                task_id,
                "verifier",
                &format!(
                    "Verified by HQ ({} evidence, tier {}, artifact {}). Because this is tier 2 it \
                     completes at {} UTC unless you veto it first: `hq verify veto {}`.",
                    recorded.level,
                    recorded.tier,
                    recorded.artifact,
                    clock(at),
                    t::get_task(c, task_id)?
                        .map(|t| t.display_id)
                        .unwrap_or_default()
                ),
                None,
            )?;
            let task = t::get_task(c, task_id)?;
            return Ok(Ok((recorded, Moved::Held, task.map(|t| (t, Vec::new())))));
        }
        if applies {
            let ctx = t::WriteCtx {
                actor: Some(verifier),
                verdict_id: Some(&recorded.id),
                ..Default::default()
            };
            let patch = t::TaskPatch {
                status: Some(t::STATUS_COMPLETE.to_string()),
                ..Default::default()
            };
            let done =
                t::update_task_as(c, task_id, &patch, Some(t::STATUS_READY_FOR_REVIEW), &ctx)?;
            t::add_comment(c, task_id, "verifier", &completion_note(&recorded), None)?;
            let unblocked =
                crate::tasks::unblocked_by_transition(c, Some(t::STATUS_READY_FOR_REVIEW), &done)?;
            return Ok(Ok((recorded, Moved::Completed, Some((done, unblocked)))));
        }
        if sends_back {
            let reworked = t::send_back(c, task_id, verifier, reasons, *max_attempts)?;
            let moved = match reworked {
                t::Reworked::Returned { .. } => Moved::Returned,
                t::Reworked::Escalated { .. } => Moved::Escalated,
            };
            // A task that now waits on the owner is the one thing the owner is told about.
            let task = (moved == Moved::Escalated)
                .then(|| t::get_task(c, task_id))
                .transpose()?
                .flatten();
            return Ok(Ok((recorded, moved, task.map(|t| (t, Vec::new())))));
        }
        Ok(Ok((recorded, Moved::Nothing, None)))
    })
}
