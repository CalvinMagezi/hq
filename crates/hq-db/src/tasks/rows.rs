use super::*;

pub(super) fn row_to_space(row: &rusqlite::Row) -> rusqlite::Result<Space> {
    Ok(Space {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        created_at: row.get(3)?,
    })
}

pub(super) fn row_to_folder(row: &rusqlite::Row) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get(0)?,
        space_id: row.get(1)?,
        name: row.get(2)?,
        slug: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub(super) const INITIATIVE_COLS: &str =
    "id, space_id, folder_id, name, slug, id_prefix, next_sequence, created_at";

pub(super) fn row_to_initiative(row: &rusqlite::Row) -> rusqlite::Result<Initiative> {
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

pub(super) const TASK_COLS: &str = "t.id, t.initiative_id, t.display_id, t.title, t.description, t.status, \
     t.priority, t.due_date, t.clickup_task_id, t.created_by, t.created_at, t.updated_at, \
     t.parent_task_id, t.start_date, t.work_started_at, t.first_ready_for_review_at, t.external_id";

pub(super) fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<Task> {
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

pub(super) fn row_to_comment(row: &rusqlite::Row) -> rusqlite::Result<TaskComment> {
    Ok(TaskComment {
        id: row.get(0)?,
        task_id: row.get(1)?,
        author: row.get(2)?,
        body: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub(super) fn event_for_status(status: &str) -> Option<(&'static str, &'static str)> {
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
pub(super) fn record_transition(conn: &Connection, task_id: &str, status: &str) -> Result<()> {
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

pub(super) fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

/// Fills the join-derived fields (tags, dependencies, sub-task counts) for a
/// batch of tasks with one query each, instead of one query per task.
pub(super) fn hydrate(conn: &Connection, tasks: &mut [Task]) -> Result<()> {
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

pub(super) fn hydrate_one(conn: &Connection, task: Option<Task>) -> Result<Option<Task>> {
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

pub(super) fn validate_schedule(start_date: Option<&str>, due_date: Option<&str>) -> Result<()> {
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
pub(super) fn resolve_parent(conn: &Connection, parent: &str, child_initiative_id: &str) -> Result<Task> {
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

pub(super) fn subtask_ids(conn: &Connection, task_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM tasks WHERE parent_task_id = ?1")?;
    let ids = stmt
        .query_map(params![task_id], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

pub(super) fn set_tags(conn: &Connection, task_id: &str, tags: &[String]) -> Result<()> {
    conn.execute("DELETE FROM task_tags WHERE task_id = ?1", params![task_id])?;
    for tag in tags {
        conn.execute(
            "INSERT OR IGNORE INTO task_tags (task_id, tag) VALUES (?1, ?2)",
            params![task_id, tag],
        )?;
    }
    Ok(())
}
