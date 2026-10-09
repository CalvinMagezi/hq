//! Picking the next thing to do. An agent asks for its next task and gets one it can start
//! now, claimed in the same step, so two agents asking at once never take the same one.

use super::*;

/// Who is asking, and what they will accept.
#[derive(Debug, Clone, Copy)]
pub struct NextTaskQuery<'a> {
    /// The name tasks are assigned to.
    pub assignee: &'a str,
    /// Also consider tasks nobody is assigned to.
    pub include_unassigned: bool,
    pub initiative_id: Option<&'a str>,
    pub tag: Option<&'a str>,
}

/// The best task to start, or `None`. Open (`to_do`), not archived, with every dependency
/// done, nobody holding it, and assigned to the asker (or to no one, when allowed). Most
/// urgent first, then soonest due, then oldest.
pub fn next_task(conn: &Connection, q: &NextTaskQuery) -> Result<Option<Task>> {
    let sql = format!(
        "SELECT {TASK_COLS} FROM tasks t \
         WHERE t.status = 'to_do' AND t.archived_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM task_dependencies d JOIN tasks b ON b.id = d.depends_on_task_id \
                           WHERE d.task_id = t.id AND b.status <> 'complete' AND b.archived_at IS NULL) \
           AND NOT EXISTS (SELECT 1 FROM task_work_sessions s WHERE s.task_id = t.id AND s.ended_at IS NULL) \
           AND (t.start_date IS NULL OR t.start_date <= date('now')) \
           AND (EXISTS (SELECT 1 FROM task_assignees a WHERE a.task_id = t.id AND a.assignee = ?1) \
                OR (?2 AND NOT EXISTS (SELECT 1 FROM task_assignees a WHERE a.task_id = t.id))) \
           AND (?3 IS NULL OR t.initiative_id = ?3) \
           AND (?4 IS NULL OR EXISTS (SELECT 1 FROM task_tags g WHERE g.task_id = t.id AND g.tag = ?4)) \
         ORDER BY {PRIORITY_RANK_SQL}, t.due_date IS NULL, t.due_date, t.created_at, t.id LIMIT 1"
    );
    let assignee = super::leases::clean_label(q.assignee);
    let task = conn
        .query_row(
            &sql,
            params![assignee, q.include_unassigned, q.initiative_id, q.tag],
            row_to_task,
        )
        .optional()?;
    hydrate_one(conn, task)
}

/// Picks the next task and claims it, in one write, or `None` when there is nothing to start.
pub fn claim_next(
    conn: &Connection,
    q: &NextTaskQuery,
    who: &LeaseIdentity,
    ttl_secs: i64,
) -> Result<Option<Claimed>> {
    in_write_tx(conn, |conn| match next_task(conn, q)? {
        Some(task) => claim(conn, &task.id, who, ttl_secs, false).map(Some),
        None => Ok(None),
    })
}
