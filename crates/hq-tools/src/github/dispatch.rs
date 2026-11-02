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
    // The same set the chat loads skills from, so a reloaded config cannot start a turn that
    // lacks the skill.
    let worker_active = crate::skills::builtin::is_active(WORKER_SKILL);
    // Without a web server in this process there is no chat to run the work in.
    let can_ask = runner.get().is_some();
    if !config.github_write.dispatch_tasks || !worker_active || !can_ask {
        return Ok(Outcome::Off);
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
        let ask_id = ask_to_work(db.clone(), runner, &task).await?;
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
            free && task.blocked_by.is_empty() && names_a_repo(task, repos)
        })
        .collect())
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
) -> Result<String> {
    let tools = crate::ask::create_ask_tools_with(db, runner, None);
    let ask = tools
        .iter()
        .find(|tool| tool.name() == ASK_TOOL)
        .context("the hq_ask tool is missing")?;
    let args = json!({
        "question": format!("Work HQ task {} end to end.", task.display_id),
        "title": format!("Edge worker: {}", task.display_id),
        "mode": ASK_MODE,
        "caller": CALLER,
        "external_id": ask_key(task),
        "wait_secs": 0,
        // Anyone the owner lets file tasks may have written the task and its comments.
        "untrusted": true,
    });
    let reply = ask.execute(args).await?;
    let ask_id = reply
        .get("ask_id")
        .and_then(|v| v.as_str())
        .context("hq_ask returned no ask id")?;
    Ok(ask_id.to_string())
}

fn park(db: &Database, task: &t::Task, reason: &str) -> Result<()> {
    db.with_conn(|c| park_in(c, task, reason))?;
    notify(db, task, reason);
    Ok(())
}

/// Moves a task the worker dropped to blocked, with the reason as a comment.
fn park_in(conn: &rusqlite::Connection, task: &t::Task, reason: &str) -> Result<()> {
    let patch = t::TaskPatch {
        status: Some(t::STATUS_BLOCKED.into()),
        blocked_reason: Some(Some(reason.into())),
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
            "{}: {reason}. It is blocked until you look at it.",
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
/// queue or rewrite one already there: the worker runs a full HQ turn on that text.
pub fn refuse_queue_write(
    scoped: bool,
    new_tags: Option<&[String]>,
    current_tags: &[String],
    edits_text: bool,
) -> Result<()> {
    if !scoped {
        return Ok(());
    }
    let is_queue = |tags: &[String]| tags.iter().any(|g| g.eq_ignore_ascii_case(DISPATCH_TAG));
    if new_tags.is_some_and(is_queue) && !is_queue(current_tags) {
        anyhow::bail!("this connection cannot add the {DISPATCH_TAG} tag; ask the owner to");
    }
    if edits_text && is_queue(current_tags) {
        anyhow::bail!(
            "this connection cannot edit the title or description of an {DISPATCH_TAG} task; add a comment instead"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
