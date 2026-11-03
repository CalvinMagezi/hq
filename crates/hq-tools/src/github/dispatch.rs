//! `edge-worker-dispatch`: with `github_write.dispatch_tasks` on, the daemon picks up open tasks
//! tagged `edge-task` that name a repository in `github_write.repos`, and has HQ work each one
//! with the `edge-coding-worker` skill, one at a time.

use std::sync::Arc;

use anyhow::{Context, Result};
use hq_core::config::HqConfig;
use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::ask_requests as asks;
use hq_db::tasks as t;
use serde_json::json;

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
const ASK_TOOL: &str = "hq_ask";
/// The ask scope and mode the worker's asks run under.
const ASK_SCOPE: &str = "full";
const ASK_MODE: &str = "full";
// ponytail: a bounded scan instead of a filtered query; enough for a personal queue.
const QUEUE_DEPTH: usize = 100;
/// Where the owner's inbox items about the worker come from.
const INBOX_SOURCE: &str = "edge-worker-dispatch";

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
}

/// One tick: start work on the next task when nothing else is running. Tasks the worker
/// dropped are parked as blocked, with a note in the owner's inbox, so none holds the queue.
/// `runner` is where the reply runs, the web server's chat ([`crate::ask::shared_runner_cell`]).
pub async fn dispatch(
    db: Arc<Database>,
    config: &HqConfig,
    runner: crate::ask::RunnerCell,
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
        Busy::Free(dropped) => {
            for task in dropped {
                park(
                    &db,
                    &task,
                    "HQ's worker stopped without handing the task over",
                )?;
            }
        }
    }
    let repos = config.github_write.repos.clone();
    let candidates = db.with_conn(move |c| candidates(c, &repos))?;
    for task in candidates {
        let key = ask_key(&task);
        let earlier = db.with_conn(move |c| asks::get_by_external(c, ASK_SCOPE, &key))?;
        if let Some(ask) = earlier {
            // The reply for this version of the task ended without taking it.
            let reason = ask
                .error
                .unwrap_or_else(|| "it did not claim the task".into());
            park(
                &db,
                &task,
                &format!("HQ's worker stopped before starting: {reason}"),
            )?;
            continue;
        }
        let model = config.github_write.dispatch_model.as_deref();
        let ask_id = ask_to_work(db.clone(), runner, &task, model).await?;
        return Ok(Outcome::Started {
            task: task.display_id,
            ask_id,
        });
    }
    Ok(Outcome::Idle)
}

/// What the worker is on: a reply still running, or a queue task held under a live lease or
/// moved to in progress by someone else (the owner working it by hand). `Free` lists the tasks
/// the worker itself left in progress with nothing running, which are parked, not waited on.
enum Busy {
    On(String),
    Free(Vec<t::Task>),
}

fn busy_with(conn: &rusqlite::Connection, ttl: i64) -> Result<Busy> {
    let prefix = format!("{CALLER}:");
    if let Some(ask) = asks::pending_by_external_prefix(conn, ASK_SCOPE, &prefix)? {
        return Ok(Busy::On(ask.external_id.unwrap_or(ask.ask_id)));
    }
    let mut dropped = Vec::new();
    for task in queue(conn, t::STATUS_IN_PROGRESS)? {
        let worked_by_hq =
            asks::latest_by_external_prefix(conn, ASK_SCOPE, &format!("{CALLER}:{}:", task.id))?
                .is_some();
        if !worked_by_hq || t::live_lease(conn, &task.id, ttl)?.is_some() {
            return Ok(Busy::On(task.display_id));
        }
        dropped.push(task);
    }
    Ok(Busy::Free(dropped))
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

/// Whether the task's title or description names an allowed repository as a whole word, so
/// `o/r` is not found inside `o/r2` or `xo/r`.
fn names_a_repo(task: &t::Task, repos: &[String]) -> bool {
    let text = format!("{}\n{}", task.title, task.description).to_ascii_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':')))
        .map(repo_of)
        .collect();
    repos
        .iter()
        .any(|r| words.contains(&r.to_ascii_lowercase().as_str()))
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

/// The ask's key names the task and its last change: a task that changes (rework, an edit) is
/// asked about afresh, and the same version never twice.
fn ask_key(task: &t::Task) -> String {
    format!("{CALLER}:{}:{}", task.id, task.updated_at)
}

/// Asks HQ, through the same path as `hq_ask`, to work the task.
async fn ask_to_work(
    db: Arc<Database>,
    runner: crate::ask::RunnerCell,
    task: &t::Task,
    model: Option<&str>,
) -> Result<String> {
    let tools = crate::ask::create_ask_tools_with(db, runner, None);
    let ask = tools
        .iter()
        .find(|tool| tool.name() == ASK_TOOL)
        .context("the hq_ask tool is missing")?;
    let args = json!({
        // The skill is named outright: its hints need not appear in a one-line question.
        "question": format!(
            "Load the {WORKER_SKILL} skill with load_skill and follow it to work HQ task {} end to end.",
            task.display_id
        ),
        "title": format!("Edge worker: {}", task.display_id),
        "mode": ASK_MODE,
        "caller": CALLER,
        "external_id": ask_key(task),
        "wait_secs": 0,
        // Anyone the owner lets file tasks may have written the task and its comments.
        "untrusted": true,
        "model": model,
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
    let asks = asks::recent_by_external_prefix(conn, ASK_SCOPE, &prefix, limit)?;
    let mut runs = Vec::with_capacity(asks.len());
    for ask in asks {
        // The key is `edge-worker:<task id>:<updated_at>`; a task id holds no colon.
        let key = ask.external_id.unwrap_or_default();
        let task_id = key
            .strip_prefix(&prefix)
            .and_then(|rest| rest.split(':').next())
            .unwrap_or_default()
            .to_string();
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
        });
    }
    Ok(runs)
}

/// Whether `model` names an enabled backend, or the model one declares, so a typo stops dispatch
/// instead of failing every task it starts. Without a `backends:` chain the router resolves it.
fn model_is_declared(config: &HqConfig, model: &str) -> bool {
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

fn park(db: &Database, task: &t::Task, reason: &str) -> Result<()> {
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
    crate::harness_session::is_tasks_scope(args)
        || crate::harness_session::is_handoff_scope(args)
        || crate::harness_session::connector_label(args).is_some()
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
