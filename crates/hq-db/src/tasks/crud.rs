use super::*;

pub fn create_task(
    conn: &Connection,
    id: &str,
    initiative_id: &str,
    new: &NewTask,
) -> Result<Task> {
    validate_schedule(new.start_date, new.due_date)?;
    if let Some(priority) = new.priority {
        validate_priority(priority)?;
    }
    if let Some(minutes) = new.estimate_minutes {
        validate_estimate(minutes)?;
    }
    let parent_id = match new.parent_task_id {
        Some(parent) => Some(resolve_parent(conn, parent, initiative_id)?.id),
        None => None,
    };
    let external = external_key(conn, initiative_id, new.external_id)?;
    let display_id = next_display_id(conn, initiative_id)?;
    conn.execute(
        "INSERT INTO tasks (id, initiative_id, display_id, title, description, priority, due_date, \
         created_by, parent_task_id, start_date, external_id, external_space_id, estimate_minutes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
            external.as_ref().map(|(space, _)| space),
            new.estimate_minutes
        ],
    )?;
    set_tags(conn, id, new.tags)?;
    notify_on_ok(conn, 
        get_task(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("task {id} vanished immediately after creation")),
    )
}

/// Longest accepted `external_id`.
pub const MAX_EXTERNAL_ID_LEN: usize = 200;

/// The (space id, trimmed external id) a new task is keyed under. A blank id
/// means no key; one over the length cap is an error rather than truncated, so
/// two long ids can never collide silently.
pub(super) fn external_key(
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

type SqlParams = Vec<Box<dyn rusqlite::types::ToSql>>;

/// The `FROM ... WHERE ...` text and its parameters for a filter, shared by the
/// page query and the count so the two can never disagree.
fn filter_sql(filter: &TaskFilter) -> (String, SqlParams) {
    let mut sql = String::from("FROM tasks t JOIN initiatives i ON i.id = t.initiative_id");
    let mut conditions: Vec<&'static str> = Vec::new();
    let mut vals: SqlParams = Vec::new();

    if let Some(tag) = &filter.tag {
        sql.push_str(" JOIN task_tags tg ON tg.task_id = t.id");
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
    (sql, vals)
}

/// One page of tasks, newest activity first. Page with `limit` and `offset`
/// and read `count_tasks` for the total, so a list longer than a page is never
/// silently cut.
pub fn list_tasks(conn: &Connection, filter: &TaskFilter) -> Result<Vec<Task>> {
    let (from_where, vals) = filter_sql(filter);
    let limit = filter.limit.unwrap_or(MAX_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
    let sql = format!(
        "SELECT {TASK_COLS} {from_where} \
         ORDER BY t.updated_at DESC, t.created_at DESC, t.display_id DESC, t.id DESC \
         LIMIT {limit} OFFSET {}",
        filter.offset.min(i64::MAX as usize)
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    let mut tasks = stmt
        .query_map(param_refs.as_slice(), row_to_task)?
        .collect::<rusqlite::Result<Vec<Task>>>()?;
    hydrate(conn, &mut tasks)?;
    Ok(tasks)
}

/// How many tasks match the filter, ignoring its `limit` and `offset`.
pub fn count_tasks(conn: &Connection, filter: &TaskFilter) -> Result<i64> {
    let (from_where, vals) = filter_sql(filter);
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = vals.iter().map(|b| b.as_ref()).collect();
    Ok(conn.query_row(
        &format!("SELECT COUNT(*) {from_where}"),
        param_refs.as_slice(),
        |r| r.get(0),
    )?)
}

/// Checks the parent and schedule a patch would produce against the task's
/// current state, before anything is written.
pub(super) fn validate_patch(conn: &Connection, current: &Task, patch: &TaskPatch) -> Result<()> {
    if let Some(status) = &patch.status {
        validate_status(status)?;
    }
    if let Some(Some(priority)) = &patch.priority {
        validate_priority(priority)?;
    }
    if let Some(Some(minutes)) = patch.estimate_minutes {
        validate_estimate(minutes)?;
    }
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
    update_task_as(conn, id, patch, expected_status, &WriteCtx::default())
}

/// `update_task`, recording `ctx` as the actor of any status event it writes.
pub fn update_task_as(
    conn: &Connection,
    id: &str,
    patch: &TaskPatch,
    expected_status: Option<&str>,
    ctx: &WriteCtx,
) -> Result<Task> {
    // The write lock is taken before the status read, so "did the status
    // change" is decided on the same state the UPDATE then modifies.
    in_write_tx(conn, |conn| apply_update(conn, id, patch, expected_status, ctx))
}

fn apply_update(
    conn: &Connection,
    id: &str,
    patch: &TaskPatch,
    expected_status: Option<&str>,
    ctx: &WriteCtx,
) -> Result<Task> {
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
    if let Some(minutes) = &patch.estimate_minutes {
        sets.push("estimate_minutes = ?");
        vals.push(Box::new(*minutes));
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
        record_transition(conn, &current.id, &current.status, status, ctx)?;
    }

    get_task(conn, &current.id)?.ok_or_else(|| anyhow::anyhow!("task {id} vanished after update"))
}

/// `PRAGMA foreign_keys=ON` (see `pool.rs`) means child rows must go first —
/// SQLite rejects the parent delete otherwise instead of silently orphaning.
/// A task with sub-tasks is only deleted when `cascade` is set, taking its
/// sub-tasks with it. Returns the internal ids of every deleted task.
pub fn delete_task(conn: &Connection, id: &str, cascade: bool) -> Result<Vec<String>> {
    in_write_tx(conn, |conn| {
        let task = get_task(conn, id)?.ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;
        let children = subtask_ids(conn, &task.id)?;
        if !children.is_empty() && !cascade {
            anyhow::bail!(
                "{} has {} sub-task(s); delete them first or pass cascade",
                task.display_id,
                children.len()
            );
        }

        let mut deleted = children;
        deleted.push(task.id);
        for task_id in &deleted {
            for table in ["task_comments", "task_tags", "task_events", "task_work_sessions"] {
                conn.execute(&format!("DELETE FROM {table} WHERE task_id = ?1"), params![task_id])?;
            }
            conn.execute(
                "DELETE FROM task_dependencies WHERE task_id = ?1 OR depends_on_task_id = ?1",
                params![task_id],
            )?;
            conn.execute("DELETE FROM tasks WHERE id = ?1", params![task_id])?;
        }
        Ok(deleted)
    })
}
