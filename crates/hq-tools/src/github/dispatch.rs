//! `edge-worker-dispatch`: with `github_write.dispatch_tasks` on, the daemon picks up open tasks
//! tagged `edge-task` that name a repository in `github_write.repos`, and has HQ work each one
//! with the `edge-coding-worker` skill, one at a time.

use std::sync::Arc;

use anyhow::{Context, Result};
use hq_core::config::HqConfig;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::ask_requests as asks;
use hq_db::coder_jobs;
use hq_db::tasks as t;
use serde_json::json;

use super::app::{AppClient, Standing, standing};
use crate::ask::AskJob;

/// Tasks carrying this tag are the worker's queue. Its own tag, so tasks that are merely about
/// the worker are never taken for work by it. Scoped connections may not set it
/// ([`refuse_queue_write`]).
pub const DISPATCH_TAG: &str = "edge-task";
/// Marks a queue task the worker dropped. The worker skips it until the owner removes the tag,
/// which no restricted caller can do, so no other key can make it run the same text again.
pub const PARKED_TAG: &str = "edge-task-parked";
/// The built-in skill the work follows; dispatch is off while it is not active.
const WORKER_SKILL: &str = "edge-coding-worker";
/// An assignee that marks a task as HQ's own, beside the unassigned ones.
const HQ_ASSIGNEE: &str = "hq";
const CALLER: &str = "edge-worker";
const STOPPED_WITHOUT_HANDOVER: &str = "HQ's worker stopped without handing the task over";
const RUN_CUT_OFF: &str = "The last run was cut off, so HQ will try again.";
/// Tries a job gets across restarts and provider errors, so a real fault still reaches the owner.
const MAX_ATTEMPTS: i64 = 4;
/// Added to the wait before each new try after a provider error or a lost network.
const RETRY_STEP_SECS: i64 = 300;
/// The ask scope and mode the worker's asks run under.
const ASK_SCOPE: &str = "full";
const ASK_MODE: &str = "full";
// ponytail: a bounded scan instead of a filtered query; enough for a personal queue.
const QUEUE_DEPTH: usize = 100;
/// Where the owner's inbox items about the worker come from.
const INBOX_SOURCE: &str = "edge-worker-dispatch";
/// The Inbox item that asks for the first task's yes; allowing it gives that yes.
pub const FIRST_TASK_KEY: &str = "edge-worker-dispatch-first-task";

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Dispatch is not switched on, or the worker skill is not active here.
    Off,
    /// The worker is already on a task; one runs at a time.
    Busy(String),
    /// Nothing to take.
    Idle,
    /// HQ was asked to work `task`; the reply runs in the chat thread of `ask_id`.
    Started { task: String, ask_id: String },
    /// `task` is next, but the owner has not yet agreed to the first run after connecting.
    AwaitingConfirmation(String),
}

/// One tick, against the App on this device. A tick already running covers this one.
pub async fn dispatch(
    db: Arc<Database>,
    config: &HqConfig,
    runner: crate::ask::RunnerCell,
) -> Result<Outcome> {
    let Ok(_only_one) = TICK.try_lock() else {
        return Ok(Outcome::Idle);
    };
    let app = AppClient::standard()?;
    report_finished(&db);
    dispatch_with(db, config, runner, &app).await
}

static TICK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static KICK: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Asks for a tick now instead of at the next two-minute one.
pub fn kick() {
    KICK.notify_one();
}

/// Runs a tick whenever [`kick`] is called, so a started or finished job does not wait.
pub fn spawn_kick_listener(db: Arc<Database>) {
    tokio::spawn(async move {
        loop {
            KICK.notified().await;
            let Ok(config) = HqConfig::load() else {
                continue;
            };
            let runner = crate::ask::shared_runner_cell();
            if let Err(e) = dispatch(db.clone(), &config, runner).await {
                tracing::warn!(error = %e, "edge-worker-dispatch: a kicked tick failed");
            }
        }
    });
}

/// The reply working a task right now, if any.
pub(super) fn running_thread(db: &Database, task_id: &str) -> Result<Option<String>> {
    let prefix = format!("{CALLER}:{task_id}:");
    db.with_conn(|c| super::coder::pending_thread(c, &prefix, ASK_SCOPE))
}

/// Tells the chat that started a job how it ended.
fn report_finished(db: &Database) {
    if let Err(e) = db.with_conn(report_finished_in) {
        tracing::warn!(error = %e, "edge-worker-dispatch: telling a chat how a job ended failed");
    }
}

fn report_finished_in(conn: &rusqlite::Connection) -> Result<()> {
    for job in coder_jobs::working_with_origin(conn)? {
        let Some(task) = t::get_task(conn, &job.task_id)? else {
            continue;
        };
        let words = match task.status.as_str() {
            t::STATUS_READY_FOR_REVIEW => format!("{} is ready for you to review.", task.title),
            t::STATUS_BLOCKED => format!(
                "{} stopped: {}.",
                task.title,
                task.blocked_reason.as_deref().unwrap_or("no reason given")
            ),
            _ => continue,
        };
        coder_jobs::finish(conn, &job.task_id)?;
        if let Some(thread) = &job.origin_thread {
            hq_db::chat::add_message(conn, thread, "assistant", &words)?;
        }
    }
    Ok(())
}

/// The repositories the worker may take tasks for. With an App: those the person picked and
/// that need no confirmation, narrowed by config. A connection that stopped working allows
/// none, rather than falling back to the config list. Without an App: the config list.
pub fn allowed_repos(config: &HqConfig, app: &AppClient) -> Vec<String> {
    let narrow = &config.github_write.repos;
    let Some(record) = app.store().load() else {
        return narrow.clone();
    };
    if standing(Some(&record)) != Standing::Connected {
        return Vec::new();
    }
    app.repositories()
        .into_iter()
        .filter(|r| r.picked())
        .map(|r| r.repo)
        .filter(|r| narrow.is_empty() || narrow.iter().any(|n| n.eq_ignore_ascii_case(r)))
        .collect()
}

/// Whether a queued task would be taken now.
pub fn armed(config: &HqConfig, app: &AppClient) -> bool {
    enabled(config) && !allowed_repos(config, app).is_empty()
}

/// Connecting is consent to the App, not yet to spending on queued work.
fn awaiting_first_confirmation(app: &AppClient) -> bool {
    app.store()
        .load()
        .is_some_and(|record| !record.first_task_confirmed)
}

/// One tick: start work on the next task when nothing else is running. Tasks the worker
/// dropped are parked as blocked, with a note in the owner's inbox, so none holds the queue.
/// `runner` is where the reply runs, the web server's chat ([`crate::ask::shared_runner_cell`]).
pub async fn dispatch_with(
    db: Arc<Database>,
    config: &HqConfig,
    runner: crate::ask::RunnerCell,
    app: &AppClient,
) -> Result<Outcome> {
    // Without a web server in this process there is no chat to run the work in.
    let can_ask = runner.get().is_some();
    if !enabled(config) || !can_ask {
        return Ok(Outcome::Off);
    }
    if let Some(model) = config.github_write.dispatch_model.as_deref()
        && !model_is_declared(config, model)
    {
        warn_misconfigured(&db, model);
        anyhow::bail!(
            "github_write.dispatch_model '{model}' names no enabled backend in backends:; nothing was dispatched"
        );
    }
    let ttl = i64::try_from(config.tasks.lease_ttl()).unwrap_or(i64::MAX);
    match db.with_conn(move |c| busy_with(c, ttl))? {
        Busy::On(what) => return Ok(Outcome::Busy(what)),
        Busy::Free { dropped, cut_off } => {
            for (task, why) in dropped {
                park(&db, &task, &why)?;
            }
            for (task, wait) in cut_off {
                db.with_conn(|c| requeue(c, &task, wait))?;
            }
        }
    }
    let repos = allowed_repos(config, app);
    let allowed = repos.clone();
    let candidates = db.with_conn(move |c| candidates(c, &allowed))?;
    if let Some(first) = candidates.first()
        && awaiting_first_confirmation(app)
    {
        ask_for_first_confirmation(&db, first, config.github_write.task_budget_usd);
        return Ok(Outcome::AwaitingConfirmation(first.display_id.clone()));
    }
    for task in candidates {
        if !job_is_due(&db, &task)? {
            continue;
        }
        let (job, model, attempt) = match open_job(&db, config, &task, &repos).await? {
            Ok(opened) => opened,
            Err(why) => {
                park(&db, &task, &why)?;
                continue;
            }
        };
        let ask_id = ask_to_work(db.clone(), runner, &task, model.as_deref(), job, attempt).await?;
        // After the start, so a start that failed does not use up a try.
        let task_id = task.id.clone();
        db.with_conn(move |c| coder_jobs::begin_attempt(c, &task_id))?;
        return Ok(Outcome::Started {
            task: task.display_id,
            ask_id,
        });
    }
    Ok(Outcome::Idle)
}

fn job_is_due(db: &Database, task: &t::Task) -> Result<bool> {
    let job = db.with_conn(|c| coder_jobs::get(c, &task.id))?;
    Ok(job.is_none_or(|j| j.is_due(chrono::Utc::now())))
}

/// Why this task's last run rules out another, if it does: a reply that simply ended without
/// claiming the task, or a job that has been cut off too often.
fn no_more_tries(
    conn: &rusqlite::Connection,
    task: &t::Task,
    job: &coder_jobs::Job,
) -> Result<Option<String>> {
    let version = format!("{CALLER}:{}:{}:", task.id, task.updated_at);
    let Some(ask) = asks::latest_by_external_prefix(conn, ASK_SCOPE, &version)? else {
        return Ok(None);
    };
    if !asks::was_cut_off(ask.error.as_deref()) {
        let reason = ask
            .error
            .unwrap_or_else(|| "it did not claim the task".into());
        return Ok(Some(format!(
            "HQ's worker stopped before starting: {reason}"
        )));
    }
    Ok((job.attempts >= MAX_ATTEMPTS).then(|| out_of_tries(job)))
}

fn out_of_tries(job: &coder_jobs::Job) -> String {
    format!(
        "HQ's worker was cut off {} times, by a restart or a provider error",
        job.attempts
    )
}

/// The job for this task, the model it runs on and the number of this attempt, or why it cannot
/// start.
async fn open_job(
    db: &Arc<Database>,
    config: &HqConfig,
    task: &t::Task,
    repos: &[String],
) -> Result<std::result::Result<(AskJob, Option<String>, i64), String>> {
    let job = db.with_conn(|c| {
        coder_jobs::ensure(
            c,
            &coder_jobs::NewJob {
                task_id: &task.id,
                repo: named_repo(task, repos).unwrap_or_default(),
                budget_usd: config.github_write.task_budget_usd,
                credit_cap: None,
                model: config.github_write.dispatch_model.as_deref(),
                origin_thread: None,
            },
        )
    })?;
    if let Some(why) = db.with_conn(|c| no_more_tries(c, task, &job))? {
        return Ok(Err(why));
    }
    if job.attempts >= MAX_ATTEMPTS {
        return Ok(Err(out_of_tries(&job)));
    }
    if let Some(why) = stop_words(&job) {
        return Ok(Err(why));
    }
    if job.spent_usd >= job.budget_usd {
        return Ok(Err(budget_words(&job)));
    }
    if job
        .model
        .as_deref()
        .is_some_and(|m| !model_is_declared(config, m))
    {
        return Ok(Err(format!(
            "its model '{}' is not an enabled backend",
            job.model.as_deref().unwrap_or_default()
        )));
    }
    let credit_ceiling = match credit_ceiling(db, &job).await? {
        Ok(ceiling) => ceiling,
        Err(why) => return Ok(Err(why)),
    };
    let attempt = job.attempts + 1;
    let ask_job = AskJob {
        task_id: job.task_id,
        budget_usd: job.budget_usd,
        spent_usd: job.spent_usd,
        credit_ceiling,
    };
    Ok(Ok((ask_job, job.model, attempt)))
}

/// A run that was cut off goes back in the queue, with its dead lease ended so nothing waits on it.
fn requeue(conn: &rusqlite::Connection, task: &t::Task, wait_secs: i64) -> Result<()> {
    t::end_live_leases(conn, &task.id)?;
    let patch = t::TaskPatch {
        status: Some(t::STATUS_TO_DO.into()),
        ..Default::default()
    };
    t::update_task(conn, &task.id, &patch, Some(&task.status))?;
    coder_jobs::requeue(conn, &task.id, wait_secs)?;
    t::add_comment(conn, &task.id, CALLER, RUN_CUT_OFF, None)?;
    Ok(())
}

/// A job with a credit cap needs a starting reading: without one its cap cannot be enforced.
async fn credit_ceiling(
    db: &Arc<Database>,
    job: &coder_jobs::Job,
) -> Result<std::result::Result<Option<f64>, String>> {
    if job.credit_cap.is_none() || job.credit_start.is_some() {
        return Ok(Ok(job.credit_ceiling()));
    }
    let reading = hq_llm::copilot_usage::fetch_quota()
        .await
        .ok()
        .filter(|quota| quota.is_metered())
        .map(|quota| quota.credits_used);
    let Some(used) = reading else {
        return Ok(Err(
            "it has a Copilot credit limit, and HQ could not read the credit counter".into(),
        ));
    };
    db.with_conn(|c| coder_jobs::set_credit_start(c, &job.task_id, used))?;
    Ok(Ok(job.credit_cap.map(|cap| used + cap)))
}

fn budget_words(job: &coder_jobs::Job) -> String {
    format!(
        "it used its budget of ${:.2} (spent ${:.2})",
        job.budget_usd, job.spent_usd
    )
}

/// Why a job's own limit ended its run, if it did.
fn stop_words(job: &coder_jobs::Job) -> Option<String> {
    match job.stop_reason.as_deref()? {
        coder_jobs::STOP_CREDITS => Some(format!(
            "it reached its limit of {:.0} Copilot credits",
            job.credit_cap.unwrap_or_default()
        )),
        _ => Some(budget_words(job)),
    }
}

/// What the worker is on: a reply still running, or a queue task held under a live lease or
/// moved to in progress by someone else (the owner working it by hand). `Free` lists the tasks
/// the worker itself left in progress with nothing running, which are parked, not waited on, each
/// with the reason. `cut_off` lists those a restart or provider error ended, with how long to
/// wait before trying again.
enum Busy {
    On(String),
    Free {
        dropped: Vec<(t::Task, String)>,
        cut_off: Vec<(t::Task, i64)>,
    },
}

/// A restart is over, so the run may go again now. A provider error or lost network needs time,
/// and more of it each try.
fn retry_wait(ask: &asks::AskRow, job: Option<&coder_jobs::Job>) -> i64 {
    if ask.error.as_deref() == Some(asks::RESTART_REASON) {
        return 0;
    }
    job.map_or(0, |j| j.attempts) * RETRY_STEP_SECS
}

fn busy_with(conn: &rusqlite::Connection, ttl: i64) -> Result<Busy> {
    let prefix = format!("{CALLER}:");
    if let Some(ask) = asks::pending_by_external_prefix(conn, ASK_SCOPE, &prefix)? {
        return Ok(Busy::On(ask.external_id.unwrap_or(ask.ask_id)));
    }
    let mut dropped = Vec::new();
    let mut cut_off = Vec::new();
    for task in queue(conn, t::STATUS_IN_PROGRESS)? {
        let latest =
            asks::latest_by_external_prefix(conn, ASK_SCOPE, &format!("{CALLER}:{}:", task.id))?;
        let Some(latest) = latest else {
            return Ok(Busy::On(task.display_id));
        };
        let job = coder_jobs::get(conn, &task.id)?;
        if let Some(why) = job.as_ref().and_then(stop_words) {
            dropped.push((task, why));
            continue;
        }
        if asks::was_cut_off(latest.error.as_deref()) {
            match job.as_ref().filter(|j| j.attempts >= MAX_ATTEMPTS) {
                Some(spent) => dropped.push((task, out_of_tries(spent))),
                None => cut_off.push((task, retry_wait(&latest, job.as_ref()))),
            }
            continue;
        }
        if t::live_lease(conn, &task.id, ttl)?.is_some() {
            return Ok(Busy::On(task.display_id));
        }
        dropped.push((task, STOPPED_WITHOUT_HANDOVER.to_string()));
    }
    Ok(Busy::Free { dropped, cut_off })
}

fn queue(conn: &rusqlite::Connection, status: &str) -> Result<Vec<t::Task>> {
    let filter = t::TaskFilter {
        tag: Some(DISPATCH_TAG.into()),
        status: Some(status.into()),
        sort: t::TaskSort::Priority,
        limit: Some(QUEUE_DEPTH),
        ..Default::default()
    };
    t::list_tasks(conn, &filter)
}

/// Open queue tasks, most urgent first, that nothing blocks, no one else holds, and that name
/// one of the repositories the worker may push to.
fn candidates(conn: &rusqlite::Connection, repos: &[String]) -> Result<Vec<t::Task>> {
    Ok(queue(conn, t::STATUS_TO_DO)?
        .into_iter()
        .filter(|task| {
            let free = task.assignees.iter().all(|a| a == HQ_ASSIGNEE);
            free && task.blocked_by.is_empty() && !parked(task) && names_a_repo(task, repos)
        })
        .collect())
}

fn parked(task: &t::Task) -> bool {
    has_tag(&task.tags, PARKED_TAG)
}

fn has_tag(tags: &[String], tag: &str) -> bool {
    tags.iter().any(|g| g.eq_ignore_ascii_case(tag))
}

fn names_a_repo(task: &t::Task, repos: &[String]) -> bool {
    named_repo(task, repos).is_some()
}

/// The allowed repository the task's title or description names as a whole word, so `o/r` is not
/// found inside `o/r2` or `xo/r`.
pub(super) fn named_repo<'a>(task: &t::Task, repos: &'a [String]) -> Option<&'a str> {
    let text = format!("{}\n{}", task.title, task.description).to_ascii_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':')))
        .map(repo_of)
        .collect();
    repos
        .iter()
        .find(|r| words.contains(&r.to_ascii_lowercase().as_str()))
        .map(String::as_str)
}

/// The `owner/name` a word stands for: itself, or the repository part of a GitHub URL
/// (`https://github.com/o/r/issues/3`, `github.com/o/r.git`).
fn repo_of(word: &str) -> &str {
    let word = word.trim_end_matches('.');
    let Some((_, path)) = word.split_once("github.com/") else {
        return word;
    };
    let mut end = 0;
    for (n, part) in path.split('/').take(2).enumerate() {
        end += part.len() + usize::from(n > 0);
    }
    path[..end].trim_end_matches(".git")
}

/// The ask's key names the task, its last change and the attempt: a task that changes (rework, an
/// edit) is asked about afresh, and a run that was cut off gets a new attempt.
fn ask_key(task: &t::Task, attempt: i64) -> String {
    format!("{CALLER}:{}:{}:a{attempt}", task.id, task.updated_at)
}

/// Asks HQ, through the same path as `hq_ask`, to work the task.
async fn ask_to_work(
    db: Arc<Database>,
    runner: crate::ask::RunnerCell,
    task: &t::Task,
    model: Option<&str>,
    job: AskJob,
    attempt: i64,
) -> Result<String> {
    let budget_usd = job.budget_usd;
    let ask = crate::ask::create_worker_ask_tool(db, runner, job);
    let args = json!({
        // The skill is named outright: its hints need not appear in a one-line question.
        "question": format!(
            "Load the {WORKER_SKILL} skill with load_skill and follow it to work HQ task {} end to end.",
            task.display_id
        ),
        "title": format!("Edge worker: {}", task.display_id),
        "mode": ASK_MODE,
        "caller": CALLER,
        "external_id": ask_key(task, attempt),
        "wait_secs": 0,
        // Anyone the owner lets file tasks may have written the task and its comments.
        "untrusted": true,
        "model": model,
        "budget_usd": budget_usd,
        "worker_preset": true,
    });
    let reply = ask.execute(args).await?;
    let ask_id = reply
        .get("ask_id")
        .and_then(|v| v.as_str())
        .context("hq_ask returned no ask id")?;
    Ok(ask_id.to_string())
}

/// Whether dispatch would take a task now: switched on and the worker skill active here. The skill
/// is checked against the set the chat loads from, so a reloaded config cannot start a turn that
/// lacks it.
pub fn enabled(config: &HqConfig) -> bool {
    config.github_write.dispatch_tasks && crate::skills::builtin::is_active(WORKER_SKILL)
}

/// One run of the worker: the ask that worked a task, and that task as it is now.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Run {
    pub ask_id: String,
    pub thread_id: String,
    /// `pending` while the reply runs, then `answered` or `failed`.
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub task_id: String,
    /// Absent when the task has since been deleted.
    pub task: Option<RunTask>,
    pub job: Option<coder_jobs::Job>,
    /// The pull request the worker named on its task, once it has one.
    pub pr_url: Option<String>,
    /// The newest notes on the task, oldest first.
    pub notes: Vec<String>,
}

/// Asks read per run shown: retries leave several for one task.
const RUNS_PER_TASK: usize = 5;
/// Notes a run's card shows.
const RUN_NOTES: usize = 5;
const PR_URL_MARK: &str = "https://github.com/";

/// The first pull request link in `text`.
fn pr_url_in(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>' | ','))
        .find(|word| word.starts_with(PR_URL_MARK) && word.contains("/pull/"))
        .map(|word| word.trim_end_matches('.').to_string())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunTask {
    pub display_id: String,
    pub title: String,
    pub status: String,
}

/// The worker's newest runs, newest first.
pub fn recent_runs(conn: &rusqlite::Connection, limit: usize) -> Result<Vec<Run>> {
    let prefix = format!("{CALLER}:");
    let asks = asks::recent_by_external_prefix(conn, ASK_SCOPE, &prefix, limit * RUNS_PER_TASK)?;
    let mut runs: Vec<Run> = Vec::with_capacity(limit);
    for ask in asks {
        // The key is `edge-worker:<task id>:<updated_at>:a<attempt>`; a task id holds no colon.
        let key = ask.external_id.unwrap_or_default();
        let task_id = key
            .strip_prefix(&prefix)
            .and_then(|rest| rest.split(':').next())
            .unwrap_or_default()
            .to_string();
        // Newest first, so the first ask seen for a task is its current attempt.
        if runs.len() == limit || runs.iter().any(|r| r.task_id == task_id) {
            continue;
        }
        let comments = t::list_comments(conn, &task_id).unwrap_or_default();
        let pr_url = comments.iter().rev().find_map(|c| pr_url_in(&c.body));
        let skip = comments.len().saturating_sub(RUN_NOTES);
        let notes = comments.into_iter().skip(skip).map(|c| c.body).collect();
        let job = coder_jobs::get(conn, &task_id)?;
        let task = t::get_task(conn, &task_id)?.map(|t| RunTask {
            display_id: t.display_id,
            title: t.title,
            status: t.status,
        });
        runs.push(Run {
            ask_id: ask.ask_id,
            thread_id: ask.thread_id,
            status: ask.status,
            started_at: ask.created_at,
            finished_at: ask.answered_at,
            task_id,
            task,
            job,
            pr_url,
            notes,
        });
    }
    Ok(runs)
}

/// Whether `model` names an enabled backend, or the model one declares, so a typo stops dispatch
/// instead of failing every task it starts. Without a `backends:` chain the router resolves it.
pub(super) fn model_is_declared(config: &HqConfig, model: &str) -> bool {
    let backends = &config.backends;
    if !backends.is_configured() {
        return true;
    }
    backends
        .backends
        .iter()
        .filter(|b| b.enabled)
        .any(|b| b.name == model || b.model.as_deref() == Some(model))
}

fn ask_for_first_confirmation(db: &Database, task: &t::Task, budget_usd: f64) {
    let item = ValueItem::new(
        INBOX_SOURCE,
        ValueKind::ActionNeeded,
        "HQ Coder is ready for its first task",
        format!(
            "{} is waiting. HQ will spend up to ${budget_usd:.2} of model credits on it and open a \
             pull request. Allow it to start.",
            task.title
        ),
    )
    .with_dedup_key(FIRST_TASK_KEY);
    if let Err(e) = hq_db::value_items::emit(db, &item) {
        tracing::warn!(error = %e, "edge-worker-dispatch: inbox item failed");
    }
}

fn warn_misconfigured(db: &Database, model: &str) {
    let item = ValueItem::new(
        INBOX_SOURCE,
        ValueKind::ActionNeeded,
        "Edge worker paused: unknown dispatch model",
        format!(
            "github_write.dispatch_model is '{model}', which no enabled backend serves. \
             Queued edge-task tasks wait until it names a backend or is removed."
        ),
    )
    .with_dedup_key(format!("{INBOX_SOURCE}-model-{model}"));
    if let Err(e) = hq_db::value_items::emit(db, &item) {
        tracing::warn!(error = %e, "edge-worker-dispatch: inbox item failed");
    }
}

pub(super) fn park(db: &Database, task: &t::Task, reason: &str) -> Result<()> {
    db.with_conn(|c| park_in(c, task, reason))?;
    notify(db, task, reason);
    Ok(())
}

/// Moves a task the worker dropped to blocked, with the reason as a comment.
fn park_in(conn: &rusqlite::Connection, task: &t::Task, reason: &str) -> Result<()> {
    let mut tags = task.tags.clone();
    if !parked(task) {
        tags.push(PARKED_TAG.into());
    }
    let patch = t::TaskPatch {
        status: Some(t::STATUS_BLOCKED.into()),
        blocked_reason: Some(Some(reason.into())),
        tags: Some(tags),
        ..Default::default()
    };
    t::update_task(conn, &task.id, &patch, Some(&task.status))?;
    t::add_comment(conn, &task.id, CALLER, reason, None)?;
    tracing::warn!(task = %task.display_id, reason, "edge-worker-dispatch: parked a task");
    Ok(())
}

fn notify(db: &Database, task: &t::Task, reason: &str) {
    let item = ValueItem::new(
        INBOX_SOURCE,
        ValueKind::ActionNeeded,
        format!("Edge worker parked {}", task.display_id),
        format!(
            "{}: {reason}. The worker skips it until you remove the {PARKED_TAG} tag and set it to do.",
            task.title
        ),
    )
    .with_dedup_key(format!("{INBOX_SOURCE}-{}-{}", task.id, task.updated_at));
    if let Err(e) = hq_db::value_items::emit(db, &item) {
        tracing::warn!(error = %e, "edge-worker-dispatch: inbox item failed");
    }
}

/// Whether the caller came in on a key whose text the owner has not vouched for: the handoff or
/// tasks key, or a claude.ai connector grant.
pub fn restricted_caller(args: &serde_json::Value) -> bool {
    hq_core::caller::is_tasks_scope(args)
        || hq_core::caller::is_handoff_scope(args)
        || hq_core::caller::connector_label(args).is_some()
}

/// Refuses a restricted caller ([`restricted_caller`]) that would put a task on the worker's
/// queue, change a queued task's tags (which would unpark it), or rewrite its text: the worker
/// runs a full HQ turn on that text.
pub fn refuse_queue_write(
    scoped: bool,
    new_tags: Option<&[String]>,
    current_tags: &[String],
    edits_text: bool,
) -> Result<()> {
    if !scoped {
        return Ok(());
    }
    let is_queue = |tags: &[String]| has_tag(tags, DISPATCH_TAG);
    if new_tags.is_some_and(is_queue) && !is_queue(current_tags) {
        anyhow::bail!("this connection cannot add the {DISPATCH_TAG} tag; ask the owner to");
    }
    if is_queue(current_tags) && new_tags.is_some_and(|new| !same_tags(new, current_tags)) {
        anyhow::bail!("this connection cannot change the tags of an {DISPATCH_TAG} task");
    }
    if edits_text && is_queue(current_tags) {
        anyhow::bail!(
            "this connection cannot edit the title or description of an {DISPATCH_TAG} task; add a comment instead"
        );
    }
    Ok(())
}

fn same_tags(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().all(|g| has_tag(b, g))
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
