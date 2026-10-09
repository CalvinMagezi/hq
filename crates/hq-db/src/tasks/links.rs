use super::*;

pub(super) fn resolve_pair(conn: &Connection, task: &str, depends_on: &str) -> Result<(Task, Task)> {
    let dependent =
        get_task(conn, task)?.ok_or_else(|| anyhow::anyhow!("task {task} not found"))?;
    let blocker = get_task(conn, depends_on)?
        .ok_or_else(|| anyhow::anyhow!("dependency {depends_on} not found"))?;
    Ok((dependent, blocker))
}

pub(super) fn touch(conn: &Connection, task_id: &str) -> Result<()> {
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
    if let Some(archived) = [&dependent, &blocker].into_iter().find(|t| t.archived_at.is_some()) {
        anyhow::bail!("{} is archived; restore it before linking it to other tasks", archived.display_id);
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
    notify_on_ok(conn, touch(conn, &dependent.id))
}

pub fn remove_dependency(conn: &Connection, task: &str, depends_on: &str) -> Result<()> {
    let (dependent, blocker) = resolve_pair(conn, task, depends_on)?;
    conn.execute(
        "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on_task_id = ?2",
        params![dependent.id, blocker.id],
    )?;
    notify_on_ok(conn, touch(conn, &dependent.id))
}

pub(super) fn tasks_by_ids(conn: &Connection, sql: &str, id: &str) -> Result<Vec<Task>> {
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
        "SELECT id FROM tasks WHERE parent_task_id = ?1 AND archived_at IS NULL ORDER BY display_id",
        parent_id,
    )
}

/// Open dependents of `completed_task_id` whose every dependency is now
/// complete: the tasks this completion just unblocked.
pub fn newly_unblocked(conn: &Connection, completed_task_id: &str) -> Result<Vec<Task>> {
    tasks_by_ids(
        conn,
        "SELECT d.task_id FROM task_dependencies d JOIN tasks dep ON dep.id = d.task_id
         WHERE d.depends_on_task_id = ?1 AND dep.status <> 'complete' AND dep.archived_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM task_dependencies d2 JOIN tasks b ON b.id = d2.depends_on_task_id
               WHERE d2.task_id = d.task_id AND b.status <> 'complete' AND b.archived_at IS NULL
           )",
        completed_task_id,
    )
}


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
    changed_outside_tx(conn);
    Ok(conn.query_row(
        "SELECT id, task_id, author, body, created_at, kind, sender_session_id, to_session_id, reply_to, delivered_at FROM task_comments WHERE id = ?1",
        params![id],
        row_to_comment,
    )?)
}

pub fn list_comments(conn: &Connection, task_id: &str) -> Result<Vec<TaskComment>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, author, body, created_at, kind, sender_session_id, to_session_id, reply_to, delivered_at FROM task_comments \
         WHERE task_id = ?1 ORDER BY created_at, id",
    )?;
    let rows = stmt
        .query_map(params![task_id], row_to_comment)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}
