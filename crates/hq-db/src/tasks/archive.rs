//! Removing a task. Deleting archives it: hidden from lists, kept with its comments,
//! events, leases and links, and restorable. Only an archived task can be purged, and
//! `task_audit` records each step and outlives the purge, so a removal is never silent.

use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: i64,
    pub task_id: String,
    pub display_id: String,
    pub title: String,
    pub action: String,
    pub actor: String,
    pub detail: String,
    pub at: String,
}

fn audit(conn: &Connection, task: &Task, action: &str, actor: &str, detail: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO task_audit (task_id, display_id, title, action, actor, detail) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![task.id, task.display_id, task.title, action, clean_actor(actor), detail],
    )?;
    Ok(())
}

fn clean_actor(actor: &str) -> String {
    let cleaned: String = actor.chars().filter(|c| !c.is_control()).take(120).collect();
    if cleaned.trim().is_empty() { "unknown".to_string() } else { cleaned.trim().to_string() }
}

/// Hides a task, and with `cascade` its sub-tasks, from lists. Everything it holds is
/// kept. A task with sub-tasks needs `cascade`. Its idempotency key is freed, so a
/// retried create after a deliberate delete makes a new task, and live leases end.
/// Returns the ids archived.
pub fn archive_task(conn: &Connection, id: &str, cascade: bool, actor: &str) -> Result<Vec<String>> {
    in_write_tx(conn, |conn| {
        let task = get_task(conn, id)?.ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;
        if task.archived_at.is_some() {
            anyhow::bail!("{} is already archived", task.display_id);
        }
        let children = subtask_ids(conn, &task.id)?;
        if !children.is_empty() && !cascade {
            anyhow::bail!(
                "{} has {} sub-task(s); archive them first or pass cascade",
                task.display_id,
                children.len()
            );
        }
        let now: String = conn.query_row("SELECT datetime('now')", [], |r| r.get(0))?;
        let mut archived = children;
        archived.push(task.id.clone());
        for task_id in &archived {
            let member = get_task(conn, task_id)?.ok_or_else(|| anyhow::anyhow!("task {task_id} vanished"))?;
            let detail = member.external_id.as_deref().map(|e| format!("external_id {e}")).unwrap_or_default();
            conn.execute(
                "UPDATE tasks SET archived_at = ?1, updated_at = ?1, external_id = NULL, external_space_id = NULL \
                 WHERE id = ?2",
                params![now, task_id],
            )?;
            conn.execute(
                "UPDATE task_work_sessions SET ended_at = ?1, end_reason = ?2 WHERE task_id = ?3 AND ended_at IS NULL",
                params![now, END_RELEASED, task_id],
            )?;
            audit(conn, &member, "archived", actor, &detail)?;
        }
        changed_outside_tx(conn);
        Ok(archived)
    })
}

/// Brings an archived task back, and the sub-tasks archived with it. A sub-task cannot
/// come back before its parent.
pub fn restore_task(conn: &Connection, id: &str, actor: &str) -> Result<Task> {
    in_write_tx(conn, |conn| {
        let task = get_task(conn, id)?.ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;
        let Some(stamp) = task.archived_at.clone() else {
            anyhow::bail!("{} is not archived", task.display_id);
        };
        if let Some(parent) = task.parent_task_id.as_deref().map(|p| get_task(conn, p)).transpose()?.flatten()
            && parent.archived_at.is_some()
        {
            anyhow::bail!("{} is a sub-task of the archived {}; restore the parent first", task.display_id, parent.display_id);
        }
        let mut stmt = conn.prepare("SELECT id FROM tasks WHERE parent_task_id = ?1 AND archived_at = ?2")?;
        let together = stmt
            .query_map(params![task.id, stamp], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for task_id in std::iter::once(&task.id).chain(together.iter()) {
            conn.execute(
                "UPDATE tasks SET archived_at = NULL, updated_at = datetime('now') WHERE id = ?1",
                params![task_id],
            )?;
            if let Some(member) = get_task(conn, task_id)? {
                audit(conn, &member, "restored", actor, "")?;
            }
        }
        changed_outside_tx(conn);
        get_task(conn, &task.id)?.ok_or_else(|| anyhow::anyhow!("task {id} vanished after restore"))
    })
}

/// Permanently removes an archived task and its archived sub-tasks, with everything they
/// hold. Refuses a task that is not archived, and one with a sub-task that still is active.
/// Returns the ids removed.
pub fn purge_task(conn: &Connection, id: &str, actor: &str) -> Result<Vec<String>> {
    in_write_tx(conn, |conn| {
        let task = get_task(conn, id)?.ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;
        if task.archived_at.is_none() {
            anyhow::bail!("{} is not archived; archive it first, then purge it", task.display_id);
        }
        let mut stmt = conn.prepare("SELECT id, archived_at IS NULL FROM tasks WHERE parent_task_id = ?1")?;
        let children = stmt
            .query_map(params![task.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if children.iter().any(|(_, active)| *active) {
            anyhow::bail!("{} has active sub-tasks; archive or restore them first", task.display_id);
        }
        let mut removed: Vec<String> = children.into_iter().map(|(child, _)| child).collect();
        removed.push(task.id.clone());
        for task_id in &removed {
            if let Some(member) = get_task(conn, task_id)? {
                audit(conn, &member, "purged", actor, "")?;
            }
            for table in ["task_comments", "task_tags", "task_events", "task_work_sessions", "task_checkpoints"] {
                conn.execute(&format!("DELETE FROM {table} WHERE task_id = ?1"), params![task_id])?;
            }
            conn.execute(
                "DELETE FROM task_dependencies WHERE task_id = ?1 OR depends_on_task_id = ?1",
                params![task_id],
            )?;
            conn.execute(
                "DELETE FROM task_links WHERE task_id = ?1 OR (kind = 'task' AND ref = ?1)",
                params![task_id],
            )?;
            conn.execute("DELETE FROM tasks WHERE id = ?1", params![task_id])?;
        }
        changed_outside_tx(conn);
        Ok(removed)
    })
}

/// The most recent removals and restores, newest first, for one task or for all.
pub fn list_task_audit(conn: &Connection, task_ref: Option<&str>, limit: usize) -> Result<Vec<AuditEntry>> {
    let task_id = match task_ref {
        Some(r) => Some(get_task(conn, r)?.map(|t| t.id).unwrap_or_else(|| r.to_string())),
        None => None,
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT id, task_id, display_id, title, action, actor, detail, at FROM task_audit \
         WHERE (?1 IS NULL OR task_id = ?1) ORDER BY id DESC LIMIT {limit}"
    ))?;
    let rows = stmt
        .query_map(params![task_id], |r| {
            Ok(AuditEntry {
                id: r.get(0)?,
                task_id: r.get(1)?,
                display_id: r.get(2)?,
                title: r.get(3)?,
                action: r.get(4)?,
                actor: r.get(5)?,
                detail: r.get(6)?,
                at: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
