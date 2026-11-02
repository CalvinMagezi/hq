//! `edge-worker-dispatch`: with `github_write.dispatch_tasks` on, the daemon picks up open tasks
//! tagged `edge-worker` and has HQ work each one with the `edge-coding-worker` skill, one at a
//! time, so a task filed from anywhere becomes a pull request with no other machine involved.

use std::sync::Arc;

use anyhow::{Context, Result};
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::json;

/// Tasks carrying this tag are the worker's queue.
pub const DISPATCH_TAG: &str = "edge-worker";
/// The built-in skill the work follows; dispatch is off while it is not active.
const WORKER_SKILL: &str = "edge-coding-worker";
/// An assignee that marks a task as HQ's own, beside the unassigned ones.
const HQ_ASSIGNEE: &str = "hq";
const CALLER: &str = "edge-worker";
/// How far down the queue one tick looks for a task it may take.
const QUEUE_DEPTH: usize = 20;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Dispatch is not switched on, or the worker skill is not active here.
    Off,
    /// A worker task is already in progress; one runs at a time.
    Busy(String),
    /// Nothing to take.
    Idle,
    /// HQ was asked to work `task`; the reply runs in the chat thread of `ask_id`.
    Started { task: String, ask_id: String },
    /// HQ was already asked to work `task` and has not yet claimed it (or could not); the task
    /// is asked about again only once it changes.
    AlreadyAsked { task: String, ask_id: String },
}

/// One tick: start work on the next task when nothing else is running.
/// `runner` is where the reply runs, the web server's chat ([`crate::ask::shared_runner_cell`]).
pub async fn dispatch(
    db: Arc<Database>,
    config: &HqConfig,
    runner: crate::ask::RunnerCell,
) -> Result<Outcome> {
    let worker_active = crate::skills::builtin::active_for(config).contains(&WORKER_SKILL);
    // Without a web server in this process there is no chat to run the work in.
    let can_ask = runner.get().is_some();
    if !config.github_write.dispatch_tasks || !worker_active || !can_ask {
        return Ok(Outcome::Off);
    }
    if let Some(busy) = db.with_conn(running_task)? {
        return Ok(Outcome::Busy(busy));
    }
    let Some(task) = db.with_conn(next_task)? else {
        return Ok(Outcome::Idle);
    };
    let (ask_id, repeated) = ask_to_work(db, runner, &task).await?;
    let task = task.display_id;
    Ok(match repeated {
        true => Outcome::AlreadyAsked { task, ask_id },
        false => Outcome::Started { task, ask_id },
    })
}

fn worker_tasks(conn: &rusqlite::Connection, status: &str) -> Result<Vec<t::Task>> {
    let filter = t::TaskFilter {
        tag: Some(DISPATCH_TAG.into()),
        status: Some(status.into()),
        sort: t::TaskSort::Priority,
        limit: Some(QUEUE_DEPTH),
        ..Default::default()
    };
    t::list_tasks(conn, &filter)
}

/// The display id of a worker task already in progress, if any.
fn running_task(conn: &rusqlite::Connection) -> Result<Option<String>> {
    let running = worker_tasks(conn, t::STATUS_IN_PROGRESS)?;
    Ok(running.into_iter().next().map(|task| task.display_id))
}

/// The most urgent open worker task that nothing blocks and no one else holds.
fn next_task(conn: &rusqlite::Connection) -> Result<Option<t::Task>> {
    Ok(worker_tasks(conn, t::STATUS_TO_DO)?
        .into_iter()
        .find(|task| {
            let free = task.assignees.is_empty() || task.assignees.iter().any(|a| a == HQ_ASSIGNEE);
            free && task.blocked_by.is_empty()
        }))
}

/// Asks HQ, through the same path as `hq_ask`, to work the task. The key names the task and its
/// last change, so a tick that comes round before HQ has claimed it reuses the same ask instead
/// of starting a second reply.
async fn ask_to_work(
    db: Arc<Database>,
    runner: crate::ask::RunnerCell,
    task: &t::Task,
) -> Result<(String, bool)> {
    let tools = crate::ask::create_ask_tools_with(db, runner, None);
    let ask = tools
        .iter()
        .find(|tool| tool.name() == "hq_ask")
        .context("the hq_ask tool is missing")?;
    let args = json!({
        "question": format!("Work HQ task {} end to end.", task.display_id),
        "title": format!("Edge worker: {}", task.display_id),
        "mode": "full",
        "caller": CALLER,
        "external_id": format!("{CALLER}:{}:{}", task.id, task.updated_at),
        "wait_secs": 0,
    });
    let reply = ask.execute(args).await?;
    let ask_id = reply
        .get("ask_id")
        .and_then(|v| v.as_str())
        .context("hq_ask returned no ask id")?;
    let repeated = reply
        .get("deduplicated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok((ask_id.to_string(), repeated))
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
