//! Shelving: the owner's way to tidy the task views. A shelved task or list is hidden
//! only from reads that ask for `Shelf::Exclude` (the web's everyday lists). Nothing else
//! reads `shelved_at`: the queue, the graph, dependencies, leases, links, search and every
//! agent tool see a shelved task exactly as they see any other, so it stays valid.
//!
//! This is not `archive_task`, which removes a task. Shelving never changes `updated_at`,
//! writes no lifecycle event and frees no key, so polling agents and sort order are
//! untouched, and `unshelve_tasks` restores the earlier view exactly.

use super::*;

/// Join that lets a list see whether a sub-task's parent is shelved.
pub(super) const SHELF_PARENT_JOIN: &str = " LEFT JOIN tasks sp ON sp.id = t.parent_task_id";

/// Most tasks one shelve or unshelve call takes.
pub const MAX_SHELVE_BATCH: usize = 500;

/// Largest `older_than_days` accepted, ten years.
pub const MAX_SHELVE_AGE_DAYS: u32 = 3650;

/// The WHERE term for a task list, over `t` (the task), `i` (its list) and `sp` (its
/// parent). A task is hidden when it, its list or its parent is shelved, so shelving a
/// list is one row and unshelving it brings back exactly what was there.
pub(super) fn shelf_condition(shelf: Shelf) -> Option<&'static str> {
    match shelf {
        Shelf::Include => None,
        Shelf::Exclude => {
            Some("t.shelved_at IS NULL AND i.shelved_at IS NULL AND sp.shelved_at IS NULL")
        }
        Shelf::Only => Some(
            "(t.shelved_at IS NOT NULL OR i.shelved_at IS NOT NULL OR sp.shelved_at IS NOT NULL)",
        ),
    }
}

/// A task left as it was, and why.
#[derive(Debug, Clone, Serialize)]
pub struct Skipped {
    pub task: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ShelveOutcome {
    /// Ids of the tasks whose shelf state changed.
    pub changed: Vec<String>,
    pub skipped: Vec<Skipped>,
}

/// Which tasks a sweep may touch.
#[derive(Debug, Clone, Default)]
pub struct SweepScope {
    pub space_id: Option<String>,
    pub initiative_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepOutcome {
    pub matched: usize,
    /// Ids shelved, empty on a dry run. Pass them to `unshelve_tasks` to undo.
    pub changed: Vec<String>,
    pub dry_run: bool,
}

fn now(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("SELECT datetime('now')", [], |r| r.get(0))?)
}

fn holder_of(conn: &Connection, task_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT actor FROM task_work_sessions WHERE task_id = ?1 AND ended_at IS NULL \
             ORDER BY started_at LIMIT 1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Whoever holds a lease on one of the task's sub-tasks: hiding the parent hides them too.
fn subtask_holder_of(conn: &Connection, task_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT w.actor FROM task_work_sessions w JOIN tasks c ON c.id = w.task_id \
             WHERE c.parent_task_id = ?1 AND w.ended_at IS NULL ORDER BY w.started_at LIMIT 1",
            params![task_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Whoever holds a lease on a task in the list.
fn list_holder_of(conn: &Connection, initiative_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT w.actor FROM task_work_sessions w JOIN tasks t ON t.id = w.task_id \
             WHERE t.initiative_id = ?1 AND w.ended_at IS NULL ORDER BY w.started_at LIMIT 1",
            params![initiative_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Why a task that carries no stamp of its own is still hidden, if it is.
fn hidden_by(conn: &Connection, task: &Task) -> Result<Option<String>> {
    let list_shelved: bool = conn.query_row(
        "SELECT shelved_at IS NOT NULL FROM initiatives WHERE id = ?1",
        params![task.initiative_id],
        |r| r.get(0),
    )?;
    if list_shelved {
        return Ok(Some(
            "hidden by its archived list; unarchive the list".to_string(),
        ));
    }
    let parent = task
        .parent_task_id
        .as_deref()
        .map(|p| get_task(conn, p))
        .transpose()?
        .flatten()
        .filter(|p| p.shelved_at.is_some());
    Ok(parent.map(|p| {
        format!(
            "hidden by its archived parent {}; unarchive the parent",
            p.display_id
        )
    }))
}

fn check_batch(refs: &[String]) -> Result<()> {
    if refs.is_empty() || refs.len() > MAX_SHELVE_BATCH {
        anyhow::bail!(
            "give between 1 and {MAX_SHELVE_BATCH} tasks, got {}",
            refs.len()
        );
    }
    Ok(())
}

fn skip(task: &str, reason: impl Into<String>) -> Skipped {
    Skipped {
        task: task.to_string(),
        reason: reason.into(),
    }
}

/// Hides tasks from the everyday lists. A removed task, an already shelved one and one an
/// agent holds a lease on are left as they are and reported in `skipped`.
pub fn shelve_tasks(conn: &Connection, refs: &[String]) -> Result<ShelveOutcome> {
    check_batch(refs)?;
    in_write_tx(conn, |conn| {
        let stamp = now(conn)?;
        let mut out = ShelveOutcome::default();
        for task_ref in refs {
            let Some(task) = get_task(conn, task_ref)? else {
                out.skipped.push(skip(task_ref, "not found"));
                continue;
            };
            if task.archived_at.is_some() {
                out.skipped
                    .push(skip(task_ref, "deleted; restore it first"));
            } else if task.shelved_at.is_some() {
                out.skipped.push(skip(task_ref, "already archived"));
            } else if let Some(holder) = holder_of(conn, &task.id)? {
                out.skipped
                    .push(skip(task_ref, format!("{holder} is working on it")));
            } else if let Some(holder) = subtask_holder_of(conn, &task.id)? {
                out.skipped.push(skip(
                    task_ref,
                    format!("{holder} is working on one of its sub-tasks"),
                ));
            } else {
                conn.execute(
                    "UPDATE tasks SET shelved_at = ?1 WHERE id = ?2",
                    params![stamp, task.id],
                )?;
                out.changed.push(task.id);
            }
        }
        changed_outside_tx(conn);
        Ok(out)
    })
}

/// Puts shelved tasks back in the everyday lists.
pub fn unshelve_tasks(conn: &Connection, refs: &[String]) -> Result<ShelveOutcome> {
    check_batch(refs)?;
    in_write_tx(conn, |conn| {
        let mut out = ShelveOutcome::default();
        for task_ref in refs {
            let Some(task) = get_task(conn, task_ref)? else {
                out.skipped.push(skip(task_ref, "not found"));
                continue;
            };
            if task.shelved_at.is_none() {
                let reason = hidden_by(conn, &task)?.unwrap_or_else(|| "not archived".to_string());
                out.skipped.push(skip(task_ref, reason));
                continue;
            }
            conn.execute(
                "UPDATE tasks SET shelved_at = NULL WHERE id = ?1",
                params![task.id],
            )?;
            out.changed.push(task.id);
        }
        changed_outside_tx(conn);
        Ok(out)
    })
}

/// Shelves or unshelves a whole list. Its tasks keep their own state, so unshelving
/// restores them as they were. Returns the list and how many of its tasks are still open.
pub fn set_initiative_shelved(
    conn: &Connection,
    id: &str,
    shelved: bool,
) -> Result<(Initiative, i64)> {
    in_write_tx(conn, |conn| {
        if shelved && let Some(holder) = list_holder_of(conn, id)? {
            anyhow::bail!(
                "{holder} is working on a task in this list; archive it when they are done"
            );
        }
        let affected = conn.execute(
            "UPDATE initiatives SET shelved_at = CASE WHEN ?1 THEN COALESCE(shelved_at, datetime('now')) END \
             WHERE id = ?2",
            params![shelved, id],
        )?;
        if affected == 0 {
            anyhow::bail!("list {id} not found");
        }
        let open: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE initiative_id = ?1 AND archived_at IS NULL \
             AND status <> 'complete'",
            params![id],
            |r| r.get(0),
        )?;
        changed_outside_tx(conn);
        let initiative = get_initiative(conn, id)?
            .ok_or_else(|| anyhow::anyhow!("list {id} vanished after update"))?;
        Ok((initiative, open))
    })
}

/// A completed task is swept when it finished before the cutoff, has no open sub-task and
/// no agent holds it. The cutoff reads `completed_at`, falling back to `updated_at` for
/// tasks completed before that column existed.
const SWEEP_WHERE: &str = "t.status = 'complete' AND t.archived_at IS NULL AND t.shelved_at IS NULL \
     AND COALESCE(t.completed_at, t.updated_at) < datetime('now', ?1) \
     AND NOT EXISTS (SELECT 1 FROM tasks c WHERE c.parent_task_id = t.id \
                     AND c.archived_at IS NULL AND c.status <> 'complete') \
     AND NOT EXISTS (SELECT 1 FROM task_work_sessions w WHERE w.task_id = t.id AND w.ended_at IS NULL) \
     AND (?2 IS NULL OR t.initiative_id IN (SELECT id FROM initiatives WHERE space_id = ?2)) \
     AND (?3 IS NULL OR t.initiative_id = ?3)";

/// Shelves every completed task older than `older_than_days`, in one statement. With
/// `dry_run` it only counts. Open work is never swept, so the sweep cannot hide a task
/// anyone still needs to see.
pub fn shelve_completed(
    conn: &Connection,
    older_than_days: u32,
    scope: &SweepScope,
    dry_run: bool,
) -> Result<SweepOutcome> {
    if !(1..=MAX_SHELVE_AGE_DAYS).contains(&older_than_days) {
        anyhow::bail!("older_than_days must be between 1 and {MAX_SHELVE_AGE_DAYS}");
    }
    let age = format!("-{older_than_days} days");
    in_write_tx(conn, |conn| {
        let args = params![age, scope.space_id, scope.initiative_id];
        if dry_run {
            let matched: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM tasks t WHERE {SWEEP_WHERE}"),
                args,
                |r| r.get(0),
            )?;
            return Ok(SweepOutcome {
                matched: usize::try_from(matched).unwrap_or(0),
                changed: Vec::new(),
                dry_run,
            });
        }
        let mut stmt = conn.prepare(&format!(
            "UPDATE tasks AS t SET shelved_at = datetime('now') WHERE {SWEEP_WHERE} RETURNING id"
        ))?;
        let changed = stmt
            .query_map(args, |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        changed_outside_tx(conn);
        Ok(SweepOutcome {
            matched: changed.len(),
            changed,
            dry_run,
        })
    })
}

/// Work resuming on a task brings it back into view, along with its parent and its list:
/// reopening, claiming or adding a task to a shelved list must never leave live work
/// hidden from the owner.
pub(super) fn surface(conn: &Connection, task_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE tasks SET shelved_at = NULL WHERE shelved_at IS NOT NULL \
         AND (id = ?1 OR id = (SELECT parent_task_id FROM tasks WHERE id = ?1))",
        params![task_id],
    )?;
    conn.execute(
        "UPDATE initiatives SET shelved_at = NULL WHERE shelved_at IS NOT NULL \
         AND id = (SELECT initiative_id FROM tasks WHERE id = ?1)",
        params![task_id],
    )?;
    Ok(())
}
