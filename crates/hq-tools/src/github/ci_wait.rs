//! CI waits without a turn (`docs/plans/edge-worker-ci-loop.md`). After `github_write` pushes,
//! the worker calls `edge_ci_wait` and ends its turn. On each `edge-worker-dispatch` tick the
//! daemon reads the checks on the pushed commit, with no model involved: it hands the task over
//! when CI passes, and resumes the worker's own chat thread when CI fails.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use hq_core::config::{HqConfig, TasksConfig};
use hq_db::Database;
use hq_db::ask_requests as asks;
use hq_db::edge_ci_waits as waits;
use hq_db::tasks as t;
use serde_json::{Value, json};

use super::dispatch::{self, ASK_SCOPE, CALLER, DISPATCH_TAG, WORKER_SKILL};
use super::write::{BRANCH_PREFIX, check_branch, newest_pushed};
use super::{records_root, repo, text};
use crate::registry::HqTool;
use crate::tasks::verify::GitHubRead;
use crate::tasks::verify::collect::collect_checks;
use crate::tasks::verify::observation::ObservationLog;

/// Rounds of fixes before a task still failing CI is parked. Matches the skill's "four failed
/// fixes in all".
pub const MAX_ROUNDS: i64 = 4;
/// How long a commit may show no checks at all before HQ takes it that the repository runs none.
const NO_CI_SECS: i64 = 20 * 60;
/// How long CI may run, or go unread, before the task is parked.
const MAX_WAIT_SECS: i64 = 6 * 60 * 60;
/// Check names passed into a resumed turn: the repository's CI files name them, so they are
/// capped and quoted, never taken as instructions.
const NAMES_SHOWN: usize = 5;
const NAME_CHARS: usize = 80;

pub struct EdgeCiWaitTool {
    db: Arc<Database>,
    settings: TasksConfig,
    /// `None`: read `github_write.repos` from the config at call time.
    repos: Option<Vec<String>>,
    records: PathBuf,
}

impl EdgeCiWaitTool {
    pub fn new(db: Arc<Database>, settings: TasksConfig) -> Self {
        Self {
            db,
            settings,
            repos: None,
            records: records_root(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        db: Arc<Database>,
        settings: TasksConfig,
        repos: Vec<String>,
        records: PathBuf,
    ) -> Self {
        Self {
            db,
            settings,
            repos: Some(repos),
            records,
        }
    }

    fn allowed_repos(&self) -> Vec<String> {
        match &self.repos {
            Some(repos) => repos.clone(),
            None => HqConfig::load()
                .map(|c| c.github_write.repos)
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl HqTool for EdgeCiWaitTool {
    fn name(&self) -> &str {
        "edge_ci_wait"
    }

    fn description(&self) -> &str {
        "For the edge coding worker, right after github_write pushed: hand CI to HQ and end your \
         turn instead of polling. Records the commit github_write pushed to the branch and \
         releases your lease. When CI passes HQ hands the task over for review; when it fails HQ \
         brings you back in this same chat with the failing checks."
    }

    fn category(&self) -> &str {
        "tasks"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "lease": {"type": "string", "description": "The lease token task_claim returned"},
                "repo": {"type": "string", "description": "owner/name, as given to github_write"},
                "branch": {"type": "string", "description": "The branch given to github_write (hq/ optional)"},
                "pull_request": {"type": "string", "description": "The pull request link github_write returned"}
            },
            "required": ["lease", "repo", "branch"]
        })
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        if dispatch::restricted_caller(&args) {
            bail!("this connection cannot hand CI over for the edge worker");
        }
        let token = text(&args, "lease")
            .context("`lease` is required")?
            .to_string();
        let repo = repo(&args)?;
        if !self
            .allowed_repos()
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&repo))
        {
            bail!("{repo} is not in github_write.repos");
        }
        let name = text(&args, "branch").context("`branch` is required")?;
        let name = name.strip_prefix(BRANCH_PREFIX).unwrap_or(name);
        check_branch(name)?;
        let branch = format!("{BRANCH_PREFIX}{name}");
        let sha = newest_pushed(&self.records.join(&repo), &branch).with_context(|| {
            format!("github_write has pushed nothing to {branch} in {repo}; push first")
        })?;
        // Only a link to this repository's pull requests is kept; anything else is dropped.
        let pull_request = text(&args, "pull_request")
            .and_then(|url| pr_number(url, &repo).map(|_| url.to_string()));
        let ttl = i64::try_from(self.settings.lease_ttl()).unwrap_or(i64::MAX);
        let (task, wait) = {
            let (repo, branch, sha) = (repo.clone(), branch.clone(), sha.clone());
            self.db.with_conn(move |c| {
                t::in_write_tx(c, |c| {
                    let lease = t::require_live_lease(c, &token, ttl)?;
                    let task = t::get_task(c, &lease.task_id)?
                        .ok_or_else(|| anyhow::anyhow!("the lease's task no longer exists"))?;
                    if !task.tags.iter().any(|g| g.eq_ignore_ascii_case(DISPATCH_TAG)) {
                        bail!(
                            "{} is not an {DISPATCH_TAG} task; release it with task_release instead",
                            task.display_id
                        );
                    }
                    if task.status != t::STATUS_IN_PROGRESS {
                        bail!(
                            "{} is {}, not in progress; nothing is waiting on CI",
                            task.display_id,
                            task.status
                        );
                    }
                    let started = asks::latest_by_external_prefix(
                        c,
                        ASK_SCOPE,
                        &format!("{CALLER}:{}:", task.id),
                    )?
                    .with_context(|| {
                        format!(
                            "{} was not started by HQ's edge worker, so there is no chat to resume; release it with task_release instead",
                            task.display_id
                        )
                    })?;
                    let wait = waits::open(
                        c,
                        &waits::NewWait {
                            task_id: &task.id,
                            repo: &repo,
                            branch: &branch,
                            sha: &sha,
                            pull_request: pull_request.as_deref(),
                            thread_id: &started.thread_id,
                        },
                    )?;
                    t::release(c, &token, None, "", ttl)?;
                    t::add_comment(
                        c,
                        &task.id,
                        CALLER,
                        &format!(
                            "Waiting on CI for {} on {branch} (round {} of {MAX_ROUNDS}). HQ hands the task over when CI passes and brings the worker back if it fails.",
                            short(&sha),
                            wait.round
                        ),
                        None,
                    )?;
                    Ok((task, wait))
                })
            })?
        };
        Ok(json!({
            "waiting": true,
            "task": task.display_id,
            "commit": wait.sha,
            "round": wait.round,
            "note": "CI is HQ's to watch now: end your turn. If CI fails, HQ continues this chat with the failing checks.",
        }))
    }
}

/// The number in `https://github.com/<repo>/pull/<n>`, for `repo` only.
fn pr_number(url: &str, repo: &str) -> Option<u64> {
    let rest = url.strip_prefix("https://github.com/")?;
    let (r, number) = rest.trim_end_matches('/').rsplit_once("/pull/")?;
    if !r.eq_ignore_ascii_case(repo) {
        return None;
    }
    number.parse().ok().filter(|n| *n > 0)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

/// One pass over the open waits. Does nothing while a worker reply runs: that reply may still
/// be pushing, and a resumed turn must not start beside it.
pub(super) async fn resolve(
    db: &Arc<Database>,
    config: &HqConfig,
    runner: &crate::ask::RunnerCell,
    gh: &dyn GitHubRead,
) -> Result<()> {
    let prefix = format!("{CALLER}:");
    let open = db.with_conn(move |c| {
        if asks::pending_by_external_prefix(c, ASK_SCOPE, &prefix)?.is_some() {
            return Ok(Vec::new());
        }
        waits::open_waits(c)
    })?;
    for wait in open {
        if resolve_one(db, config, runner, gh, &wait).await? {
            // A reply is running again; the rest waits for the next tick.
            break;
        }
    }
    Ok(())
}

/// Decides one wait. True when it started a reply.
async fn resolve_one(
    db: &Arc<Database>,
    config: &HqConfig,
    runner: &crate::ask::RunnerCell,
    gh: &dyn GitHubRead,
    wait: &waits::CiWait,
) -> Result<bool> {
    let task_id = wait.task_id.clone();
    let task = db.with_conn(move |c| t::get_task(c, &task_id))?;
    let Some(task) = task.filter(|task| {
        task.status == t::STATUS_IN_PROGRESS
            && task
                .tags
                .iter()
                .any(|g| g.eq_ignore_ascii_case(DISPATCH_TAG))
    }) else {
        close(db, wait, waits::STATE_CLOSED, "the task moved on")?;
        return Ok(false);
    };

    let mut log = ObservationLog::new();
    let checks = collect_checks(gh, &mut log, &wait.repo, &wait.sha, None).await;
    let read = !log.of_kind("check_runs").any(|o| o.unavailable);
    {
        let id = wait.id;
        db.with_conn(move |c| waits::touch(c, id, read))?;
    }
    let sha = short(&wait.sha);

    if !read {
        if wait.unread_secs > MAX_WAIT_SECS {
            let reason = format!("HQ could not read CI on {sha} for six hours");
            park(db, wait, &task, &reason)?;
        }
        return Ok(false);
    }
    if checks.failed {
        let failing = failing_checks(&log);
        if wait.round >= MAX_ROUNDS {
            let reason = format!(
                "CI still fails on {sha} after {MAX_ROUNDS} rounds ({}){}",
                failing.join(", "),
                pr_suffix(wait)
            );
            park(db, wait, &task, &reason)?;
            return Ok(false);
        }
        // Closed first, so a crash after the ask starts can never start it twice.
        if !close(db, wait, waits::STATE_FAILED, &failing.join(", "))? {
            return Ok(false);
        }
        return match resume(db, config, runner, wait, &task, &failing).await {
            Ok(_) => Ok(true),
            Err(e) => {
                let reason = format!("HQ could not bring the worker back after CI failed: {e}");
                dispatch::park(db, &task, &reason)?;
                Ok(false)
            }
        };
    }
    if checks.green {
        let note = format!(
            "CI passed: all {} checks succeeded on {sha}.",
            checks.reported
        );
        hand_over(db, wait, &task, &note)?;
    } else if checks.reported == 0 && wait.age_secs > NO_CI_SECS {
        let note = format!(
            "No CI ran on {sha} in twenty minutes, so nothing tested this change. Review it with that in mind."
        );
        hand_over(db, wait, &task, &note)?;
    } else if wait.age_secs > MAX_WAIT_SECS {
        let reason = format!("CI on {sha} did not finish in six hours{}", pr_suffix(wait));
        park(db, wait, &task, &reason)?;
    }
    Ok(false)
}

fn close(db: &Database, wait: &waits::CiWait, state: &str, detail: &str) -> Result<bool> {
    let (id, state, detail) = (wait.id, state.to_string(), detail.to_string());
    db.with_conn(move |c| waits::resolve(c, id, &state, &detail))
}

fn park(db: &Database, wait: &waits::CiWait, task: &t::Task, reason: &str) -> Result<()> {
    if close(db, wait, waits::STATE_PARKED, reason)? {
        dispatch::park(db, task, reason)?;
    }
    Ok(())
}

fn pr_suffix(wait: &waits::CiWait) -> String {
    wait.pull_request
        .as_deref()
        .map(|url| format!("; pull request {url}"))
        .unwrap_or_default()
}

/// The failing checks' names, as `collect_checks` recorded them, capped and cleaned.
fn failing_checks(log: &ObservationLog) -> Vec<String> {
    log.of_kind("check_runs")
        .filter_map(|o| o.data.get("failed").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .take(NAMES_SHOWN)
        .map(|name| {
            name.chars()
                .filter(|c| !c.is_control())
                .take(NAME_CHARS)
                .collect::<String>()
        })
        .map(|name| serde_json::to_string(&name).unwrap_or_default())
        .collect()
}

/// CI passed (or never ran): the task goes to review with its pull request linked, which is
/// where task verification looks for it.
fn hand_over(db: &Database, wait: &waits::CiWait, task: &t::Task, note: &str) -> Result<()> {
    let (wait, task, note) = (wait.clone(), task.clone(), note.to_string());
    db.with_conn(move |c| {
        t::in_write_tx(c, |c| {
            if !waits::resolve(c, wait.id, waits::STATE_PASSED, &note)? {
                return Ok(());
            }
            let ctx = t::WriteCtx {
                actor: Some(CALLER),
                ..Default::default()
            };
            let patch = t::TaskPatch {
                status: Some(t::STATUS_READY_FOR_REVIEW.into()),
                ..Default::default()
            };
            t::update_task_as(c, &task.id, &patch, Some(t::STATUS_IN_PROGRESS), &ctx)?;
            if let Some(n) = wait
                .pull_request
                .as_deref()
                .and_then(|url| pr_number(url, &wait.repo))
            {
                let reference = format!("{}#{n}", wait.repo);
                t::add_task_link(
                    c,
                    &task.id,
                    "pr",
                    &reference,
                    "pull request",
                    Some("produced"),
                    CALLER,
                )?;
            }
            t::add_comment(
                c,
                &task.id,
                CALLER,
                &format!("{note} Handed over for review{}.", pr_suffix(&wait)),
                None,
            )?;
            Ok(())
        })
    })
}

/// Continues the worker's own thread with what failed. The text is HQ's, apart from the quoted
/// check names.
async fn resume(
    db: &Arc<Database>,
    config: &HqConfig,
    runner: &crate::ask::RunnerCell,
    wait: &waits::CiWait,
    task: &t::Task,
    failing: &[String],
) -> Result<String> {
    let failing = match failing.is_empty() {
        true => "(names not reported)".to_string(),
        false => failing.join(", "),
    };
    let question = format!(
        "CI failed on {} of {} for HQ task {} (round {} of {MAX_ROUNDS}). Failing checks: {failing}. \
         Load the {WORKER_SKILL} skill with load_skill and continue from its step 6: claim the task \
         again, read the failing job's log, fix through a sub-agent, push to the same branch with \
         github_write, then call edge_ci_wait and end your turn.",
        short(&wait.sha),
        wait.branch,
        task.display_id,
        wait.round,
    );
    dispatch::ask(
        db.clone(),
        runner.clone(),
        dispatch::Ask {
            question,
            title: format!("Edge worker: {}", task.display_id),
            key: format!("{CALLER}:{}:ci-{}", task.id, wait.sha),
            thread: Some(wait.thread_id.clone()),
            model: config.github_write.dispatch_model.clone(),
        },
    )
    .await
}

#[cfg(test)]
#[path = "ci_wait_tests.rs"]
mod tests;
