//! Sweeps: a persisted batch over the tasks waiting for review, so a large backlog is one plan the
//! owner reads and one apply they confirm instead of a review of each task
//! (docs/plans/task-verification.md, "Clearing a queue on demand").
//!
//! This module is the data only. The work that fills it (reading GitHub, asking a reviewer,
//! moving tasks) is in `hq-tools`, and runs from the daemon a few items per tick so a sweep
//! survives a restart.

use super::verification::review_marker;
use super::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const SWEEP_PLANNING: &str = "planning";
pub const SWEEP_PLANNED: &str = "planned";
pub const SWEEP_RUNNING: &str = "running";
pub const SWEEP_PAUSED: &str = "paused";
pub const SWEEP_CANCELLED: &str = "cancelled";
pub const SWEEP_READY: &str = "ready";
pub const SWEEP_APPLYING: &str = "applying";
pub const SWEEP_APPLIED: &str = "applied";
pub const SWEEP_UNDONE: &str = "undone";

pub const BUCKET_UNPLANNED: &str = "unplanned";
pub const BUCKET_NEEDS_REVIEW: &str = "needs_review";
pub const BUCKET_PASSED: &str = "passed";
pub const BUCKET_FAILED: &str = "failed";
pub const BUCKET_NO_ARTIFACT: &str = "no_artifact";
pub const BUCKET_UNDECIDABLE: &str = "undecidable";
pub const BUCKET_NEEDS_YOU: &str = "needs_you";

pub const ITEM_UNPLANNED: &str = "unplanned";
pub const ITEM_PLANNED: &str = "planned";
pub const ITEM_VERDICT: &str = "verdict";
pub const ITEM_APPLIED: &str = "applied";
pub const ITEM_KEPT: &str = "kept";
pub const ITEM_STALE: &str = "stale";
pub const ITEM_UNDONE: &str = "undone";
pub const ITEM_LEFT_ALONE: &str = "left_alone";

/// Most tasks one sweep takes, so a sweep ends in bounded time and a plan stays readable.
pub const MAX_SWEEP_TASKS: usize = 1000;

/// Which tasks waiting for review a sweep covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SweepScope {
    /// Everything waiting for review.
    All,
    /// One initiative (id).
    Initiative { id: String },
    /// Tasks that entered review on or after a date (`YYYY-MM-DD`).
    Since { date: String },
    /// A hand-picked set (ids or display ids).
    Tasks { ids: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sweep {
    pub id: String,
    pub scope: Value,
    pub status: String,
    pub plan: Value,
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
    pub applied_at: Option<String>,
    pub undone_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SweepItem {
    pub sweep_id: String,
    pub task_id: String,
    pub display_id: String,
    pub title: String,
    pub rank: i64,
    pub submission: Option<i64>,
    pub status_before: String,
    pub tier: u8,
    pub bucket: String,
    pub state: String,
    pub verdict_id: Option<String>,
    pub finding: String,
    pub currency: Value,
    pub tries: i64,
    pub retry_at: i64,
    pub applied_status: Option<String>,
    pub applied_updated_at: Option<String>,
    pub detail: String,
}

const SWEEP_COLS: &str =
    "id, scope, status, plan, note, created_at, updated_at, applied_at, undone_at";

fn row_to_sweep(r: &rusqlite::Row) -> rusqlite::Result<Sweep> {
    let scope: String = r.get(1)?;
    let plan: String = r.get(3)?;
    Ok(Sweep {
        id: r.get(0)?,
        scope: serde_json::from_str(&scope).unwrap_or(Value::Null),
        status: r.get(2)?,
        plan: serde_json::from_str(&plan).unwrap_or(Value::Null),
        note: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        applied_at: r.get(7)?,
        undone_at: r.get(8)?,
    })
}

const ITEM_COLS: &str = "i.sweep_id, i.task_id, t.display_id, t.title, i.rank, i.submission, \
     i.status_before, i.tier, i.bucket, i.state, i.verdict_id, i.finding, i.currency, i.tries, \
     i.retry_at, i.applied_status, i.applied_updated_at, i.detail";

fn row_to_item(r: &rusqlite::Row) -> rusqlite::Result<SweepItem> {
    let currency: String = r.get(12)?;
    Ok(SweepItem {
        sweep_id: r.get(0)?,
        task_id: r.get(1)?,
        display_id: r.get(2)?,
        title: r.get(3)?,
        rank: r.get(4)?,
        submission: r.get(5)?,
        status_before: r.get(6)?,
        tier: r.get::<_, i64>(7)?.clamp(0, 2) as u8,
        bucket: r.get(8)?,
        state: r.get(9)?,
        verdict_id: r.get(10)?,
        finding: r.get(11)?,
        currency: serde_json::from_str(&currency).unwrap_or(Value::Null),
        tries: r.get(13)?,
        retry_at: r.get(14)?,
        applied_status: r.get(15)?,
        applied_updated_at: r.get(16)?,
        detail: r.get(17)?,
    })
}

/// Tasks waiting for review that `scope` covers, at most [`MAX_SWEEP_TASKS`], with their rank:
/// those other open tasks wait on first, then tier 2, then the longest waiting.
pub fn tasks_in_scope(
    conn: &Connection,
    scope: &SweepScope,
    cfg: &hq_core::config::VerificationConfig,
) -> Result<Vec<(Task, u8, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT t.id FROM tasks t WHERE t.status = 'ready_for_review' AND t.archived_at IS NULL \
         ORDER BY t.created_at, t.id",
    )?;
    let ids: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let wanted: Option<std::collections::HashSet<String>> = match scope {
        SweepScope::Tasks { ids } => {
            let mut set = std::collections::HashSet::new();
            for reference in ids {
                if let Some(task) = get_task(conn, reference)? {
                    set.insert(task.id);
                }
            }
            Some(set)
        }
        _ => None,
    };
    let mut out = Vec::new();
    for id in ids {
        let Some(task) = get_task(conn, &id)? else {
            continue;
        };
        if wanted.as_ref().is_some_and(|w| !w.contains(&task.id)) {
            continue;
        }
        if let SweepScope::Initiative { id } = scope
            && &task.initiative_id != id
        {
            continue;
        }
        let entered: String = conn.query_row(
            "SELECT COALESCE(MAX(occurred_at), '') FROM task_events \
             WHERE task_id = ?1 AND event_type = 'entered_ready_for_review'",
            params![task.id],
            |r| r.get(0),
        )?;
        if let SweepScope::Since { date } = scope
            && entered.as_str() < date.as_str()
        {
            continue;
        }
        let tier = task_tier(conn, &task, cfg)?;
        let dependents: i64 = conn.query_row(
            "SELECT COUNT(*) FROM task_dependencies d JOIN tasks x ON x.id = d.task_id \
             WHERE d.depends_on_task_id = ?1 AND x.status <> 'complete' AND x.archived_at IS NULL",
            params![task.id],
            |r| r.get(0),
        )?;
        // Sorting key as one number: waited-on first, then tier 2, then oldest.
        let rank =
            i64::from(dependents == 0) * 4 + i64::from(tier < super::verification::MAX_TIER) * 2;
        out.push((task, tier, rank));
    }
    // Stable by rank then the creation order the query gave, so the oldest come first.
    out.sort_by_key(|(_, _, rank)| *rank);
    out.truncate(MAX_SWEEP_TASKS);
    Ok(out)
}

/// A task to put in a sweep.
pub struct NewSweepItem {
    pub task_id: String,
    pub rank: i64,
    pub submission: Option<i64>,
    pub status_before: String,
    pub tier: u8,
}

/// Creates a sweep in `planning` with one unplanned item per task.
pub fn create_sweep(
    conn: &Connection,
    scope: &SweepScope,
    items: &[NewSweepItem],
) -> Result<Sweep> {
    in_write_tx(conn, |conn| {
        let id = format!("sw-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        conn.execute(
            "INSERT INTO task_sweeps (id, scope, status) VALUES (?1, ?2, ?3)",
            params![id, serde_json::to_string(scope)?, SWEEP_PLANNING],
        )?;
        for (position, item) in items.iter().enumerate() {
            conn.execute(
                "INSERT INTO task_sweep_items (sweep_id, task_id, rank, submission, status_before, tier) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    item.task_id,
                    item.rank * 100_000 + position as i64,
                    item.submission,
                    item.status_before,
                    item.tier
                ],
            )?;
        }
        get_sweep(conn, &id)?.ok_or_else(|| anyhow::anyhow!("the sweep was not created"))
    })
}

/// Plans a sweep over `scope` now: the tasks it covers, ranked, as unplanned items.
pub fn plan_sweep_items(
    conn: &Connection,
    scope: &SweepScope,
    cfg: &hq_core::config::VerificationConfig,
) -> Result<Sweep> {
    let tasks = tasks_in_scope(conn, scope, cfg)?;
    if tasks.is_empty() {
        anyhow::bail!("nothing is waiting for review in that scope");
    }
    let mut items = Vec::with_capacity(tasks.len());
    for (task, tier, rank) in tasks {
        items.push(NewSweepItem {
            submission: review_marker(conn, &task.id)?,
            status_before: task.status.clone(),
            task_id: task.id,
            rank,
            tier,
        });
    }
    create_sweep(conn, scope, &items)
}

pub fn get_sweep(conn: &Connection, id: &str) -> Result<Option<Sweep>> {
    Ok(conn
        .query_row(
            &format!("SELECT {SWEEP_COLS} FROM task_sweeps WHERE id = ?1"),
            params![id],
            row_to_sweep,
        )
        .optional()?)
}

/// Newest first.
pub fn list_sweeps(conn: &Connection, limit: usize) -> Result<Vec<Sweep>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SWEEP_COLS} FROM task_sweeps ORDER BY created_at DESC, rowid DESC LIMIT {}",
        limit.clamp(1, 200)
    ))?;
    let rows = stmt
        .query_map([], row_to_sweep)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Sweeps in a status the daemon advances.
pub fn active_sweeps(conn: &Connection) -> Result<Vec<Sweep>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SWEEP_COLS} FROM task_sweeps \
         WHERE status IN ('planning', 'running', 'applying') ORDER BY created_at, rowid"
    ))?;
    let rows = stmt
        .query_map([], row_to_sweep)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn sweep_items(conn: &Connection, sweep_id: &str) -> Result<Vec<SweepItem>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ITEM_COLS} FROM task_sweep_items i JOIN tasks t ON t.id = i.task_id \
         WHERE i.sweep_id = ?1 ORDER BY i.rank"
    ))?;
    let rows = stmt
        .query_map(params![sweep_id], row_to_item)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Items in `state` that are due, in work order.
pub fn items_to_work(
    conn: &Connection,
    sweep_id: &str,
    state: &str,
    now: i64,
    limit: usize,
) -> Result<Vec<SweepItem>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ITEM_COLS} FROM task_sweep_items i JOIN tasks t ON t.id = i.task_id \
         WHERE i.sweep_id = ?1 AND i.state = ?2 AND i.retry_at <= ?3 ORDER BY i.rank LIMIT {}",
        limit.clamp(1, 200)
    ))?;
    let rows = stmt
        .query_map(params![sweep_id, state, now], row_to_item)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Whether the sweep still has items in `state`, due or not.
pub fn any_items_in(conn: &Connection, sweep_id: &str, state: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM task_sweep_items WHERE sweep_id = ?1 AND state = ?2 LIMIT 1",
            params![sweep_id, state],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Moves a sweep to `to` if it is in one of `from`.
pub fn set_sweep_status(conn: &Connection, id: &str, from: &[&str], to: &str) -> Result<Sweep> {
    in_write_tx(conn, |conn| {
        let sweep = get_sweep(conn, id)?.ok_or_else(|| anyhow::anyhow!("no sweep '{id}'"))?;
        if !from.contains(&sweep.status.as_str()) {
            anyhow::bail!(
                "sweep {id} is {}, so it cannot become {to} (it must be {})",
                sweep.status,
                from.join(" or ")
            );
        }
        let stamp = match to {
            SWEEP_APPLIED => ", applied_at = datetime('now')",
            SWEEP_UNDONE => ", undone_at = datetime('now')",
            _ => "",
        };
        conn.execute(
            &format!("UPDATE task_sweeps SET status = ?2, updated_at = datetime('now'){stamp} WHERE id = ?1"),
            params![id, to],
        )?;
        get_sweep(conn, id)?.ok_or_else(|| anyhow::anyhow!("sweep vanished"))
    })
}

pub fn set_sweep_plan(conn: &Connection, id: &str, plan: &Value, note: &str) -> Result<()> {
    conn.execute(
        "UPDATE task_sweeps SET plan = ?2, note = ?3, updated_at = datetime('now') WHERE id = ?1",
        params![
            id,
            serde_json::to_string(plan)?,
            note.chars().take(500).collect::<String>()
        ],
    )?;
    Ok(())
}

/// Records what planning found out about one item.
pub fn plan_item(
    conn: &Connection,
    sweep_id: &str,
    task_id: &str,
    bucket: &str,
    finding: &str,
    currency: &Value,
) -> Result<()> {
    conn.execute(
        "UPDATE task_sweep_items SET bucket = ?3, state = 'planned', finding = ?4, currency = ?5 \
         WHERE sweep_id = ?1 AND task_id = ?2 AND state = 'unplanned'",
        params![
            sweep_id,
            task_id,
            bucket,
            finding.chars().take(500).collect::<String>(),
            serde_json::to_string(currency)?
        ],
    )?;
    Ok(())
}

/// Puts an item back to try again later, counting the try. Returns the tries so far.
pub fn defer_item(
    conn: &Connection,
    sweep_id: &str,
    task_id: &str,
    retry_at: i64,
    detail: &str,
) -> Result<i64> {
    conn.execute(
        "UPDATE task_sweep_items SET tries = tries + 1, retry_at = ?3, detail = ?4 \
         WHERE sweep_id = ?1 AND task_id = ?2",
        params![
            sweep_id,
            task_id,
            retry_at,
            detail.chars().take(300).collect::<String>()
        ],
    )?;
    Ok(conn.query_row(
        "SELECT tries FROM task_sweep_items WHERE sweep_id = ?1 AND task_id = ?2",
        params![sweep_id, task_id],
        |r| r.get(0),
    )?)
}

/// Records the verdict a sweep produced for an item and its final bucket.
pub fn record_item_verdict(
    conn: &Connection,
    sweep_id: &str,
    task_id: &str,
    verdict_id: Option<&str>,
    bucket: &str,
    finding: &str,
    currency: &Value,
) -> Result<()> {
    conn.execute(
        "UPDATE task_sweep_items SET state = 'verdict', verdict_id = ?3, bucket = ?4, finding = ?5, \
         currency = ?6 WHERE sweep_id = ?1 AND task_id = ?2",
        params![
            sweep_id,
            task_id,
            verdict_id,
            bucket,
            finding.chars().take(500).collect::<String>(),
            serde_json::to_string(currency)?
        ],
    )?;
    Ok(())
}

/// Records what apply or undo did to an item.
pub fn mark_item(
    conn: &Connection,
    sweep_id: &str,
    task_id: &str,
    state: &str,
    detail: &str,
    applied: Option<(&str, &str)>,
) -> Result<()> {
    conn.execute(
        "UPDATE task_sweep_items SET state = ?3, detail = ?4, \
         applied_status = COALESCE(?5, applied_status), \
         applied_updated_at = COALESCE(?6, applied_updated_at) \
         WHERE sweep_id = ?1 AND task_id = ?2",
        params![
            sweep_id,
            task_id,
            state,
            detail.chars().take(300).collect::<String>(),
            applied.map(|a| a.0),
            applied.map(|a| a.1)
        ],
    )?;
    Ok(())
}

/// Items per bucket, and per state, for the progress and the bucket summary.
pub fn sweep_summary(conn: &Connection, sweep_id: &str) -> Result<Value> {
    let mut buckets: BTreeMap<String, i64> = BTreeMap::new();
    let mut states: BTreeMap<String, i64> = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT bucket, state, COUNT(*) FROM task_sweep_items WHERE sweep_id = ?1 GROUP BY bucket, state",
    )?;
    let rows = stmt.query_map(params![sweep_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut total = 0;
    for row in rows {
        let (bucket, state, n) = row?;
        *buckets.entry(bucket).or_insert(0) += n;
        *states.entry(state).or_insert(0) += n;
        total += n;
    }
    Ok(json!({ "total": total, "buckets": buckets, "states": states }))
}

/// The check an apply makes on a task: still in review and still the submission the sweep looked
/// at. `None` means it is.
pub fn still_the_same_submission(conn: &Connection, item: &SweepItem) -> Result<Option<String>> {
    let Some(task) = get_task(conn, &item.task_id)? else {
        return Ok(Some("the task is gone".into()));
    };
    if task.status != STATUS_READY_FOR_REVIEW {
        return Ok(Some(format!("it is {} now", task.status)));
    }
    if review_marker(conn, &task.id)? != item.submission {
        return Ok(Some("it was submitted again".into()));
    }
    Ok(None)
}

/// Sets an item aside before it was looked at, because the task moved on.
pub fn set_aside(
    conn: &Connection,
    sweep_id: &str,
    task_id: &str,
    bucket: &str,
    state: &str,
    detail: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE task_sweep_items SET bucket = ?3, state = ?4, detail = ?5 \
         WHERE sweep_id = ?1 AND task_id = ?2",
        params![
            sweep_id,
            task_id,
            bucket,
            state,
            detail.chars().take(300).collect::<String>()
        ],
    )?;
    Ok(())
}

/// What applying a sweep would do, and whether it needs the owner's confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplyPreview {
    /// Tasks apply would move (complete, return to `to_do`, or open a veto window on).
    pub will_move: i64,
    /// Tasks apply leaves where they are.
    pub will_keep: i64,
    pub confirm_over: u32,
    pub needs_confirmation: bool,
}

/// Counts what apply would do from the recorded verdicts.
pub fn apply_preview(
    conn: &Connection,
    cfg: &hq_core::config::VerificationConfig,
    sweep_id: &str,
) -> Result<ApplyPreview> {
    let mut will_move = 0;
    let mut will_keep = 0;
    for item in sweep_items(conn, sweep_id)? {
        if item.state != ITEM_VERDICT {
            continue;
        }
        let moves = match item.bucket.as_str() {
            BUCKET_PASSED | BUCKET_FAILED | BUCKET_NO_ARTIFACT => true,
            BUCKET_NEEDS_YOU => item.tier >= super::verification::MAX_TIER && cfg.veto_hours > 0,
            _ => false,
        };
        if moves {
            will_move += 1;
        } else {
            will_keep += 1;
        }
    }
    Ok(ApplyPreview {
        will_move,
        will_keep,
        confirm_over: cfg.sweep_confirm_over,
        needs_confirmation: will_move > i64::from(cfg.sweep_confirm_over),
    })
}

/// Asks for a sweep to be applied. Above `sweep_confirm_over` moves it needs `confirmed`, whatever
/// the mode. The daemon does the work, a few tasks at a time.
pub fn request_apply(
    conn: &Connection,
    cfg: &hq_core::config::VerificationConfig,
    sweep_id: &str,
    confirmed: bool,
) -> Result<Sweep> {
    if cfg.mode != hq_core::config::VerificationMode::Enforce {
        anyhow::bail!(
            "verification is not enforcing (tasks.verification.mode is {:?}), so a sweep can be \
             read but not applied",
            cfg.mode
        );
    }
    let preview = apply_preview(conn, cfg, sweep_id)?;
    if preview.needs_confirmation && !confirmed {
        anyhow::bail!(
            "applying moves {} tasks, which is over {}: confirm it explicitly",
            preview.will_move,
            preview.confirm_over
        );
    }
    set_sweep_status(conn, sweep_id, &[SWEEP_READY], SWEEP_APPLYING)
}
