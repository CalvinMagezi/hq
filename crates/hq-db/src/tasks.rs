//! Native task management: replaces ClickUp. Spaces > initiatives > tasks >
//! comments, tags as a join table (the primary filter/routing dimension).
//! Single write surface for both the MCP tool layer (`hq-tools::tasks`) and
//! the web REST layer (`hq-web::tasks_api`) — see docs/plans/native-tasks.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

pub const STATUS_TO_DO: &str = "to_do";
pub const STATUS_IN_PROGRESS: &str = "in_progress";
pub const STATUS_BLOCKED: &str = "blocked";
/// Owner-neutral handoff state (ClickUp's "ready for <person>" columns map here).
pub const STATUS_READY_FOR_REVIEW: &str = "ready_for_review";
pub const STATUS_COMPLETE: &str = "complete";

/// Every status a task may hold. Validation names these in its error.
pub const STATUSES: [&str; 5] = [
    STATUS_TO_DO,
    STATUS_IN_PROGRESS,
    STATUS_BLOCKED,
    STATUS_READY_FOR_REVIEW,
    STATUS_COMPLETE,
];

pub const EVENT_ENTERED_TO_DO: &str = "entered_to_do";
pub const EVENT_ENTERED_IN_PROGRESS: &str = "entered_in_progress";
pub const EVENT_ENTERED_BLOCKED: &str = "entered_blocked";
pub const EVENT_ENTERED_READY_FOR_REVIEW: &str = "entered_ready_for_review";
pub const EVENT_ENTERED_COMPLETE: &str = "entered_complete";

pub const PRIORITY_URGENT: &str = "urgent";
pub const PRIORITY_HIGH: &str = "high";
pub const PRIORITY_NORMAL: &str = "normal";
pub const PRIORITY_LOW: &str = "low";

pub const PRIORITIES: [&str; 4] = [
    PRIORITY_URGENT,
    PRIORITY_HIGH,
    PRIORITY_NORMAL,
    PRIORITY_LOW,
];

/// Largest page `list_tasks` returns. Larger lists are read with `offset`.
pub const MAX_LIST_LIMIT: usize = 500;

type ChangeHook = Box<dyn Fn() + Send + Sync>;

static CHANGE_HOOKS: std::sync::RwLock<Vec<ChangeHook>> = std::sync::RwLock::new(Vec::new());

/// Run `hook` after every task write in this process, so a watcher can react at
/// once instead of polling. Writes from other processes are not seen.
pub fn on_change(hook: impl Fn() + Send + Sync + 'static) {
    CHANGE_HOOKS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(Box::new(hook));
}

fn changed() {
    for hook in CHANGE_HOOKS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
    {
        hook();
    }
}

/// Runs `f` in one write transaction that takes the write lock up front, so a
/// read-then-write sequence decides on the state it then changes. Joins a
/// transaction the caller already holds, which then owns the commit.
pub fn in_write_tx<T>(conn: &Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    if !conn.is_autocommit() {
        return f(conn);
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let out = f(&tx)?;
    tx.commit()?;
    changed();
    Ok(out)
}

/// Refuses a status the board and every status query would not recognise.
pub fn validate_status(status: &str) -> Result<()> {
    if STATUSES.contains(&status) {
        return Ok(());
    }
    anyhow::bail!("unknown status '{status}', expected one of {}", STATUSES.join(", "))
}

pub fn validate_priority(priority: &str) -> Result<()> {
    if PRIORITIES.contains(&priority) {
        return Ok(());
    }
    anyhow::bail!("unknown priority '{priority}', expected one of {}", PRIORITIES.join(", "))
}

/// Fires the change hooks after a successful write, unless the connection is
/// inside a transaction: the transaction's owner fires them once it commits, so
/// a watcher never reads state that is about to roll back.
fn notify_on_ok<T>(conn: &Connection, result: Result<T>) -> Result<T> {
    if result.is_ok() {
        changed_outside_tx(conn);
    }
    result
}

fn changed_outside_tx(conn: &Connection) {
    if conn.is_autocommit() {
        changed();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Space {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Folder {
    pub id: String,
    pub space_id: String,
    pub name: String,
    pub slug: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Initiative {
    pub id: String,
    pub space_id: String,
    /// `None` = folderless, directly under the Space (ClickUp allows both).
    pub folder_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub id_prefix: String,
    pub next_sequence: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub initiative_id: String,
    pub display_id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: Option<String>,
    pub due_date: Option<String>,
    #[serde(skip_serializing)]
    pub clickup_task_id: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub updated_at: String,
    /// `None` = top level. Sub-tasks nest one level only.
    pub parent_task_id: Option<String>,
    pub start_date: Option<String>,
    /// Populated by a `task_tags` join, not a raw column.
    pub tags: Vec<String>,
    /// Internal ids of every task this one depends on (finish-to-start).
    pub depends_on: Vec<String>,
    /// Display ids of the dependencies that are not yet complete.
    pub blocked_by: Vec<String>,
    pub subtask_count: i64,
    pub subtask_done: i64,
    /// UTC `YYYY-MM-DD HH:MM:SS` of the first move into in_progress. `None` = unknown
    /// (legacy task) or never started; later attempts are in `list_task_events`.
    pub work_started_at: Option<String>,
    /// UTC timestamp of the first move into ready_for_review, `None` as above.
    pub first_ready_for_review_at: Option<String>,
    /// UTC timestamp of the latest move into complete, cleared when the task
    /// reopens. `None` = not complete, or completed before it was recorded.
    pub completed_at: Option<String>,
    /// Caller-supplied idempotency key, unique within the task's space.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

/// One lifecycle transition. `list_task_events` returns them oldest first.
#[derive(Debug, Clone, Serialize)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: String,
    pub event_type: String,
    pub occurred_at: String,
    /// `None` on events recorded before the full log existed.
    pub from_status: Option<String>,
    pub to_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskComment {
    pub id: i64,
    pub task_id: String,
    pub author: String,
    pub body: String,
    pub created_at: String,
    /// `comment`, or `message` for one agent session writing to another.
    pub kind: String,
    pub sender_session_id: Option<String>,
    pub to_session_id: Option<String>,
    pub reply_to: Option<i64>,
    pub delivered_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub space_id: Option<String>,
    pub initiative_id: Option<String>,
    pub status: Option<String>,
    pub tag: Option<String>,
    pub priority: Option<String>,
    /// `Some(None)` = top-level tasks only; `Some(Some(id))` = that task's sub-tasks.
    pub parent_task_id: Option<Option<String>>,
    /// Page size, at most `MAX_LIST_LIMIT`. `None` = the maximum.
    pub limit: Option<usize>,
    pub offset: usize,
}

/// Everything a new task needs besides its id and initiative.
#[derive(Debug, Clone, Default)]
pub struct NewTask<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub priority: Option<&'a str>,
    pub due_date: Option<&'a str>,
    pub start_date: Option<&'a str>,
    /// Id or display id of the parent; the sub-task must share its initiative.
    pub parent_task_id: Option<&'a str>,
    pub tags: &'a [String],
    pub created_by: &'a str,
    /// Idempotency key, unique per space. See `create_task_dedup`.
    pub external_id: Option<&'a str>,
}

/// A field left `None` is untouched. The `Option<Option<String>>` fields
/// distinguish "don't touch" from "clear to NULL".
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub priority: Option<Option<String>>,
    pub due_date: Option<Option<String>>,
    pub start_date: Option<Option<String>>,
    pub parent_task_id: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
}

mod crud;
mod links;
mod messages;
mod org;
mod rows;
#[cfg(test)]
mod tests;

pub use crud::*;
pub use links::*;
pub use messages::*;
pub use org::*;
pub use rows::{list_task_events, validate_date};
use rows::*;
