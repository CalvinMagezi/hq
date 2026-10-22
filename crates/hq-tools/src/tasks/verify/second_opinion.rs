//! Second opinions (docs/plans/task-verification.md, "Verify and audit are different jobs").
//!
//! The owner asks for a fresh verifier to judge one task again. It looks at the same evidence
//! as the first (read again, so a change since counts), and asks a reviewer other than the one that
//! made the first verdict when another is configured. A second opinion that disagrees counts as an
//! audit datum against the first verdict and puts the task in front of the owner. It never changes
//! the task by itself.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::tasks as t;

use super::pipeline::{Env, Gate, Resolution, gate, gather, review};
use super::reviewer::{ReviewReply, ReviewRequest, Reviewer};

/// A reviewer that leaves out the models that made the first verdict, when others exist.
struct Excluding<'a> {
    inner: &'a dyn Reviewer,
    exclude: Vec<String>,
}

#[async_trait]
impl Reviewer for Excluding<'_> {
    fn models(&self) -> Vec<String> {
        let all = self.inner.models();
        let others: Vec<String> = all
            .iter()
            .filter(|m| !self.exclude.iter().any(|e| e == *m))
            .cloned()
            .collect();
        if others.is_empty() { all } else { others }
    }
    async fn review(&self, request: &ReviewRequest) -> Result<ReviewReply> {
        self.inner.review(request).await
    }
}

/// How a resolution compares with a verdict: a pass agrees with a pass, and anything else with
/// anything else.
fn agrees(first: &str, second: &Resolution) -> Option<bool> {
    match second {
        Resolution::Pass { .. } => Some(first == t::VERDICT_PASS),
        Resolution::Fail { .. } | Resolution::Inconclusive { .. } => Some(first != t::VERDICT_PASS),
        // Not decided: no second opinion yet.
        Resolution::Defer { .. } => None,
    }
}

/// Answers up to `limit` pending requests. Returns how many it answered.
pub async fn answer_second_opinions(
    env: &Env<'_>,
    reviewer: &dyn Reviewer,
    limit: usize,
) -> Result<usize> {
    let pending = env.db.with_conn(|c| t::pending_reverifies(c, limit))?;
    let mut answered = 0;
    for request in pending {
        let id = request.task_id.clone();
        let loaded = env
            .db
            .with_conn(move |c| Ok((t::get_task(c, &id)?, t::latest_verdict(c, &id)?)))?;
        let (Some(task), Some(first)) = loaded else {
            let id = request.task_id.clone();
            env.db.with_conn(move |c| {
                t::answer_reverify(
                    c,
                    &id,
                    t::REVERIFY_FAILED,
                    "the task or its verdict is gone",
                )
            })?;
            continue;
        };
        let mut ev = gather(env, &task).await?;
        let resolution = match gate(&ev, env.cfg) {
            Gate::Settled(r) => r,
            Gate::NeedsReview { level } => {
                let used: Vec<String> = first
                    .verifier_config
                    .split('+')
                    .map(|m| m.to_string())
                    .collect();
                let fresh = Excluding {
                    inner: reviewer,
                    exclude: used,
                };
                review(env, &mut ev, level, &fresh).await
            }
        };
        let Some(agreed) = agrees(&first.verdict, &resolution) else {
            // Cannot be decided now (a check still running, no reviewer reachable): try later.
            continue;
        };
        let why = match &resolution {
            Resolution::Pass { summary, .. } => summary.clone(),
            Resolution::Fail { reasons, .. } => reasons.clone(),
            Resolution::Inconclusive { reasons, .. } => reasons.clone(),
            Resolution::Defer { reason, .. } => reason.clone(),
        };
        let line: String = why
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect();
        let (task_id, verdict_id, display, vault_path) = (
            task.id.clone(),
            first.id.clone(),
            task.display_id.clone(),
            env.vault_path,
        );
        let comment = if agreed {
            format!(
                "A second opinion agreed with the first verdict ({}).",
                first.verdict
            )
        } else {
            format!(
                "A second opinion disagreed with the first verdict ({}):\n> {line}",
                first.verdict
            )
        };
        let task_for_notice = task.clone();
        env.db.with_conn(move |c| {
            t::in_write_tx(c, |c| {
                if !agreed {
                    t::record_second_opinion_disagreement(c, &verdict_id, &line)?;
                }
                t::add_comment(c, &task_id, "audit", &comment, None)?;
                t::answer_reverify(
                    c,
                    &task_id,
                    if agreed {
                        t::REVERIFY_AGREED
                    } else {
                        t::REVERIFY_DISAGREED
                    },
                    &line,
                )
            })
        })?;
        if !agreed {
            crate::tasks::notify_owner_exception(
                vault_path.to_path_buf(),
                task_for_notice,
                format!("{display}: a second opinion disagreed with the first verdict. {why}"),
            )
            .await;
        }
        answered += 1;
    }
    Ok(answered)
}
