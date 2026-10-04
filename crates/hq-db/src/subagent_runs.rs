//! Durable registry of delegated child runs and the outbox that wakes a
//! parent turn when one settles. Execution status (did the process finish)
//! is kept apart from acceptance status (was the deliverable verified).
//! Timestamps are unix epoch seconds.

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

pub const EXEC_QUEUED: &str = "queued";
pub const EXEC_RUNNING: &str = "running";
pub const EXEC_COMPLETED: &str = "completed";
pub const EXEC_FAILED: &str = "failed";
pub const EXEC_TIMED_OUT: &str = "timed_out";
pub const EXEC_BLOCKED: &str = "blocked";
pub const EXEC_REJECTED: &str = "rejected";
pub const EXEC_CANCELLED: &str = "cancelled";
pub const EXEC_INTERRUPTED: &str = "interrupted";

pub const ACCEPT_UNVERIFIED: &str = "unverified";
pub const ACCEPT_ACCEPTED: &str = "accepted";
pub const ACCEPT_PARTIAL: &str = "partial";
pub const ACCEPT_BLOCKED: &str = "blocked";

pub const EVENT_PENDING: &str = "pending";
pub const EVENT_CLAIMED: &str = "claimed";
pub const EVENT_DELIVERED: &str = "delivered";
pub const EVENT_FAILED: &str = "failed";
pub const EVENT_SUPPRESSED: &str = "suppressed";

pub const KIND_SETTLED: &str = "settled";
pub const KIND_INTERRUPTED: &str = "interrupted";
pub const KIND_STALE: &str = "stale";
pub const KIND_HUNG: &str = "hung";

/// A running child with no activity for this long is stale.
pub const STALE_AFTER_SECS: i64 = 300;
/// A running child this far past its deadline is treated as hung.
pub const HUNG_GRACE_SECS: i64 = 120;
/// A claimed event nobody acknowledged within this window may be re-claimed.
pub const CLAIM_TTL_SECS: i64 = 180;
pub const MAX_DELIVERY_ATTEMPTS: i64 = 5;
const RETRY_BASE_SECS: i64 = 30;
const RETRY_CAP_SECS: i64 = 900;
/// Child runs started inside a follow-up turn inherit depth+1; past this the
/// parent is no longer woken automatically.
pub const MAX_FOLLOWUP_DEPTH: i64 = 3;
/// `parent_turn_id` prefix marking a turn HQ started to follow up a run.
pub const FOLLOWUP_TURN_PREFIX: &str = "followup:";
/// Events older than this are never woken automatically: switching the
/// follow-up loop on must not replay a backlog.
pub const MAX_EVENT_AGE_SECS: i64 = 24 * 3600;
const ROUTABLE_PLATFORMS: &str = "('web','telegram','discord')";
pub const RESULT_PREVIEW_BYTES: usize = 500;
/// A queued child with no start must still settle by then, or it is closed.
pub const QUEUE_DEADLINE_SECS: i64 = 3 * 3600;
/// Past its deadline by this much, a run is closed whoever owns it: a child
/// that is alive always settles by its deadline through its own timeout.
pub const OVERDUE_GRACE_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize)]
pub struct RunRow {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub parent_turn_id: Option<String>,
    pub platform: Option<String>,
    pub chat_id: Option<String>,
    pub thread_id: Option<String>,
    pub identity: Option<String>,
    pub task_id: Option<String>,
    pub child_id: String,
    pub role: String,
    pub goal: String,
    pub success_criteria: Vec<String>,
    pub required_tools: Vec<String>,
    pub detached: bool,
    pub owner_pid: Option<i64>,
    pub followup_depth: i64,
    pub exec_status: String,
    pub accept_status: String,
    pub missing_deliverables: Vec<String>,
    pub blocker_reason: Option<String>,
    pub next_action: Option<String>,
    pub output_full: Option<String>,
    pub output_preview: Option<String>,
    pub error: Option<String>,
    pub resolved_backend: Option<String>,
    pub telemetry_seen: bool,
    pub started_at: i64,
    pub last_activity_at: i64,
    pub deadline_at: Option<i64>,
    pub settled_at: Option<i64>,
    pub escalated_at: Option<i64>,
}

/// How fresh a run's evidence is. `Unknown` means no telemetry ever arrived,
/// which is not the same as working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Liveness {
    Settled,
    Working,
    Unknown,
    Stale,
    Hung,
}

impl RunRow {
    pub fn liveness(&self, now: i64) -> Liveness {
        if self.settled_at.is_some() {
            return Liveness::Settled;
        }
        if let Some(deadline) = self.deadline_at
            && now > deadline + HUNG_GRACE_SECS
        {
            return Liveness::Hung;
        }
        if now - self.last_activity_at <= STALE_AFTER_SECS {
            return Liveness::Working;
        }
        if self.telemetry_seen {
            Liveness::Stale
        } else {
            Liveness::Unknown
        }
    }
}

const RUN_COLS: &str = "run_id, parent_run_id, parent_turn_id, platform, chat_id, thread_id, identity, task_id, child_id, role, goal, success_criteria, required_tools, detached, owner_pid, followup_depth, exec_status, accept_status, missing_deliverables, blocker_reason, next_action, output_full, output_preview, error, resolved_backend, telemetry_seen, started_at, last_activity_at, deadline_at, settled_at, escalated_at";

fn json_list(row: &Row, idx: usize) -> rusqlite::Result<Vec<String>> {
    let raw: String = row.get(idx)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn row_to_run(row: &Row) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        run_id: row.get(0)?,
        parent_run_id: row.get(1)?,
        parent_turn_id: row.get(2)?,
        platform: row.get(3)?,
        chat_id: row.get(4)?,
        thread_id: row.get(5)?,
        identity: row.get(6)?,
        task_id: row.get(7)?,
        child_id: row.get(8)?,
        role: row.get(9)?,
        goal: row.get(10)?,
        success_criteria: json_list(row, 11)?,
        required_tools: json_list(row, 12)?,
        detached: row.get::<_, i64>(13)? != 0,
        owner_pid: row.get(14)?,
        followup_depth: row.get(15)?,
        exec_status: row.get(16)?,
        accept_status: row.get(17)?,
        missing_deliverables: json_list(row, 18)?,
        blocker_reason: row.get(19)?,
        next_action: row.get(20)?,
        output_full: row.get(21)?,
        output_preview: row.get(22)?,
        error: row.get(23)?,
        resolved_backend: row.get(24)?,
        telemetry_seen: row.get::<_, i64>(25)? != 0,
        started_at: row.get(26)?,
        last_activity_at: row.get(27)?,
        deadline_at: row.get(28)?,
        settled_at: row.get(29)?,
        escalated_at: row.get(30)?,
    })
}

/// The chat a run belongs to. Used to keep one chat from reading another's runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub platform: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub identity: Option<String>,
}

impl Origin {
    /// Scope for a caller with no chat (CLI, proxy): sees only unrouted runs.
    pub fn unrouted() -> Self {
        Self {
            platform: String::new(),
            chat_id: String::new(),
            thread_id: None,
            identity: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewRun {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub parent_turn_id: Option<String>,
    pub origin: Option<Origin>,
    pub task_id: Option<String>,
    pub child_id: String,
    pub role: String,
    pub goal: String,
    pub success_criteria: Vec<String>,
    pub required_tools: Vec<String>,
    pub detached: bool,
    pub owner_pid: Option<i64>,
    pub followup_depth: i64,
}

pub fn insert_run(conn: &Connection, run: &NewRun, now: i64) -> Result<()> {
    let (platform, chat_id, thread_id, identity) = match &run.origin {
        Some(o) => (
            Some(o.platform.as_str()),
            Some(o.chat_id.as_str()),
            o.thread_id.as_deref(),
            o.identity.as_deref(),
        ),
        None => (None, None, None, None),
    };
    conn.execute(
        "INSERT INTO subagent_runs (run_id, parent_run_id, parent_turn_id, platform, chat_id, thread_id, identity, task_id, child_id, role, goal, success_criteria, required_tools, detached, owner_pid, followup_depth, started_at, last_activity_at, deadline_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17, ?17 + ?18)",
        params![
            run.run_id,
            run.parent_run_id,
            run.parent_turn_id,
            platform,
            chat_id,
            thread_id,
            identity,
            run.task_id,
            run.child_id,
            run.role,
            run.goal,
            serde_json::to_string(&run.success_criteria)?,
            serde_json::to_string(&run.required_tools)?,
            run.detached as i64,
            run.owner_pid,
            run.followup_depth,
            now,
            QUEUE_DEADLINE_SECS,
        ],
    )?;
    Ok(())
}

/// Mark a queued child as actually running and record its deadline.
pub fn mark_running(conn: &Connection, run_id: &str, now: i64, timeout_secs: i64) -> Result<()> {
    conn.execute(
        "UPDATE subagent_runs SET exec_status = ?2, started_at = ?3, last_activity_at = ?3, deadline_at = ?4
         WHERE run_id = ?1 AND settled_at IS NULL",
        params![run_id, EXEC_RUNNING, now, now + timeout_secs],
    )?;
    Ok(())
}

/// Record that the child produced evidence of life.
pub fn touch(conn: &Connection, run_id: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE subagent_runs SET last_activity_at = ?2, telemetry_seen = 1
         WHERE run_id = ?1 AND settled_at IS NULL",
        params![run_id, now],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Settlement {
    pub exec_status: String,
    pub accept_status: String,
    pub missing_deliverables: Vec<String>,
    pub blocker_reason: Option<String>,
    pub next_action: Option<String>,
    pub output_full: String,
    pub output_preview: String,
    pub error: Option<String>,
    pub resolved_backend: String,
}

/// Settle a run once. Returns `true` only for the call that did it, so a
/// duplicate settle (race sweep, retry) is a no-op. `wake` also writes the
/// terminal event in the same transaction.
pub fn settle(
    conn: &Connection,
    run_id: &str,
    s: &Settlement,
    now: i64,
    wake: bool,
) -> Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let changed = tx.execute(
        "UPDATE subagent_runs SET exec_status = ?2, accept_status = ?3, missing_deliverables = ?4, blocker_reason = ?5, next_action = ?6, output_full = ?7, output_preview = ?8, error = ?9, resolved_backend = ?10, settled_at = ?11, last_activity_at = ?11
         WHERE run_id = ?1 AND settled_at IS NULL",
        params![
            run_id,
            s.exec_status,
            s.accept_status,
            serde_json::to_string(&s.missing_deliverables)?,
            s.blocker_reason,
            s.next_action,
            s.output_full,
            s.output_preview,
            s.error,
            s.resolved_backend,
            now,
        ],
    )?;
    if changed == 1 && wake {
        insert_event(
            &tx,
            run_id,
            KIND_SETTLED,
            &format!("{KIND_SETTLED}:{run_id}"),
            now,
        )?;
    }
    tx.commit()?;
    Ok(changed == 1)
}

fn insert_event(
    conn: &Connection,
    run_id: &str,
    kind: &str,
    dedupe_key: &str,
    now: i64,
) -> Result<()> {
    let depth: i64 = conn
        .query_row(
            "SELECT followup_depth FROM subagent_runs WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    let status = if depth > MAX_FOLLOWUP_DEPTH {
        EVENT_SUPPRESSED
    } else {
        EVENT_PENDING
    };
    conn.execute(
        "INSERT OR IGNORE INTO subagent_events (run_id, dedupe_key, kind, status, next_attempt_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![run_id, dedupe_key, kind, status, now],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, run_id: &str) -> Result<Option<RunRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {RUN_COLS} FROM subagent_runs WHERE run_id = ?1"),
            params![run_id],
            row_to_run,
        )
        .optional()?)
}

fn visible(row: &RunRow, scope: Option<&Origin>) -> bool {
    match scope {
        None => true,
        Some(o) if o.platform.is_empty() => row.platform.is_none(),
        Some(o) => {
            row.platform.as_deref() == Some(o.platform.as_str())
                && row.chat_id.as_deref() == Some(o.chat_id.as_str())
        }
    }
}

/// Look a run up by full id or unique prefix (6+ chars). With a scope, a run
/// from another chat is reported as not found.
pub fn find(
    conn: &Connection,
    id_or_prefix: &str,
    scope: Option<&Origin>,
) -> Result<Option<RunRow>> {
    let id = id_or_prefix.trim();
    if let Some(row) = get(conn, id)? {
        return Ok(visible(&row, scope).then_some(row));
    }
    if id.len() < 6 {
        return Ok(None);
    }
    let like = format!("{}%", id.replace(['%', '_'], ""));
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLS} FROM subagent_runs WHERE run_id LIKE ?1 ORDER BY started_at DESC LIMIT 2"
    ))?;
    let rows = stmt
        .query_map(params![like], row_to_run)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match rows.as_slice() {
        [only] if visible(only, scope) => Ok(Some(only.clone())),
        _ => Ok(None),
    }
}

#[derive(Debug, Default, Clone)]
pub struct ListFilter {
    pub scope: Option<Origin>,
    pub task_id: Option<String>,
    pub open_only: bool,
    pub limit: i64,
}

pub fn list(conn: &Connection, f: &ListFilter) -> Result<Vec<RunRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLS} FROM subagent_runs
         WHERE (?1 IS NULL OR (platform = ?1 AND chat_id = ?2) OR (?1 = '' AND platform IS NULL))
           AND (?3 IS NULL OR task_id = ?3)
           AND (?4 = 0 OR settled_at IS NULL)
         ORDER BY started_at DESC LIMIT ?5"
    ))?;
    let (platform, chat_id) = match &f.scope {
        Some(o) => (Some(o.platform.clone()), Some(o.chat_id.clone())),
        None => (None, None),
    };
    let limit = if f.limit <= 0 { 20 } else { f.limit.min(200) };
    let rows = stmt
        .query_map(
            params![platform, chat_id, f.task_id, f.open_only as i64, limit],
            row_to_run,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Settled runs still `partial` or `blocked` after the parent had time to
/// review them: work that is not done and has no one on it.
pub fn list_unresolved(
    conn: &Connection,
    settled_after: i64,
    settled_before: i64,
) -> Result<Vec<RunRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLS} FROM subagent_runs
         WHERE settled_at IS NOT NULL AND settled_at >= ?1 AND settled_at <= ?2
           AND (accept_status IN ('partial','blocked') OR exec_status = 'interrupted')
           AND exec_status != 'cancelled' AND escalated_at IS NULL
         ORDER BY settled_at LIMIT 100"
    ))?;
    let rows = stmt
        .query_map(params![settled_after, settled_before], row_to_run)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Remember that a run was escalated to the user, so it is raised once.
pub fn mark_escalated(conn: &Connection, run_id: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE subagent_runs SET escalated_at = ?2 WHERE run_id = ?1 AND escalated_at IS NULL",
        params![run_id, now],
    )?;
    Ok(())
}

/// Parent verdict on a settled run. Refuses an unsettled one: acceptance
/// needs a finished result to check.
pub fn review(
    conn: &Connection,
    run_id: &str,
    accept: &str,
    missing: &[String],
    note: Option<&str>,
    now: i64,
) -> Result<()> {
    if ![ACCEPT_ACCEPTED, ACCEPT_PARTIAL, ACCEPT_BLOCKED].contains(&accept) {
        bail!("accept_status must be accepted, partial or blocked");
    }
    if accept == ACCEPT_ACCEPTED && !missing.is_empty() {
        bail!("an accepted run cannot list missing deliverables");
    }
    let changed = conn.execute(
        "UPDATE subagent_runs SET accept_status = ?2, missing_deliverables = ?3, next_action = ?4,
             blocker_reason = CASE WHEN ?2 = 'accepted' THEN NULL ELSE blocker_reason END,
             last_activity_at = ?5
         WHERE run_id = ?1 AND settled_at IS NOT NULL AND exec_status != 'cancelled'
           AND (?2 != 'accepted' OR exec_status = 'completed')",
        params![run_id, accept, serde_json::to_string(missing)?, note, now],
    )?;
    if changed == 0 {
        bail!("run `{run_id}` cannot take this verdict: it is unsettled, cancelled, missing, or (for accepted) did not complete");
    }
    Ok(())
}

/// Cancel an open run. Later settle calls become no-ops and any unsent event
/// is suppressed, so cancelled work is never resumed or reported as done.
pub fn cancel(conn: &Connection, run_id: &str, now: i64) -> Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let changed = tx.execute(
        "UPDATE subagent_runs SET exec_status = ?2, accept_status = ?3, blocker_reason = 'cancelled', settled_at = ?4, last_activity_at = ?4
         WHERE run_id = ?1 AND settled_at IS NULL",
        params![run_id, EXEC_CANCELLED, ACCEPT_BLOCKED, now],
    )?;
    tx.execute(
        "UPDATE subagent_events SET status = ?2 WHERE run_id = ?1 AND status IN ('pending','claimed')",
        params![run_id, EVENT_SUPPRESSED],
    )?;
    tx.commit()?;
    Ok(changed == 1)
}

fn open_runs_oldest_first(conn: &Connection) -> Result<Vec<RunRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLS} FROM subagent_runs WHERE settled_at IS NULL ORDER BY started_at LIMIT 1000"
    ))?;
    let rows = stmt
        .query_map([], row_to_run)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Interrupt the given open runs and queue one event each. Only a run this
/// call actually closed gets an event, so a run that settled in the meantime
/// is not reported as interrupted.
fn interrupt(conn: &Connection, ids: &[String], reason: &str, now: i64) -> Result<Vec<String>> {
    let tx = conn.unchecked_transaction()?;
    let mut closed = Vec::new();
    for id in ids {
        let changed = tx.execute(
            "UPDATE subagent_runs SET exec_status = ?2, blocker_reason = ?3,
                 next_action = 'read back any external side effects, then re-dispatch or escalate', settled_at = ?4
             WHERE run_id = ?1 AND settled_at IS NULL",
            params![id, EXEC_INTERRUPTED, reason, now],
        )?;
        if changed == 1 {
            insert_event(&tx, id, KIND_INTERRUPTED, &format!("{KIND_INTERRUPTED}:{id}"), now)?;
            closed.push(id.clone());
        }
    }
    tx.commit()?;
    Ok(closed)
}

/// Close every open run whose owning process is gone: its child died with
/// it. `alive` answers whether a pid still exists. A run with no recorded
/// owner is left alone. Returns the affected run ids.
pub fn reconcile_dead_owners(
    conn: &Connection,
    now: i64,
    alive: impl Fn(i64) -> bool,
) -> Result<Vec<String>> {
    let dead: Vec<String> = open_runs_oldest_first(conn)?
        .into_iter()
        .filter(|r| r.owner_pid.is_some_and(|pid| !alive(pid)))
        .map(|r| r.run_id)
        .collect();
    interrupt(conn, &dead, "the process running this child ended before it settled", now)
}

/// Close open runs far past their deadline whoever owns them. A live child
/// settles by its deadline through its own timeout, so this catches what a
/// pid check cannot: a reused pid, or a plan task that died inside a live
/// process.
pub fn reconcile_overdue(conn: &Connection, now: i64) -> Result<Vec<String>> {
    let overdue: Vec<String> = open_runs_oldest_first(conn)?
        .into_iter()
        .filter(|r| r.deadline_at.is_some_and(|d| now > d + OVERDUE_GRACE_SECS))
        .map(|r| r.run_id)
        .collect();
    interrupt(conn, &overdue, "this child did not settle before its deadline", now)
}

/// Queue one event per open run that has gone quiet or overrun its deadline.
/// The run stays open: this reports, it does not declare the child dead.
pub fn flag_stalled(conn: &Connection, now: i64) -> Result<Vec<(String, Liveness)>> {
    let open = list(
        conn,
        &ListFilter {
            open_only: true,
            limit: 200,
            ..Default::default()
        },
    )?;
    let mut flagged = Vec::new();
    for run in open {
        let live = run.liveness(now);
        if run.exec_status == EXEC_QUEUED {
            continue;
        }
        let kind = match live {
            Liveness::Hung => KIND_HUNG,
            Liveness::Stale => KIND_STALE,
            _ => continue,
        };
        let before: i64 =
            conn.query_row("SELECT COUNT(*) FROM subagent_events", [], |r| r.get(0))?;
        insert_event(
            conn,
            &run.run_id,
            kind,
            &format!("{kind}:{}", run.run_id),
            now,
        )?;
        let after: i64 =
            conn.query_row("SELECT COUNT(*) FROM subagent_events", [], |r| r.get(0))?;
        if after > before {
            flagged.push((run.run_id, live));
        }
    }
    Ok(flagged)
}

#[derive(Debug, Clone, Serialize)]
pub struct EventRow {
    pub id: i64,
    pub run_id: String,
    pub kind: String,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
}

/// Claim up to `limit` due events for delivery. The UPDATE re-checks the
/// predicate, so two claimants never both win the same event.
pub fn claim_due(
    conn: &Connection,
    claimant: &str,
    platform: &str,
    now: i64,
    limit: i64,
) -> Result<Vec<(EventRow, RunRow)>> {
    let stale_before = now - CLAIM_TTL_SECS;
    let candidates: Vec<(i64, i64)> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT e.id, e.attempts FROM subagent_events e JOIN subagent_runs r ON r.run_id = e.run_id
             WHERE r.platform = ?4 AND r.platform IN {ROUTABLE_PLATFORMS}
               AND e.created_at >= ?5
               AND ((e.status = 'pending' AND e.next_attempt_at <= ?1)
                 OR (e.status = 'claimed' AND e.claimed_at <= ?2))
             ORDER BY e.id LIMIT ?3"
        ))?;
        stmt.query_map(
            params![now, stale_before, limit, platform, now - MAX_EVENT_AGE_SECS],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut won = Vec::new();
    for (id, attempts) in candidates {
        if attempts >= MAX_DELIVERY_ATTEMPTS {
            conn.execute(
                "UPDATE subagent_events SET status = ?2, last_error = COALESCE(last_error, 'delivery attempts exhausted') WHERE id = ?1 AND status IN ('pending','claimed')",
                params![id, EVENT_FAILED],
            )?;
            continue;
        }
        let n = conn.execute(
            "UPDATE subagent_events SET status = 'claimed', claimed_at = ?2, claimed_by = ?3, attempts = attempts + 1
             WHERE id = ?1 AND ((status = 'pending' AND next_attempt_at <= ?2) OR (status = 'claimed' AND claimed_at <= ?4))",
            params![id, now, claimant, stale_before],
        )?;
        if n == 1 {
            won.push(id);
        }
    }
    let mut out = Vec::new();
    for id in won {
        let event = get_event(conn, id)?;
        if let Some(run) = get(conn, &event.run_id)? {
            out.push((event, run));
        }
    }
    Ok(out)
}

pub fn get_event(conn: &Connection, id: i64) -> Result<EventRow> {
    Ok(conn.query_row(
        "SELECT id, run_id, kind, status, attempts, last_error FROM subagent_events WHERE id = ?1",
        params![id],
        |r| {
            Ok(EventRow {
                id: r.get(0)?,
                run_id: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                attempts: r.get(4)?,
                last_error: r.get(5)?,
            })
        },
    )?)
}

pub fn events_for_run(conn: &Connection, run_id: &str) -> Result<Vec<EventRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, run_id, kind, status, attempts, last_error FROM subagent_events WHERE run_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![run_id], |r| {
            Ok(EventRow {
                id: r.get(0)?,
                run_id: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                attempts: r.get(4)?,
                last_error: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn mark_delivered(conn: &Connection, event_id: i64, now: i64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE subagent_events SET status = ?2, delivered_at = ?3 WHERE id = ?1 AND status = 'claimed'",
        params![event_id, EVENT_DELIVERED, now],
    )?;
    Ok(n == 1)
}

/// Drop a claimed event that no longer needs a turn (the run was accepted or
/// cancelled while it waited).
pub fn suppress(conn: &Connection, event_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE subagent_events SET status = ?2 WHERE id = ?1 AND status = 'claimed'",
        params![event_id, EVENT_SUPPRESSED],
    )?;
    Ok(())
}

/// Return a failed delivery to the queue with backoff, or fail it for good
/// once the attempt budget is spent. Returns the resulting status.
pub fn release_failed(conn: &Connection, event_id: i64, error: &str, now: i64) -> Result<String> {
    let attempts: i64 = conn.query_row(
        "SELECT attempts FROM subagent_events WHERE id = ?1",
        params![event_id],
        |r| r.get(0),
    )?;
    if attempts >= MAX_DELIVERY_ATTEMPTS {
        conn.execute(
            "UPDATE subagent_events SET status = ?2, last_error = ?3 WHERE id = ?1 AND status = 'claimed'",
            params![event_id, EVENT_FAILED, error],
        )?;
        return Ok(EVENT_FAILED.to_string());
    }
    let backoff = (RETRY_BASE_SECS << (attempts - 1).clamp(0, 5)).min(RETRY_CAP_SECS);
    conn.execute(
        "UPDATE subagent_events SET status = 'pending', next_attempt_at = ?2, last_error = ?3 WHERE id = ?1 AND status = 'claimed'",
        params![event_id, now + backoff, error],
    )?;
    Ok(EVENT_PENDING.to_string())
}

/// Hand a claim back without spending an attempt (the chat was busy).
pub fn defer(conn: &Connection, event_id: i64, now: i64, delay_secs: i64) -> Result<()> {
    conn.execute(
        "UPDATE subagent_events SET status = 'pending', attempts = MAX(attempts - 1, 0), next_attempt_at = ?2
         WHERE id = ?1 AND status = 'claimed'",
        params![event_id, now + delay_secs],
    )?;
    Ok(())
}

/// Follow-up turns already delivered for a chat since `since`; the daily cap
/// reads this so a misbehaving loop cannot burn unbounded model calls.
pub fn followups_since(conn: &Connection, origin: &Origin, since: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM subagent_events e JOIN subagent_runs r ON r.run_id = e.run_id
         WHERE e.status = 'delivered' AND e.delivered_at >= ?1 AND r.platform = ?2 AND r.chat_id = ?3",
        params![since, origin.platform, origin.chat_id],
        |r| r.get(0),
    )?)
}

/// Depth a run spawned from `parent_turn_id` should carry: 0 for a normal
/// turn, parent run's depth + 1 for a follow-up turn.
pub fn followup_depth_for(conn: &Connection, parent_turn_id: Option<&str>) -> Result<i64> {
    let Some(run_id) = parent_turn_id.and_then(|t| t.strip_prefix(FOLLOWUP_TURN_PREFIX)) else {
        return Ok(0);
    };
    let depth: Option<i64> = conn
        .query_row(
            "SELECT followup_depth FROM subagent_runs WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(depth.map_or(1, |d| d + 1))
}

/// Cut `text` to at most `max` bytes on a char boundary.
pub fn preview(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

#[cfg(test)]
mod tests;
