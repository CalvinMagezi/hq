//! Work leases: one agent session holding a task for a stretch of time. The
//! lease is the session lock, the source of time-on-task and the record of which
//! session did the work, for any MCP client that claims a task and for sessions
//! HQ spawns. Only the hash of an external lease's token is stored.
//!
//! An external lease ends at its last heartbeat once it has been silent for the
//! TTL, so a crashed agent costs no phantom time. A spawned session's lease has
//! no TTL: the session registry is authoritative, and the events that move its
//! task open and close the lease.

use super::*;
use sha2::{Digest, Sha256};

/// Recognizable prefix, so a leaked lease token is easy to spot and scan for.
pub const LEASE_TOKEN_PREFIX: &str = "hql_";

pub const END_RELEASED: &str = "released";
pub const END_EXPIRED: &str = "expired";
pub const END_SUPERSEDED: &str = "superseded";
pub const END_SESSION_ENDED: &str = "session_ended";

/// Longest label (actor, harness, host, cwd, branch, session ref) kept.
const MAX_LABEL_CHARS: usize = 120;
/// Longest release summary kept on the task thread.
const MAX_SUMMARY_CHARS: usize = 2000;
/// An actor may start this many leases in `CLAIM_WINDOW_MINUTES`.
pub const CLAIMS_PER_WINDOW: i64 = 30;
const CLAIM_WINDOW_MINUTES: i64 = 10;

/// Statuses a claim may start work from. `complete` is never reopened by a claim.
const CLAIMABLE: &[&str] = &[STATUS_TO_DO, STATUS_BLOCKED, STATUS_READY_FOR_REVIEW];

#[derive(Debug, Clone, Serialize)]
pub struct WorkSession {
    pub id: String,
    pub task_id: String,
    pub actor: String,
    pub harness: String,
    pub external_session_ref: String,
    pub host: String,
    pub cwd: String,
    pub branch: String,
    /// Set for a lease that follows a spawned session.
    pub harness_session_id: Option<String>,
    pub started_at: String,
    pub last_heartbeat_at: String,
    pub ended_at: Option<String>,
    pub end_reason: Option<String>,
    /// Seconds from start to the end, or to the last sign of life while live.
    pub active_seconds: i64,
}

/// What a claimer says about itself. Self-declared: the lease is attribution
/// for agents that behave, not authentication.
#[derive(Debug, Clone, Copy, Default)]
pub struct LeaseIdentity<'a> {
    pub actor: &'a str,
    pub harness: &'a str,
    pub external_session_ref: &'a str,
    pub host: &'a str,
    pub cwd: &'a str,
    pub branch: &'a str,
}

/// A fresh lease. `token` is shown once and never stored in the clear.
#[derive(Debug, Clone, Serialize)]
pub struct Claimed {
    pub session: WorkSession,
    pub token: String,
    pub task: Task,
    /// Whether the claim moved the task into in_progress.
    pub moved: bool,
}

const LEASE_COLS: &str = "id, task_id, actor, harness, external_session_ref, host, cwd, branch, \
     harness_session_id, started_at, last_heartbeat_at, ended_at, end_reason, \
     CAST(strftime('%s', COALESCE(ended_at, CASE WHEN harness_session_id IS NULL \
         THEN last_heartbeat_at ELSE datetime('now') END)) AS INTEGER) \
       - CAST(strftime('%s', started_at) AS INTEGER)";

fn row_to_lease(r: &rusqlite::Row) -> rusqlite::Result<WorkSession> {
    Ok(WorkSession {
        id: r.get(0)?,
        task_id: r.get(1)?,
        actor: r.get(2)?,
        harness: r.get(3)?,
        external_session_ref: r.get(4)?,
        host: r.get(5)?,
        cwd: r.get(6)?,
        branch: r.get(7)?,
        harness_session_id: r.get(8)?,
        started_at: r.get(9)?,
        last_heartbeat_at: r.get(10)?,
        ended_at: r.get(11)?,
        end_reason: r.get(12)?,
        active_seconds: r.get::<_, Option<i64>>(13)?.unwrap_or(0).max(0),
    })
}

/// A label with control characters dropped and its length capped. Lease text is
/// shown to other agents and written to the task thread, which is read back as
/// trusted input, so it is kept short and single-line.
pub fn clean_label(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| !c.is_control() && !is_invisible(*c))
        .take(MAX_LABEL_CHARS)
        .collect();
    kept.trim().to_string()
}

/// Zero-width, bidirectional-override and byte-order characters: they print as
/// nothing or reorder what is around them, so a label could look like another.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Names a claimer may not take, because HQ writes under them: spawned sessions
/// are `hs-...` and the supervisor comments as `harness-session`.
fn is_reserved_actor(actor: &str) -> bool {
    let lower = actor.to_ascii_lowercase();
    lower.starts_with("hs-") || lower == "harness-session" || lower == "unknown"
}

fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn new_token() -> String {
    format!(
        "{LEASE_TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn ttl_expr(ttl_secs: i64) -> String {
    format!("-{ttl_secs} seconds")
}

/// Closes every external lease silent for longer than `ttl_secs`, at its last
/// heartbeat. Returns how many it closed.
pub fn expire_stale_leases(conn: &Connection, ttl_secs: i64) -> Result<usize> {
    let mut closed = conn.execute(
        "UPDATE task_work_sessions SET ended_at = last_heartbeat_at, end_reason = ?1 \
         WHERE ended_at IS NULL AND harness_session_id IS NULL \
           AND last_heartbeat_at < datetime('now', ?2)",
        params![END_EXPIRED, ttl_expr(ttl_secs)],
    )?;
    // A spawned session's lease has no ttl, but a session that is gone from the
    // registry or no longer running cannot still hold a task, whatever event was
    // missed on the way out.
    closed += conn.execute(
        "UPDATE task_work_sessions SET ended_at = datetime('now'), end_reason = ?1 \
         WHERE ended_at IS NULL AND harness_session_id IS NOT NULL \
           AND NOT EXISTS (SELECT 1 FROM harness_sessions s \
                           WHERE s.id = task_work_sessions.harness_session_id AND s.status = 'running')",
        params![END_SESSION_ENDED],
    )?;
    if closed > 0 {
        changed_outside_tx(conn);
    }
    Ok(closed)
}

fn live_on_task(conn: &Connection, task_id: &str) -> Result<Vec<WorkSession>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {LEASE_COLS} FROM task_work_sessions \
         WHERE task_id = ?1 AND ended_at IS NULL ORDER BY started_at"
    ))?;
    let rows = stmt
        .query_map(params![task_id], row_to_lease)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The lease holding a task right now, after closing any that went silent.
pub fn live_lease(conn: &Connection, task_id: &str, ttl_secs: i64) -> Result<Option<WorkSession>> {
    expire_stale_leases(conn, ttl_secs)?;
    Ok(live_on_task(conn, task_id)?.into_iter().next())
}

/// A lease by its token, whether or not it has ended. `None` for an unknown token.
pub fn lease_for_token(conn: &Connection, token: &str) -> Result<Option<WorkSession>> {
    if !token.starts_with(LEASE_TOKEN_PREFIX) {
        return Ok(None);
    }
    Ok(conn
        .query_row(
            &format!("SELECT {LEASE_COLS} FROM task_work_sessions WHERE token_hash = ?1"),
            params![hash_token(token)],
            row_to_lease,
        )
        .optional()?)
}

/// The live lease a token names, or an error that says what to do next.
pub fn require_live_lease(conn: &Connection, token: &str, ttl_secs: i64) -> Result<WorkSession> {
    expire_stale_leases(conn, ttl_secs)?;
    match lease_for_token(conn, token)? {
        None => anyhow::bail!("unknown lease token; claim the task with task_claim"),
        Some(lease) if lease.ended_at.is_some() => anyhow::bail!(
            "lease {} ended ({}); claim the task again with task_claim",
            lease.id,
            lease.end_reason.as_deref().unwrap_or("ended")
        ),
        Some(lease) => Ok(lease),
    }
}

/// Recent leases on a task, newest first, live ones included.
pub fn list_work_sessions(conn: &Connection, task_id: &str, limit: usize) -> Result<Vec<WorkSession>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {LEASE_COLS} FROM task_work_sessions WHERE task_id = ?1 \
         ORDER BY started_at DESC, rowid DESC LIMIT {limit}"
    ))?;
    let rows = stmt
        .query_map(params![task_id], row_to_lease)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn end_lease(conn: &Connection, id: &str, ended_at_sql: &str, reason: &str) -> Result<()> {
    conn.execute(
        &format!("UPDATE task_work_sessions SET ended_at = {ended_at_sql}, end_reason = ?1 WHERE id = ?2"),
        params![reason, id],
    )?;
    Ok(())
}

/// Holding again means the same actor and the same, named, session. Without a
/// session ref two agents using one name cannot tell themselves apart, so
/// neither may replace the other without `takeover`.
fn same_holder(lease: &WorkSession, who: &LeaseIdentity, actor: &str) -> bool {
    !lease.external_session_ref.is_empty()
        && lease.harness_session_id.is_none()
        && lease.actor == actor
        && lease.external_session_ref == clean_label(who.external_session_ref)
}

fn held_message(task: &Task, lease: &WorkSession) -> String {
    format!(
        "{} is held by {} ({}) since {} UTC, last seen {} UTC. Message that session, wait, or \
         pass takeover=true if it is gone.",
        task.display_id,
        lease.actor,
        if lease.harness.is_empty() { "unknown harness" } else { &lease.harness },
        lease.started_at,
        lease.last_heartbeat_at
    )
}

/// The thread says who started work and with which harness, never where: every reader of
/// the thread, the tasks-scoped key included, sees it, while the host, folder and branch stay
/// on the lease for the owner.
fn claimed_comment(who: &LeaseIdentity, actor: &str) -> String {
    let mut text = format!("Work lease started by {actor}");
    let harness = clean_label(who.harness);
    if !harness.is_empty() {
        text.push_str(&format!(", harness {harness}"));
    }
    text.push('.');
    text
}

/// Starts a lease on `task_ref` for `who` and moves the task into in_progress
/// when it is waiting. A task another session holds is refused unless
/// `takeover`, which ends that lease as superseded. Claiming again as the same
/// holder replaces its lease, so an agent that lost its token can resume.
pub fn claim(
    conn: &Connection,
    task_ref: &str,
    who: &LeaseIdentity,
    ttl_secs: i64,
    takeover: bool,
) -> Result<Claimed> {
    let actor = clean_label(who.actor);
    if actor.is_empty() {
        anyhow::bail!("actor is required: name yourself, for example your agent name");
    }
    if is_reserved_actor(&actor) {
        anyhow::bail!("'{actor}' is a name HQ uses for itself; pick your own agent name");
    }
    let out = in_write_tx(conn, |conn| {
        let task = get_task(conn, task_ref)?
            .ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
        if task.status == STATUS_COMPLETE {
            anyhow::bail!("task {} is complete; reopen it before claiming it", task.display_id);
        }
        let recent: i64 = conn.query_row(
            "SELECT COUNT(*) FROM task_work_sessions WHERE actor = ?1 \
             AND started_at >= datetime('now', ?2)",
            params![actor, format!("-{CLAIM_WINDOW_MINUTES} minutes")],
            |r| r.get(0),
        )?;
        if recent >= CLAIMS_PER_WINDOW {
            anyhow::bail!("{actor} started {recent} leases in {CLAIM_WINDOW_MINUTES} minutes; slow down");
        }
        expire_stale_leases(conn, ttl_secs)?;
        for held in live_on_task(conn, &task.id)? {
            if !same_holder(&held, who, &actor) && !takeover {
                anyhow::bail!(held_message(&task, &held));
            }
            end_lease(conn, &held.id, "datetime('now')", END_SUPERSEDED)?;
        }

        let id = format!("ws-{}", uuid::Uuid::new_v4().simple());
        let token = new_token();
        conn.execute(
            "INSERT INTO task_work_sessions \
             (id, task_id, actor, harness, external_session_ref, host, cwd, branch, token_hash) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                task.id,
                actor,
                clean_label(who.harness),
                clean_label(who.external_session_ref),
                clean_label(who.host),
                clean_label(who.cwd),
                clean_label(who.branch),
                hash_token(&token)
            ],
        )?;
        let ctx = WriteCtx { actor: Some(&actor), work_session_id: Some(&id) };
        let moved = CLAIMABLE.contains(&task.status.as_str());
        if moved {
            let patch = TaskPatch { status: Some(STATUS_IN_PROGRESS.to_string()), ..Default::default() };
            update_task_as(conn, &task.id, &patch, Some(&task.status), &ctx)?;
        }
        add_comment(conn, &task.id, &actor, &claimed_comment(who, &actor), None)?;
        let session = conn.query_row(
            &format!("SELECT {LEASE_COLS} FROM task_work_sessions WHERE id = ?1"),
            params![id],
            row_to_lease,
        )?;
        let task = get_task(conn, &task.id)?.ok_or_else(|| anyhow::anyhow!("task vanished while claiming"))?;
        Ok(Claimed { session, token, task, moved })
    })?;
    Ok(out)
}

/// Marks a live lease as seen now. A lease that already went silent past the
/// TTL is closed and refused, so time is never counted across a gap.
pub fn heartbeat(conn: &Connection, token: &str, ttl_secs: i64) -> Result<WorkSession> {
    // Closed in its own commit: the refusal below must not roll the expiry back.
    expire_stale_leases(conn, ttl_secs)?;
    in_write_tx(conn, |conn| {
        let lease = require_live_lease(conn, token, ttl_secs)?;
        conn.execute(
            "UPDATE task_work_sessions SET last_heartbeat_at = datetime('now') WHERE id = ?1",
            params![lease.id],
        )?;
        conn.query_row(
            &format!("SELECT {LEASE_COLS} FROM task_work_sessions WHERE id = ?1"),
            params![lease.id],
            row_to_lease,
        )
        .map_err(Into::into)
    })
}

/// What a release did.
#[derive(Debug, Clone, Serialize)]
pub struct Released {
    pub session: WorkSession,
    pub task: Task,
    /// False when a requested status was not applied because the task had moved
    /// on since an expired lease ended. The summary is still left on the thread.
    pub status_applied: bool,
}

/// Ends the lease a token names. With `status` the task moves there too, under
/// the lease's actor, and a non-empty `summary` is left on the task thread as a
/// quoted block, so what an agent typed never reads as a line HQ wrote. A lease
/// that went silent is ended at its last heartbeat, not now. One that already
/// expired can still report where the work stands, but may change the status
/// only while nobody else holds the task and nothing has moved it since the
/// lease ended: a stale token must not undo or finish another session's work.
pub fn release(
    conn: &Connection,
    token: &str,
    status: Option<&str>,
    summary: &str,
    ttl_secs: i64,
) -> Result<Released> {
    if let Some(status) = status {
        validate_status(status)?;
    }
    in_write_tx(conn, |conn| {
        let lease = lease_for_token(conn, token)?
            .ok_or_else(|| anyhow::anyhow!("unknown lease token"))?;
        let may_change_status = match (&lease.ended_at, lease.end_reason.as_deref()) {
            (None, _) => {
                let silent: bool = conn.query_row(
                    "SELECT harness_session_id IS NULL AND last_heartbeat_at < datetime('now', ?2) \
                     FROM task_work_sessions WHERE id = ?1",
                    params![lease.id, ttl_expr(ttl_secs)],
                    |r| r.get(0),
                )?;
                let ended_at = if silent { "last_heartbeat_at" } else { "datetime('now')" };
                end_lease(conn, &lease.id, ended_at, END_RELEASED)?;
                true
            }
            (Some(ended_at), Some(END_EXPIRED)) => {
                let moved_since: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM task_events WHERE task_id = ?1 AND occurred_at > ?2 \
                     AND COALESCE(work_session_id, '') != ?3",
                    params![lease.task_id, ended_at, lease.id],
                    |r| r.get(0),
                )?;
                moved_since == 0 && live_on_task(conn, &lease.task_id)?.is_empty()
            }
            (Some(_), reason) => anyhow::bail!(
                "lease {} already ended ({})",
                lease.id,
                reason.unwrap_or("ended")
            ),
        };
        let ctx = WriteCtx { actor: Some(&lease.actor), work_session_id: Some(&lease.id) };
        let status_applied = match status {
            Some(status) if may_change_status => {
                let patch = TaskPatch { status: Some(status.to_string()), ..Default::default() };
                update_task_as(conn, &lease.task_id, &patch, None, &ctx)?;
                true
            }
            _ => false,
        };
        add_comment(conn, &lease.task_id, &lease.actor, &release_note(status, status_applied, summary), None)?;
        let session = conn.query_row(
            &format!("SELECT {LEASE_COLS} FROM task_work_sessions WHERE id = ?1"),
            params![lease.id],
            row_to_lease,
        )?;
        let task = get_task(conn, &lease.task_id)?
            .ok_or_else(|| anyhow::anyhow!("task vanished while releasing"))?;
        Ok(Released { session, task, status_applied })
    })
}

/// The thread comment for a release: what HQ did, then the agent's own words
/// quoted line by line so they cannot pass for HQ's.
fn release_note(status: Option<&str>, applied: bool, summary: &str) -> String {
    let head = match (status, applied) {
        (Some(s), true) => format!("Work lease released ({s})."),
        (Some(s), false) => format!(
            "Work lease released. Status {s} was not applied: the task moved on since this lease expired."
        ),
        (None, _) => "Work lease released.".to_string(),
    };
    let quoted: Vec<String> = summary
        .chars()
        .filter(|c| *c == '\n' || (!c.is_control() && !is_invisible(*c)))
        .take(MAX_SUMMARY_CHARS)
        .collect::<String>()
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(|l| format!("> {l}"))
        .collect();
    if quoted.is_empty() { head } else { format!("{head}\n{}", quoted.join("\n")) }
}

/// Opens the lease of a spawned session on its task, replacing any earlier lease
/// the same session held. Never refused: the owner launching a session is not
/// subject to another holder's lock.
pub fn open_for_session(
    conn: &Connection,
    task_id: &str,
    harness_session_id: &str,
    actor: &str,
    harness: &str,
    host: &str,
    cwd: &str,
) -> Result<String> {
    close_for_session(conn, harness_session_id, END_SUPERSEDED)?;
    let id = format!("ws-{}", uuid::Uuid::new_v4().simple());
    conn.execute(
        "INSERT INTO task_work_sessions \
         (id, task_id, actor, harness, host, cwd, harness_session_id, token_hash) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            task_id,
            clean_label(actor),
            clean_label(harness),
            clean_label(host),
            clean_label(cwd),
            harness_session_id,
            hash_token(&new_token())
        ],
    )?;
    changed_outside_tx(conn);
    Ok(id)
}

/// Ends every open lease of a spawned session. Returns how many it ended.
pub fn close_for_session(conn: &Connection, harness_session_id: &str, reason: &str) -> Result<usize> {
    let closed = conn.execute(
        "UPDATE task_work_sessions SET ended_at = datetime('now'), end_reason = ?1 \
         WHERE harness_session_id = ?2 AND ended_at IS NULL",
        params![reason, harness_session_id],
    )?;
    if closed > 0 {
        changed_outside_tx(conn);
    }
    Ok(closed)
}

/// The open lease a spawned session holds, if any.
pub fn lease_for_session(conn: &Connection, harness_session_id: &str) -> Result<Option<WorkSession>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {LEASE_COLS} FROM task_work_sessions \
                 WHERE harness_session_id = ?1 AND ended_at IS NULL"
            ),
            params![harness_session_id],
            row_to_lease,
        )
        .optional()?)
}
