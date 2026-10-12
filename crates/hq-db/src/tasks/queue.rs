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
    /// Leave out code-shaped tasks: tagged `code`, or linked to a commit or a
    /// pull request. What a general-purpose agent is never offered.
    pub no_code_work: bool,
}

/// The tag that marks a task as code work for `no_code_work`.
pub const CODE_TAG: &str = "code";

/// The best task to start, or `None`. Open (`to_do`), not archived, with every dependency
/// done, nobody holding it, and assigned to the asker (or to no one, when allowed). Most
/// urgent first, then soonest due, then oldest.
pub fn next_task(conn: &Connection, q: &NextTaskQuery) -> Result<Option<Task>> {
    Ok(next_tasks(conn, q, 1)?.into_iter().next())
}

/// The first `limit` tasks `next_task` would pick, best first, for a caller that
/// may have to pass over some of them (the stack skips a task whose last launch
/// failed). The order is `next_task`'s.
pub fn next_tasks(conn: &Connection, q: &NextTaskQuery, limit: usize) -> Result<Vec<Task>> {
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
           AND (NOT ?5 OR NOT {code_work}) \
         ORDER BY {PRIORITY_RANK_SQL}, t.due_date IS NULL, t.due_date, t.created_at, t.id LIMIT ?6",
        code_work = code_work_sql("t")
    );
    let assignee = super::leases::clean_label(q.assignee);
    let mut stmt = conn.prepare(&sql)?;
    let mut tasks: Vec<Task> = stmt
        .query_map(
            params![
                assignee,
                q.include_unassigned,
                q.initiative_id,
                q.tag,
                q.no_code_work,
                limit.max(1) as i64
            ],
            row_to_task,
        )?
        .collect::<std::result::Result<_, _>>()?;
    super::rows::hydrate(conn, &mut tasks)?;
    Ok(tasks)
}

/// SQL that is true when the task row aliased `alias` is code work: tagged
/// `code`, or linked to a commit or a pull request. The one definition the
/// queue (`no_code_work`) and `is_code_work` share.
fn code_work_sql(alias: &str) -> String {
    format!(
        "(EXISTS (SELECT 1 FROM task_tags g WHERE g.task_id = {alias}.id AND g.tag = '{CODE_TAG}') \
          OR EXISTS (SELECT 1 FROM task_links l WHERE l.task_id = {alias}.id AND l.kind IN ('commit', 'pr')))"
    )
}

/// Whether a task is code work by the queue's own rule, for a caller that holds
/// a task instead of asking for the next one (`hq stack run`).
pub fn is_code_work(conn: &Connection, task_id: &str) -> Result<bool> {
    let sql = format!("SELECT {} FROM tasks t WHERE t.id = ?1", code_work_sql("t"));
    Ok(conn
        .query_row(&sql, [task_id], |r| r.get::<_, bool>(0))
        .optional()?
        .unwrap_or(false))
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
