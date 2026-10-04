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

pub const EVENT_ENTERED_IN_PROGRESS: &str = "entered_in_progress";
pub const EVENT_ENTERED_READY_FOR_REVIEW: &str = "entered_ready_for_review";

pub const PRIORITY_URGENT: &str = "urgent";
pub const PRIORITY_HIGH: &str = "high";
pub const PRIORITY_NORMAL: &str = "normal";
pub const PRIORITY_LOW: &str = "low";

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

fn notify_on_ok<T>(result: Result<T>) -> Result<T> {
    if result.is_ok() {
        changed();
    }
    result
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
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskComment {
    pub id: i64,
    pub task_id: String,
    pub author: String,
    pub body: String,
    pub created_at: String,
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

fn row_to_space(row: &rusqlite::Row) -> rusqlite::Result<Space> {
    Ok(Space {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn row_to_folder(row: &rusqlite::Row) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get(0)?,
        space_id: row.get(1)?,
        name: row.get(2)?,
        slug: row.get(3)?,
        created_at: row.get(4)?,
    })
}

const INITIATIVE_COLS: &str =
    "id, space_id, folder_id, name, slug, id_prefix, next_sequence, created_at";

fn row_to_initiative(row: &rusqlite::Row) -> rusqlite::Result<Initiative> {
    Ok(Initiative {
        id: row.get(0)?,
        space_id: row.get(1)?,
        folder_id: row.get(2)?,
        name: row.get(3)?,
        slug: row.get(4)?,
        id_prefix: row.get(5)?,
        next_sequence: row.get(6)?,
        created_at: row.get(7)?,
    })
}

const TASK_COLS: &str = "t.id, t.initiative_id, t.display_id, t.title, t.description, t.status, \
     t.priority, t.due_date, t.clickup_task_id, t.created_by, t.created_at, t.updated_at, \
     t.parent_task_id, t.start_date, t.work_started_at, t.first_ready_for_review_at, t.external_id";

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        initiative_id: row.get(1)?,
        display_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        status: row.get(5)?,
        priority: row.get(6)?,
        due_date: row.get(7)?,
        clickup_task_id: row.get(8)?,
        created_by: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        parent_task_id: row.get(12)?,
        start_date: row.get(13)?,
        work_started_at: row.get(14)?,
        first_ready_for_review_at: row.get(15)?,
        external_id: row.get(16)?,
        tags: Vec::new(),
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        subtask_count: 0,
        subtask_done: 0,
    })
}

fn row_to_comment(row: &rusqlite::Row) -> rusqlite::Result<TaskComment> {
    Ok(TaskComment {
        id: row.get(0)?,
        task_id: row.get(1)?,
        author: row.get(2)?,
        body: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn event_for_status(status: &str) -> Option<(&'static str, &'static str)> {
    match status {
        STATUS_IN_PROGRESS => Some((EVENT_ENTERED_IN_PROGRESS, "work_started_at")),
        STATUS_READY_FOR_REVIEW => {
            Some((EVENT_ENTERED_READY_FOR_REVIEW, "first_ready_for_review_at"))
        }
        _ => None,
    }
}

/// Appends a lifecycle event and stamps the task's first-time summary column.
/// Runs inside `update_task`'s write transaction so a status and its event
/// commit together.
fn record_transition(conn: &Connection, task_id: &str, status: &str) -> Result<()> {
    let Some((event_type, summary_col)) = event_for_status(status) else {
        return Ok(());
    };
    let now: String = conn.query_row("SELECT datetime('now')", [], |r| r.get(0))?;
    conn.execute(
        "INSERT INTO task_events (task_id, event_type, occurred_at) VALUES (?1, ?2, ?3)",
        params![task_id, event_type, now],
    )?;
    conn.execute(
        &format!("UPDATE tasks SET {summary_col} = COALESCE({summary_col}, ?1) WHERE id = ?2"),
        params![now, task_id],
    )?;
    Ok(())
}

/// Lifecycle transitions of a task (id or display id), oldest first.
pub fn list_task_events(conn: &Connection, id_or_display_id: &str) -> Result<Vec<TaskEvent>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.task_id, e.event_type, e.occurred_at FROM task_events e \
         JOIN tasks t ON t.id = e.task_id \
         WHERE t.id = ?1 OR t.display_id = ?1 ORDER BY e.id",
    )?;
    let rows = stmt
        .query_map(params![id_or_display_id], |r| {
            Ok(TaskEvent {
                id: r.get(0)?,
                task_id: r.get(1)?,
                event_type: r.get(2)?,
                occurred_at: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

/// Fills the join-derived fields (tags, dependencies, sub-task counts) for a
/// batch of tasks with one query each, instead of one query per task.
fn hydrate(conn: &Connection, tasks: &mut [Task]) -> Result<()> {
    if tasks.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
    let marks = placeholders(ids.len());
    let index: std::collections::HashMap<String, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect();

    let mut stmt = conn.prepare(&format!(
        "SELECT task_id, tag FROM task_tags WHERE task_id IN ({marks}) ORDER BY tag"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(&ids), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for (task_id, tag) in rows.filter_map(|r| r.ok()) {
        tasks[index[&task_id]].tags.push(tag);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT d.task_id, b.id, b.display_id, b.status FROM task_dependencies d \
         JOIN tasks b ON b.id = d.depends_on_task_id \
         WHERE d.task_id IN ({marks}) ORDER BY b.display_id"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(&ids), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for (task_id, blocker_id, blocker_display, blocker_status) in rows.filter_map(|r| r.ok()) {
        let task = &mut tasks[index[&task_id]];
        task.depends_on.push(blocker_id);
        if blocker_status != STATUS_COMPLETE {
            task.blocked_by.push(blocker_display);
        }
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT parent_task_id, COUNT(*), SUM(status = '{STATUS_COMPLETE}') FROM tasks \
         WHERE parent_task_id IN ({marks}) GROUP BY parent_task_id"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(&ids), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for (parent_id, count, done) in rows.filter_map(|r| r.ok()) {
        let task = &mut tasks[index[&parent_id]];
        task.subtask_count = count;
        task.subtask_done = done;
    }
    Ok(())
}

fn hydrate_one(conn: &Connection, task: Option<Task>) -> Result<Option<Task>> {
    let Some(task) = task else { return Ok(None) };
    let mut batch = [task];
    hydrate(conn, &mut batch)?;
    let [task] = batch;
    Ok(Some(task))
}

/// Accepts only a real `YYYY-MM-DD` calendar date, the one format the web
/// date inputs and the Gantt view understand.
pub fn validate_date(field: &str, value: &str) -> Result<()> {
    if value.len() != 10 || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err() {
        anyhow::bail!("{field} must be a YYYY-MM-DD date, got '{value}'");
    }
    Ok(())
}

fn validate_schedule(start_date: Option<&str>, due_date: Option<&str>) -> Result<()> {
    if let Some(start) = start_date {
        validate_date("start_date", start)?;
    }
    if let Some(due) = due_date {
        validate_date("due_date", due)?;
    }
    // Validated YYYY-MM-DD strings order the same as the dates they encode.
    if let (Some(start), Some(due)) = (start_date, due_date)
        && start > due
    {
        anyhow::bail!("start_date {start} is after due_date {due}");
    }
    Ok(())
}

/// Resolves and checks a would-be parent: it must exist, be top level (one
/// level of nesting), and share the child's initiative.
fn resolve_parent(conn: &Connection, parent: &str, child_initiative_id: &str) -> Result<Task> {
    let parent_task =
        get_task(conn, parent)?.ok_or_else(|| anyhow::anyhow!("parent task {parent} not found"))?;
    if parent_task.parent_task_id.is_some() {
        anyhow::bail!(
            "{} is itself a sub-task; sub-tasks nest one level only",
            parent_task.display_id
        );
    }
    if parent_task.initiative_id != child_initiative_id {
        anyhow::bail!(
            "sub-task must be in the same initiative as its parent {}",
            parent_task.display_id
        );
    }
    Ok(parent_task)
}

fn subtask_ids(conn: &Connection, task_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM tasks WHERE parent_task_id = ?1")?;
    let ids = stmt
        .query_map(params![task_id], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

fn set_tags(conn: &Connection, task_id: &str, tags: &[String]) -> Result<()> {
    conn.execute("DELETE FROM task_tags WHERE task_id = ?1", params![task_id])?;
    for tag in tags {
        conn.execute(
            "INSERT OR IGNORE INTO task_tags (task_id, tag) VALUES (?1, ?2)",
            params![task_id, tag],
        )?;
    }
    Ok(())
}

// ─── Spaces ─────────────────────────────────────────────────────────────

pub fn create_space(conn: &Connection, id: &str, name: &str, slug: &str) -> Result<Space> {
    conn.execute(
        "INSERT INTO spaces (id, name, slug) VALUES (?1, ?2, ?3)",
        params![id, name, slug],
    )?;
    notify_on_ok(
        get_space(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("space {id} vanished immediately after creation")),
    )
}

pub fn get_space(conn: &Connection, id: &str) -> Result<Option<Space>> {
    Ok(conn
        .query_row(
            "SELECT id, name, slug, created_at FROM spaces WHERE id = ?1",
            params![id],
            row_to_space,
        )
        .optional()?)
}

pub fn list_spaces(conn: &Connection) -> Result<Vec<Space>> {
    let mut stmt = conn.prepare("SELECT id, name, slug, created_at FROM spaces ORDER BY name")?;
    let rows = stmt
        .query_map([], row_to_space)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

pub fn update_space(conn: &Connection, id: &str, name: &str) -> Result<Space> {
    let affected = conn.execute(
        "UPDATE spaces SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    if affected == 0 {
        anyhow::bail!("space {id} not found");
    }
    notify_on_ok(
        get_space(conn, id)?.ok_or_else(|| anyhow::anyhow!("space {id} vanished after update")),
    )
}

// ─── Folders ────────────────────────────────────────────────────────────

pub fn create_folder(
    conn: &Connection,
    id: &str,
    space_id: &str,
    name: &str,
    slug: &str,
) -> Result<Folder> {
    conn.execute(
        "INSERT INTO folders (id, space_id, name, slug) VALUES (?1, ?2, ?3, ?4)",
        params![id, space_id, name, slug],
    )?;
    notify_on_ok(
        get_folder(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("folder {id} vanished immediately after creation")),
    )
}

pub fn get_folder(conn: &Connection, id: &str) -> Result<Option<Folder>> {
    Ok(conn
        .query_row(
            "SELECT id, space_id, name, slug, created_at FROM folders WHERE id = ?1",
            params![id],
            row_to_folder,
        )
        .optional()?)
}

pub fn list_folders(conn: &Connection, space_id: Option<&str>) -> Result<Vec<Folder>> {
    let (sql, filtered) = match space_id {
        Some(_) => (
            "SELECT id, space_id, name, slug, created_at FROM folders WHERE space_id = ?1 ORDER BY name",
            true,
        ),
        None => (
            "SELECT id, space_id, name, slug, created_at FROM folders ORDER BY name",
            false,
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = if filtered {
        stmt.query_map(params![space_id.unwrap()], row_to_folder)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], row_to_folder)?
            .filter_map(|r| r.ok())
            .collect()
    };
    Ok(rows)
}

// ─── Initiatives ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn create_initiative(
    conn: &Connection,
    id: &str,
    space_id: &str,
    folder_id: Option<&str>,
    name: &str,
    slug: &str,
    id_prefix: &str,
) -> Result<Initiative> {
    conn.execute(
        "INSERT INTO initiatives (id, space_id, folder_id, name, slug, id_prefix) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, space_id, folder_id, name, slug, id_prefix],
    )?;
    notify_on_ok(
        get_initiative(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("initiative {id} vanished immediately after creation")),
    )
}

pub fn get_initiative(conn: &Connection, id: &str) -> Result<Option<Initiative>> {
    Ok(conn
        .query_row(
            &format!("SELECT {INITIATIVE_COLS} FROM initiatives WHERE id = ?1"),
            params![id],
            row_to_initiative,
        )
        .optional()?)
}

/// `folder_id` filter: `None` (the arg omitted) means no filter on folder;
/// `Some(None)` filters to folderless initiatives only; `Some(Some(id))`
/// filters to that folder.
pub fn list_initiatives(
    conn: &Connection,
    space_id: Option<&str>,
    folder_id: Option<Option<&str>>,
) -> Result<Vec<Initiative>> {
    let mut sql = format!("SELECT {INITIATIVE_COLS} FROM initiatives");
    let mut conditions: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(sid) = space_id {
        conditions.push("space_id = ?");
        vals.push(Box::new(sid.to_string()));
    }
    match folder_id {
        Some(Some(fid)) => {
            conditions.push("folder_id = ?");
            vals.push(Box::new(fid.to_string()));
        }
        Some(None) => conditions.push("folder_id IS NULL"),
        None => {}
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY name");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), row_to_initiative)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Atomically allocates and returns the next `"{prefix}-{seq:03}"` display id
/// for an initiative. A single `UPDATE ... RETURNING` statement so the
/// read-and-increment can't race across pooled connections (a plain
/// read-then-write here would let two concurrent callers both observe the
/// same pre-increment value and mint duplicate display ids).
pub fn next_display_id(conn: &Connection, initiative_id: &str) -> Result<String> {
    let (seq, prefix): (i64, String) = conn.query_row(
        "UPDATE initiatives SET next_sequence = next_sequence + 1
         WHERE id = ?1
         RETURNING next_sequence - 1, id_prefix",
        params![initiative_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(format!("{prefix}-{seq:03}"))
}

// ─── Tasks ──────────────────────────────────────────────────────────────

pub fn create_task(
    conn: &Connection,
    id: &str,
    initiative_id: &str,
    new: &NewTask,
) -> Result<Task> {
    validate_schedule(new.start_date, new.due_date)?;
    let parent_id = match new.parent_task_id {
        Some(parent) => Some(resolve_parent(conn, parent, initiative_id)?.id),
        None => None,
    };
    let external = external_key(conn, initiative_id, new.external_id)?;
    let display_id = next_display_id(conn, initiative_id)?;
    conn.execute(
        "INSERT INTO tasks (id, initiative_id, display_id, title, description, priority, due_date, \
         created_by, parent_task_id, start_date, external_id, external_space_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            initiative_id,
            display_id,
            new.title,
            new.description,
            new.priority,
            new.due_date,
            new.created_by,
            parent_id,
            new.start_date,
            external.as_ref().map(|(_, ext)| ext),
            external.as_ref().map(|(space, _)| space)
        ],
    )?;
    set_tags(conn, id, new.tags)?;
    notify_on_ok(
        get_task(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("task {id} vanished immediately after creation")),
    )
}

/// Longest accepted `external_id`.
pub const MAX_EXTERNAL_ID_LEN: usize = 200;

/// The (space id, trimmed external id) a new task is keyed under. A blank id
/// means no key; one over the length cap is an error rather than truncated, so
/// two long ids can never collide silently.
fn external_key(
    conn: &Connection,
    initiative_id: &str,
    external_id: Option<&str>,
) -> Result<Option<(String, String)>> {
    let Some(ext) = external_id.map(str::trim).filter(|e| !e.is_empty()) else {
        return Ok(None);
    };
    if ext.chars().count() > MAX_EXTERNAL_ID_LEN {
        anyhow::bail!("external_id is longer than {MAX_EXTERNAL_ID_LEN} characters");
    }
    let space: String = conn.query_row(
        "SELECT space_id FROM initiatives WHERE id = ?1",
        params![initiative_id],
        |r| r.get(0),
    )?;
    Ok(Some((space, ext.to_string())))
}

/// The task a space already holds under `external_id`.
pub fn find_by_external_id(
    conn: &Connection,
    space_id: &str,
    external_id: &str,
) -> Result<Option<Task>> {
    let task = conn
        .query_row(
            &format!(
                "SELECT {TASK_COLS} FROM tasks t WHERE t.external_space_id = ?1 AND t.external_id = ?2"
            ),
            params![space_id, external_id.trim()],
            row_to_task,
        )
        .optional()?;
    hydrate_one(conn, task)
}

/// `create_task`, except a task already keyed by the same `external_id` in the
/// initiative's space is returned instead of a duplicate. The flag says whether
/// a task was created. The unique index settles a race between two writers.
pub fn create_task_dedup(
    conn: &Connection,
    id: &str,
    initiative_id: &str,
    new: &NewTask,
) -> Result<(Task, bool)> {
    let key = external_key(conn, initiative_id, new.external_id)?;
    let existing = |key: &Option<(String, String)>| match key {
        Some((space, ext)) => find_by_external_id(conn, space, ext),
        None => Ok(None),
    };
    if let Some(task) = existing(&key)? {
        return Ok((task, false));
    }
    match create_task(conn, id, initiative_id, new) {
        Ok(task) => Ok((task, true)),
        // A concurrent writer took the key between the lookup and the insert.
        Err(e) => existing(&key)?.map(|t| (t, false)).ok_or(e),
    }
}

pub fn get_task(conn: &Connection, id_or_display_id: &str) -> Result<Option<Task>> {
    let task = conn
        .query_row(
            &format!("SELECT {TASK_COLS} FROM tasks t WHERE t.id = ?1 OR t.display_id = ?1"),
            params![id_or_display_id],
            row_to_task,
        )
        .optional()?;
    hydrate_one(conn, task)
}

pub fn list_tasks(conn: &Connection, filter: &TaskFilter) -> Result<Vec<Task>> {
    let mut sql =
        format!("SELECT {TASK_COLS} FROM tasks t JOIN initiatives i ON i.id = t.initiative_id");
    let mut conditions: Vec<&'static str> = Vec::new();
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if filter.tag.is_some() {
        sql.push_str(" JOIN task_tags tg ON tg.task_id = t.id");
    }
    if let Some(tag) = &filter.tag {
        conditions.push("tg.tag = ?");
        vals.push(Box::new(tag.clone()));
    }
    if let Some(space_id) = &filter.space_id {
        conditions.push("i.space_id = ?");
        vals.push(Box::new(space_id.clone()));
    }
    if let Some(initiative_id) = &filter.initiative_id {
        conditions.push("t.initiative_id = ?");
        vals.push(Box::new(initiative_id.clone()));
    }
    if let Some(status) = &filter.status {
        conditions.push("t.status = ?");
        vals.push(Box::new(status.clone()));
    }
    if let Some(priority) = &filter.priority {
        conditions.push("t.priority = ?");
        vals.push(Box::new(priority.clone()));
    }
    match &filter.parent_task_id {
        Some(Some(parent)) => {
            conditions
                .push("t.parent_task_id = (SELECT id FROM tasks WHERE id = ? OR display_id = ?)");
            vals.push(Box::new(parent.clone()));
            vals.push(Box::new(parent.clone()));
        }
        Some(None) => conditions.push("t.parent_task_id IS NULL"),
        None => {}
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY t.updated_at DESC, t.created_at DESC, t.display_id DESC LIMIT 500");

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let mut tasks: Vec<Task> = stmt
        .query_map(param_refs.as_slice(), row_to_task)?
        .filter_map(|r| r.ok())
        .collect();
    hydrate(conn, &mut tasks)?;
    Ok(tasks)
}

/// Checks the parent and schedule a patch would produce against the task's
/// current state, before anything is written.
fn validate_patch(conn: &Connection, current: &Task, patch: &TaskPatch) -> Result<()> {
    let start = match &patch.start_date {
        Some(v) => v.as_deref(),
        None => current.start_date.as_deref(),
    };
    let due = match &patch.due_date {
        Some(v) => v.as_deref(),
        None => current.due_date.as_deref(),
    };
    // Only re-validate the format of dates this patch actually sets, so a
    // legacy value already in the row can't block an unrelated edit.
    if let Some(Some(v)) = &patch.start_date {
        validate_date("start_date", v)?;
    }
    if let Some(Some(v)) = &patch.due_date {
        validate_date("due_date", v)?;
    }
    if patch.start_date.is_some() || patch.due_date.is_some() {
        validate_schedule(start, due)?;
    }

    let Some(Some(parent)) = &patch.parent_task_id else {
        return Ok(());
    };
    let parent_task = resolve_parent(conn, parent, &current.initiative_id)?;
    if parent_task.id == current.id {
        anyhow::bail!("a task cannot be its own parent");
    }
    if current.subtask_count > 0 {
        anyhow::bail!(
            "{} has sub-tasks, so it cannot become a sub-task itself",
            current.display_id
        );
    }
    Ok(())
}

/// Claim-safe update: when `expected_status` is set, the underlying `UPDATE`
/// carries `AND status = ?`, so two agents racing the same transition (e.g.
/// both claiming a `to_do` task) have exactly one winner — the loser's
/// `rows_affected() == 0` surfaces as an error instead of silently
/// overwriting the winner's write.
pub fn update_task(
    conn: &Connection,
    id: &str,
    patch: &TaskPatch,
    expected_status: Option<&str>,
) -> Result<Task> {
    // IMMEDIATE takes the write lock before the status read, so "did the status
    // change" is decided on the same state the UPDATE then modifies.
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let conn = &*tx;
    let current = get_task(conn, id)?.ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;
    validate_patch(conn, &current, patch)?;

    // 'static: only literals can become part of the statement text.
    let mut sets: Vec<&'static str> = vec!["updated_at = datetime('now')"];
    let mut vals: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(title) = &patch.title {
        sets.push("title = ?");
        vals.push(Box::new(title.clone()));
    }
    if let Some(description) = &patch.description {
        sets.push("description = ?");
        vals.push(Box::new(description.clone()));
    }
    if let Some(status) = &patch.status {
        sets.push("status = ?");
        vals.push(Box::new(status.clone()));
    }
    if let Some(priority) = &patch.priority {
        sets.push("priority = ?");
        vals.push(Box::new(priority.clone()));
    }
    if let Some(due_date) = &patch.due_date {
        sets.push("due_date = ?");
        vals.push(Box::new(due_date.clone()));
    }
    if let Some(start_date) = &patch.start_date {
        sets.push("start_date = ?");
        vals.push(Box::new(start_date.clone()));
    }
    if let Some(parent) = &patch.parent_task_id {
        sets.push("parent_task_id = (SELECT id FROM tasks WHERE id = ? OR display_id = ?)");
        vals.push(Box::new(parent.clone()));
        vals.push(Box::new(parent.clone()));
    }

    let mut sql = format!("UPDATE tasks SET {} WHERE id = ?", sets.join(", "));
    vals.push(Box::new(current.id.clone()));
    if let Some(expected) = expected_status {
        sql.push_str(" AND status = ?");
        vals.push(Box::new(expected.to_string()));
    }

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let affected = conn.execute(&sql, param_refs.as_slice())?;

    if affected == 0 {
        match expected_status {
            Some(expected) => {
                anyhow::bail!("task {id} was not in expected status '{expected}' (claim conflict)")
            }
            None => anyhow::bail!("task {id} not found"),
        }
    }

    if let Some(tags) = &patch.tags {
        set_tags(conn, &current.id, tags)?;
    }
    if let Some(status) = patch.status.as_deref().filter(|s| *s != current.status) {
        record_transition(conn, &current.id, status)?;
    }

    let updated = get_task(conn, &current.id)?
        .ok_or_else(|| anyhow::anyhow!("task {id} vanished after update"))?;
    tx.commit()?;
    changed();
    Ok(updated)
}

/// `PRAGMA foreign_keys=ON` (see `pool.rs`) means child rows must go first —
/// SQLite rejects the parent delete otherwise instead of silently orphaning.
/// A task with sub-tasks is only deleted when `cascade` is set, taking its
/// sub-tasks with it. Returns the internal ids of every deleted task.
pub fn delete_task(conn: &Connection, id: &str, cascade: bool) -> Result<Vec<String>> {
    let task = get_task(conn, id)?.ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;
    let children = subtask_ids(conn, &task.id)?;
    if !children.is_empty() && !cascade {
        anyhow::bail!(
            "{} has {} sub-task(s); delete them first or pass cascade",
            task.display_id,
            children.len()
        );
    }

    let tx = conn.unchecked_transaction()?;
    let mut deleted = children;
    deleted.push(task.id);
    for task_id in &deleted {
        tx.execute(
            "DELETE FROM task_comments WHERE task_id = ?1",
            params![task_id],
        )?;
        tx.execute("DELETE FROM task_tags WHERE task_id = ?1", params![task_id])?;
        tx.execute(
            "DELETE FROM task_events WHERE task_id = ?1",
            params![task_id],
        )?;
        tx.execute(
            "DELETE FROM task_dependencies WHERE task_id = ?1 OR depends_on_task_id = ?1",
            params![task_id],
        )?;
        tx.execute("DELETE FROM tasks WHERE id = ?1", params![task_id])?;
    }
    tx.commit()?;
    changed();
    Ok(deleted)
}

// ─── Dependencies ───────────────────────────────────────────────────────

fn resolve_pair(conn: &Connection, task: &str, depends_on: &str) -> Result<(Task, Task)> {
    let dependent =
        get_task(conn, task)?.ok_or_else(|| anyhow::anyhow!("task {task} not found"))?;
    let blocker = get_task(conn, depends_on)?
        .ok_or_else(|| anyhow::anyhow!("dependency {depends_on} not found"))?;
    Ok((dependent, blocker))
}

fn touch(conn: &Connection, task_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE tasks SET updated_at = datetime('now') WHERE id = ?1",
        params![task_id],
    )?;
    Ok(())
}

/// Records that `task` cannot finish before `depends_on` does. Rejects
/// self-dependencies and anything that would close a cycle. Adding an
/// existing dependency is a no-op.
pub fn add_dependency(
    conn: &Connection,
    task: &str,
    depends_on: &str,
    created_by: &str,
) -> Result<()> {
    let (dependent, blocker) = resolve_pair(conn, task, depends_on)?;
    if dependent.id == blocker.id {
        anyhow::bail!("a task cannot depend on itself");
    }
    let closes_cycle: bool = conn
        .query_row(
            "WITH RECURSIVE chain(id) AS (
                 SELECT ?1
                 UNION
                 SELECT d.depends_on_task_id FROM task_dependencies d JOIN chain c ON d.task_id = c.id
             )
             SELECT 1 FROM chain WHERE id = ?2 LIMIT 1",
            params![blocker.id, dependent.id],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if closes_cycle {
        anyhow::bail!(
            "{} depending on {} would create a dependency cycle",
            dependent.display_id,
            blocker.display_id
        );
    }
    conn.execute(
        "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_task_id, created_by) \
         VALUES (?1, ?2, ?3)",
        params![dependent.id, blocker.id, created_by],
    )?;
    notify_on_ok(touch(conn, &dependent.id))
}

pub fn remove_dependency(conn: &Connection, task: &str, depends_on: &str) -> Result<()> {
    let (dependent, blocker) = resolve_pair(conn, task, depends_on)?;
    conn.execute(
        "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_task_id = ?2",
        params![dependent.id, blocker.id],
    )?;
    notify_on_ok(touch(conn, &dependent.id))
}

fn tasks_by_ids(conn: &Connection, sql: &str, id: &str) -> Result<Vec<Task>> {
    let mut stmt = conn.prepare(sql)?;
    let ids: Vec<String> = stmt
        .query_map(params![id], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    let mut tasks = Vec::with_capacity(ids.len());
    for task_id in ids {
        if let Some(task) = get_task(conn, &task_id)? {
            tasks.push(task);
        }
    }
    Ok(tasks)
}

/// Tasks that depend on `task_id` (the ones it is blocking).
pub fn list_dependents(conn: &Connection, task_id: &str) -> Result<Vec<Task>> {
    tasks_by_ids(
        conn,
        "SELECT task_id FROM task_dependencies WHERE depends_on_task_id = ?1",
        task_id,
    )
}

pub fn list_subtasks(conn: &Connection, parent_id: &str) -> Result<Vec<Task>> {
    tasks_by_ids(
        conn,
        "SELECT id FROM tasks WHERE parent_task_id = ?1 ORDER BY display_id",
        parent_id,
    )
}

/// Open dependents of `completed_task_id` whose every dependency is now
/// complete: the tasks this completion just unblocked.
pub fn newly_unblocked(conn: &Connection, completed_task_id: &str) -> Result<Vec<Task>> {
    tasks_by_ids(
        conn,
        "SELECT d.task_id FROM task_dependencies d JOIN tasks dep ON dep.id = d.task_id
         WHERE d.depends_on_task_id = ?1 AND dep.status <> 'complete'
           AND NOT EXISTS (
               SELECT 1 FROM task_dependencies d2 JOIN tasks b ON b.id = d2.depends_on_task_id
               WHERE d2.task_id = d.task_id AND b.status <> 'complete'
           )",
        completed_task_id,
    )
}

// ─── Comments ───────────────────────────────────────────────────────────

/// `created_at` is settable (defaults to now) so the migration can preserve
/// original ClickUp comment timestamps instead of stamping import time.
pub fn add_comment(
    conn: &Connection,
    task_id: &str,
    author: &str,
    body: &str,
    created_at: Option<&str>,
) -> Result<TaskComment> {
    conn.execute(
        "INSERT INTO task_comments (task_id, author, body, created_at) \
         VALUES (?1, ?2, ?3, COALESCE(?4, datetime('now')))",
        params![task_id, author, body, created_at],
    )?;
    let id = conn.last_insert_rowid();
    changed();
    Ok(conn.query_row(
        "SELECT id, task_id, author, body, created_at FROM task_comments WHERE id = ?1",
        params![id],
        row_to_comment,
    )?)
}

pub fn list_comments(conn: &Connection, task_id: &str) -> Result<Vec<TaskComment>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, author, body, created_at FROM task_comments \
         WHERE task_id = ?1 ORDER BY created_at",
    )?;
    let rows = stmt
        .query_map(params![task_id], row_to_comment)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    #[test]
    fn event_for_status_only_names_real_task_columns() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        for status in [STATUS_IN_PROGRESS, STATUS_READY_FOR_REVIEW] {
            let (_, column) = event_for_status(status).unwrap();
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM pragma_table_info('tasks') WHERE name = ?1",
                    [column],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(exists, "{column} is not a tasks column");
        }
    }

    #[test]
    fn event_for_status_rejects_unknown_and_hostile_statuses() {
        for status in [
            "",
            "complete",
            "work_started_at",
            "in_progress'; DROP TABLE tasks; --",
        ] {
            assert_eq!(event_for_status(status), None, "{status}");
        }
    }

    use super::*;
    use crate::pool::Database;

    fn setup() -> (Database, String) {
        let db = Database::open_memory().unwrap();
        let initiative_id = db
            .with_conn(|c| {
                create_initiative(
                    c, "in-1", "personal", None, "Agent HQ", "agent-hq", "AGENT-HQ",
                )?;
                Ok("in-1".to_string())
            })
            .unwrap();
        (db, initiative_id)
    }

    fn new_task<'a>(title: &'a str, tags: &'a [String]) -> NewTask<'a> {
        NewTask {
            title,
            tags,
            created_by: "test",
            ..Default::default()
        }
    }

    fn make(db: &Database, id: &str, initiative_id: &str, parent: Option<&str>) -> Result<Task> {
        db.with_conn(|c| {
            create_task(
                c,
                id,
                initiative_id,
                &NewTask {
                    parent_task_id: parent,
                    ..new_task("Task", &[])
                },
            )
        })
    }

    fn set_status(db: &Database, id: &str, status: &str) {
        let patch = TaskPatch {
            status: Some(status.to_string()),
            ..Default::default()
        };
        db.with_conn(|c| update_task(c, id, &patch, None)).unwrap();
    }

    fn get(db: &Database, id: &str) -> Task {
        db.with_conn(|c| get_task(c, id)).unwrap().unwrap()
    }

    #[test]
    fn display_ids_are_sequential_and_unique() {
        let (db, initiative_id) = setup();
        let ids: Vec<String> = (0..5)
            .map(|i| {
                make(&db, &format!("tk-{i}"), &initiative_id, None)
                    .unwrap()
                    .display_id
            })
            .collect();
        assert_eq!(
            ids,
            vec![
                "AGENT-HQ-001",
                "AGENT-HQ-002",
                "AGENT-HQ-003",
                "AGENT-HQ-004",
                "AGENT-HQ-005"
            ]
        );
    }

    #[test]
    fn get_task_resolves_by_display_id() {
        let (db, initiative_id) = setup();
        let tags = ["hq".to_string()];
        db.with_conn(|c| create_task(c, "tk-1", &initiative_id, &new_task("Task", &tags)))
            .unwrap();
        let found = get(&db, "AGENT-HQ-001");
        assert_eq!(found.id, "tk-1");
        assert_eq!(found.tags, vec!["hq".to_string()]);
        assert!(found.parent_task_id.is_none() && found.start_date.is_none());
    }

    #[test]
    fn claim_safe_update_rejects_status_conflict() {
        let (db, initiative_id) = setup();
        make(&db, "tk-1", &initiative_id, None).unwrap();

        let patch = TaskPatch {
            status: Some(STATUS_IN_PROGRESS.to_string()),
            ..Default::default()
        };
        db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)))
            .unwrap();

        // Second claim attempt still expects to_do — must fail, task is now in_progress.
        let result = db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)));
        assert!(result.is_err());
    }

    #[test]
    fn retag_by_display_id_writes_internal_id() {
        let (db, initiative_id) = setup();
        make(&db, "tk-1", &initiative_id, None).unwrap();
        let patch = TaskPatch {
            tags: Some(vec!["hq".to_string()]),
            ..Default::default()
        };
        db.with_conn(|c| update_task(c, "AGENT-HQ-001", &patch, None))
            .unwrap();
        assert_eq!(get(&db, "tk-1").tags, vec!["hq".to_string()]);
    }

    #[test]
    fn delete_task_removes_children_first() {
        let (db, initiative_id) = setup();
        let tags = ["hq".to_string()];
        db.with_conn(|c| create_task(c, "tk-1", &initiative_id, &new_task("Task", &tags)))
            .unwrap();
        db.with_conn(|c| add_comment(c, "tk-1", "test", "a comment", None))
            .unwrap();

        db.with_conn(|c| delete_task(c, "tk-1", false)).unwrap();

        assert!(db.with_conn(|c| get_task(c, "tk-1")).unwrap().is_none());
    }

    #[test]
    fn subtasks_nest_one_level_and_share_the_initiative() {
        let (db, initiative_id) = setup();
        make(&db, "tk-p", &initiative_id, None).unwrap();
        let child = make(&db, "tk-c", &initiative_id, Some("AGENT-HQ-001")).unwrap();
        assert_eq!(child.parent_task_id.as_deref(), Some("tk-p"));
        assert_eq!(child.display_id, "AGENT-HQ-002");

        assert!(make(&db, "tk-g", &initiative_id, Some("tk-c")).is_err());

        db.with_conn(|c| create_initiative(c, "in-2", "personal", None, "Other", "other", "OTHER"))
            .unwrap();
        assert!(make(&db, "tk-x", "in-2", Some("tk-p")).is_err());

        set_status(&db, "tk-c", STATUS_COMPLETE);
        let parent = get(&db, "tk-p");
        assert_eq!((parent.subtask_count, parent.subtask_done), (1, 1));
    }

    #[test]
    fn reparenting_rejects_a_task_that_has_subtasks() {
        let (db, initiative_id) = setup();
        make(&db, "tk-a", &initiative_id, None).unwrap();
        make(&db, "tk-b", &initiative_id, None).unwrap();
        make(&db, "tk-b1", &initiative_id, Some("tk-b")).unwrap();

        let into_a = TaskPatch {
            parent_task_id: Some(Some("tk-a".into())),
            ..Default::default()
        };
        assert!(
            db.with_conn(|c| update_task(c, "tk-b", &into_a, None))
                .is_err()
        );

        let promote = TaskPatch {
            parent_task_id: Some(None),
            ..Default::default()
        };
        db.with_conn(|c| update_task(c, "tk-b1", &promote, None))
            .unwrap();
        assert!(get(&db, "tk-b1").parent_task_id.is_none());
    }

    #[test]
    fn dates_are_validated_and_ordered() {
        let (db, initiative_id) = setup();
        let bad = NewTask {
            start_date: Some("2026-10-05"),
            due_date: Some("2026-10-01"),
            ..new_task("T", &[])
        };
        assert!(
            db.with_conn(|c| create_task(c, "tk-1", &initiative_id, &bad))
                .is_err()
        );

        let garbage = NewTask {
            due_date: Some("1759276800000"),
            ..new_task("T", &[])
        };
        assert!(
            db.with_conn(|c| create_task(c, "tk-2", &initiative_id, &garbage))
                .is_err()
        );

        make(&db, "tk-3", &initiative_id, None).unwrap();
        let patch = TaskPatch {
            start_date: Some(Some("2026-10-01".into())),
            due_date: Some(Some("2026-10-05".into())),
            ..Default::default()
        };
        let task = db
            .with_conn(|c| update_task(c, "tk-3", &patch, None))
            .unwrap();
        assert_eq!(task.start_date.as_deref(), Some("2026-10-01"));
    }

    #[test]
    fn dependencies_reject_self_and_cycles() {
        let (db, initiative_id) = setup();
        for id in ["tk-a", "tk-b", "tk-c"] {
            make(&db, id, &initiative_id, None).unwrap();
        }
        let add = |task: &str, dep: &str| db.with_conn(|c| add_dependency(c, task, dep, "test"));

        assert!(add("tk-a", "tk-a").is_err());
        add("tk-a", "tk-b").unwrap();
        assert!(add("tk-b", "tk-a").is_err());
        add("tk-b", "tk-c").unwrap();
        assert!(add("tk-c", "tk-a").is_err());
        add("tk-a", "tk-b").unwrap();

        assert_eq!(get(&db, "tk-a").depends_on, vec!["tk-b".to_string()]);
        assert_eq!(
            get(&db, "tk-a").blocked_by,
            vec!["AGENT-HQ-002".to_string()]
        );
    }

    #[test]
    fn completing_the_last_blocker_unblocks_dependents() {
        let (db, initiative_id) = setup();
        for id in ["tk-a", "tk-b", "tk-c", "tk-d"] {
            make(&db, id, &initiative_id, None).unwrap();
        }
        db.with_conn(|c| {
            add_dependency(c, "tk-a", "tk-b", "test")?;
            add_dependency(c, "tk-a", "tk-c", "test")?;
            add_dependency(c, "tk-d", "tk-b", "test")
        })
        .unwrap();

        set_status(&db, "tk-b", STATUS_COMPLETE);
        let unblocked: Vec<String> = db
            .with_conn(|c| newly_unblocked(c, "tk-b"))
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(unblocked, vec!["tk-d".to_string()]);
        assert_eq!(
            get(&db, "tk-a").blocked_by,
            vec!["AGENT-HQ-003".to_string()]
        );

        db.with_conn(|c| remove_dependency(c, "tk-a", "tk-c"))
            .unwrap();
        assert!(get(&db, "tk-a").blocked_by.is_empty());
    }

    #[test]
    fn delete_with_subtasks_needs_cascade_and_cleans_dependencies() {
        let (db, initiative_id) = setup();
        make(&db, "tk-p", &initiative_id, None).unwrap();
        make(&db, "tk-c", &initiative_id, Some("tk-p")).unwrap();
        make(&db, "tk-o", &initiative_id, None).unwrap();
        db.with_conn(|c| add_dependency(c, "tk-o", "tk-c", "test"))
            .unwrap();

        assert!(db.with_conn(|c| delete_task(c, "tk-p", false)).is_err());
        let deleted = db.with_conn(|c| delete_task(c, "tk-p", true)).unwrap();
        assert_eq!(deleted, vec!["tk-c".to_string(), "tk-p".to_string()]);
        assert!(get(&db, "tk-o").depends_on.is_empty());
    }

    /// A file-backed database: the in-memory one uses shared-cache locking,
    /// which fails fast instead of waiting like real WAL connections do.
    fn setup_file_db() -> (Database, String) {
        let path = std::env::temp_dir().join(format!("hq-tasks-{}.db", uuid::Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        db.with_conn(|c| {
            create_initiative(
                c, "in-1", "personal", None, "Agent HQ", "agent-hq", "AGENT-HQ",
            )?;
            Ok(())
        })
        .unwrap();
        (db, "in-1".to_string())
    }

    fn events(db: &Database, id: &str) -> Vec<(String, String)> {
        db.with_conn(|c| list_task_events(c, id))
            .unwrap()
            .into_iter()
            .map(|e| (e.event_type, e.occurred_at))
            .collect()
    }

    fn assert_utc_stamp(stamp: &str) {
        chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").unwrap();
        let utc_now = chrono::Utc::now().naive_utc();
        let parsed = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").unwrap();
        assert!(
            (utc_now - parsed).num_seconds().abs() < 60,
            "{stamp} is not UTC now"
        );
    }

    #[test]
    fn new_and_legacy_tasks_have_unknown_timestamps() {
        let (db, initiative_id) = setup();
        let task = make(&db, "tk-1", &initiative_id, None).unwrap();
        assert!(task.work_started_at.is_none() && task.first_ready_for_review_at.is_none());
        assert!(events(&db, "tk-1").is_empty());

        // A task already in_progress before the feature has no event and stays unknown.
        db.with_conn(|c| {
            c.execute(
                "UPDATE tasks SET status = 'in_progress' WHERE id = 'tk-1'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        set_status(&db, "tk-1", STATUS_IN_PROGRESS);
        let task = get(&db, "tk-1");
        assert!(task.work_started_at.is_none());
        assert!(events(&db, "tk-1").is_empty());
    }

    #[test]
    fn transitions_record_utc_summaries_and_events() {
        let (db, initiative_id) = setup();
        make(&db, "tk-1", &initiative_id, None).unwrap();
        set_status(&db, "tk-1", STATUS_IN_PROGRESS);
        set_status(&db, "tk-1", STATUS_READY_FOR_REVIEW);

        let task = get(&db, "tk-1");
        assert_utc_stamp(task.work_started_at.as_deref().unwrap());
        assert_utc_stamp(task.first_ready_for_review_at.as_deref().unwrap());
        let kinds: Vec<String> = events(&db, "AGENT-HQ-001")
            .into_iter()
            .map(|e| e.0)
            .collect();
        assert_eq!(
            kinds,
            [EVENT_ENTERED_IN_PROGRESS, EVENT_ENTERED_READY_FOR_REVIEW]
        );
    }

    #[test]
    fn reopen_appends_events_and_keeps_first_timestamps() {
        let (db, initiative_id) = setup();
        make(&db, "tk-1", &initiative_id, None).unwrap();
        set_status(&db, "tk-1", STATUS_IN_PROGRESS);
        set_status(&db, "tk-1", STATUS_READY_FOR_REVIEW);
        let first = get(&db, "tk-1");
        // Age the summaries so an overwrite would be visible.
        db.with_conn(|c| {
            c.execute(
                "UPDATE tasks SET work_started_at = '2020-01-01 00:00:00', \
                 first_ready_for_review_at = '2020-01-02 00:00:00'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        set_status(&db, "tk-1", STATUS_IN_PROGRESS);
        set_status(&db, "tk-1", STATUS_READY_FOR_REVIEW);

        let task = get(&db, "tk-1");
        assert_eq!(task.work_started_at.as_deref(), Some("2020-01-01 00:00:00"));
        assert_eq!(
            task.first_ready_for_review_at.as_deref(),
            Some("2020-01-02 00:00:00")
        );
        assert!(first.work_started_at.is_some());
        assert_eq!(events(&db, "tk-1").len(), 4);
    }

    #[test]
    fn same_status_and_other_edits_record_nothing() {
        let (db, initiative_id) = setup();
        make(&db, "tk-1", &initiative_id, None).unwrap();
        set_status(&db, "tk-1", STATUS_IN_PROGRESS);
        set_status(&db, "tk-1", STATUS_IN_PROGRESS);
        set_status(&db, "tk-1", STATUS_BLOCKED);
        set_status(&db, "tk-1", STATUS_COMPLETE);
        assert_eq!(events(&db, "tk-1").len(), 1);
        assert!(get(&db, "tk-1").first_ready_for_review_at.is_none());
    }

    #[test]
    fn lost_claim_records_no_event() {
        let (db, initiative_id) = setup();
        make(&db, "tk-1", &initiative_id, None).unwrap();
        let patch = TaskPatch {
            status: Some(STATUS_IN_PROGRESS.to_string()),
            ..Default::default()
        };
        db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)))
            .unwrap();
        assert!(
            db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)))
                .is_err()
        );
        assert_eq!(events(&db, "tk-1").len(), 1);
    }

    #[test]
    fn concurrent_claims_have_one_winner_and_one_event() {
        let (db, initiative_id) = setup_file_db();
        make(&db, "tk-1", &initiative_id, None).unwrap();
        let db = std::sync::Arc::new(db);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || {
                    let patch = TaskPatch {
                        status: Some(STATUS_IN_PROGRESS.to_string()),
                        ..Default::default()
                    };
                    db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)))
                        .is_ok()
                })
            })
            .collect();
        let winners = handles
            .into_iter()
            .filter(|_| true)
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(winners, 1);
        assert_eq!(events(&db, "tk-1").len(), 1);
    }

    #[test]
    fn unguarded_concurrent_updates_record_one_transition() {
        let (db, initiative_id) = setup_file_db();
        make(&db, "tk-1", &initiative_id, None).unwrap();
        let db = std::sync::Arc::new(db);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || {
                    let patch = TaskPatch {
                        status: Some(STATUS_IN_PROGRESS.to_string()),
                        ..Default::default()
                    };
                    db.with_conn(|c| update_task(c, "tk-1", &patch, None))
                        .unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(events(&db, "tk-1").len(), 1);
    }

    #[test]
    fn delete_removes_lifecycle_events() {
        let (db, initiative_id) = setup();
        make(&db, "tk-1", &initiative_id, None).unwrap();
        set_status(&db, "tk-1", STATUS_IN_PROGRESS);
        db.with_conn(|c| delete_task(c, "tk-1", false)).unwrap();
        assert!(events(&db, "tk-1").is_empty());
    }
    fn keyed<'a>(title: &'a str, ext: &'a str) -> NewTask<'a> {
        NewTask {
            external_id: Some(ext),
            ..new_task(title, &[])
        }
    }

    #[test]
    fn an_external_id_returns_the_existing_task_in_the_same_space() {
        let (db, initiative_id) = setup();
        let (first, created) = db
            .with_conn(|c| create_task_dedup(c, "tk-1", &initiative_id, &keyed("A", "ext-1")))
            .unwrap();
        assert!(created);
        assert_eq!(first.external_id.as_deref(), Some("ext-1"));

        let (again, created) = db
            .with_conn(|c| create_task_dedup(c, "tk-2", &initiative_id, &keyed("B", " ext-1 ")))
            .unwrap();
        assert!(!created, "a retry must not create a second task");
        assert_eq!((again.id.as_str(), again.title.as_str()), ("tk-1", "A"));

        let count: i64 = db
            .with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn an_external_id_is_scoped_to_its_space() {
        let (db, initiative_id) = setup();
        let other = db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO spaces (id, name, slug) VALUES ('sp-2', 'Other', 'other')",
                    [],
                )?;
                create_initiative(c, "in-2", "sp-2", None, "Inbox", "inbox", "OTHER-INBOX")?;
                Ok("in-2".to_string())
            })
            .unwrap();
        db.with_conn(|c| create_task_dedup(c, "tk-1", &initiative_id, &keyed("A", "ext-1")))
            .unwrap();
        let (_, created) = db
            .with_conn(|c| create_task_dedup(c, "tk-2", &other, &keyed("A", "ext-1")))
            .unwrap();
        assert!(created, "the same key in another space is a different task");

        let sibling = db
            .with_conn(|c| {
                create_initiative(
                    c,
                    "in-3",
                    "personal",
                    None,
                    "Second",
                    "second",
                    "AGENT-HQ-2",
                )?;
                create_task_dedup(c, "tk-3", "in-3", &keyed("C", "ext-1"))
            })
            .unwrap();
        assert!(
            !sibling.1,
            "another initiative in the same space shares the key"
        );
    }

    #[test]
    fn the_unique_index_rejects_a_duplicate_that_skips_the_lookup() {
        let (db, initiative_id) = setup();
        db.with_conn(|c| create_task(c, "tk-1", &initiative_id, &keyed("A", "ext-1")))
            .unwrap();
        assert!(
            db.with_conn(|c| create_task(c, "tk-2", &initiative_id, &keyed("A", "ext-1")))
                .is_err()
        );
    }

    #[test]
    fn a_blank_external_id_is_no_key_and_an_oversized_one_is_refused() {
        let (db, initiative_id) = setup();
        for (n, blank) in ["", "   "].iter().enumerate() {
            let (task, created) = db
                .with_conn(|c| {
                    create_task_dedup(c, &format!("tk-{n}"), &initiative_id, &keyed("A", blank))
                })
                .unwrap();
            assert!(created && task.external_id.is_none());
        }
        let long = "x".repeat(MAX_EXTERNAL_ID_LEN + 1);
        assert!(
            db.with_conn(|c| create_task_dedup(c, "tk-9", &initiative_id, &keyed("A", &long)))
                .is_err()
        );
    }
}
