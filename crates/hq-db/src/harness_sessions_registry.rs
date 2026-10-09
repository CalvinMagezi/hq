//! Registry of long-lived external harness sessions. One row per host agent:
//! which host runs it, liveness fields for the supervisor, a resume token, and
//! an optional owning mission, which is an HQ task.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

pub const STATUS_RUNNING: &str = "running";
pub const STATUS_EXITED: &str = "exited";
pub const STATUS_STOPPED: &str = "stopped";
pub const STATUS_ORPHANED: &str = "orphaned";

#[derive(Debug, Clone, Serialize)]
pub struct HarnessSessionRow {
    pub id: String,
    pub harness: String,
    pub label: String,
    /// host the session runs on (`local` or a configured remote).
    pub host: String,
    /// host agent name; unique among live agents on that host.
    pub agent_name: String,
    pub workspace_id: Option<String>,
    pub pane_id: Option<String>,
    pub cwd: String,
    pub status: String,
    pub resume_token: Option<String>,
    /// Internal id of the HQ task this session works on. Rows from the retired
    /// mission engine may hold ids that match no task; readers treat those as
    /// unlinked.
    pub mission_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Web chat thread watching this session; its events post there.
    pub owner_thread: Option<String>,
    /// Whether that thread drives the session: answers it and approves its prompts.
    pub drive: bool,
    /// Why the owning thread owes the session a look, until the web driver takes it.
    pub pm_wake: Option<String>,
    pub last_driven_at: Option<String>,
    /// host agent status at the supervisor's last sweep.
    pub last_agent_status: Option<String>,
    pub last_seen_at: Option<String>,
    /// What the session is for. HQ drives it only while this and `done_criteria`
    /// pass `harness_drive_gate::gaps`.
    pub goal: Option<String>,
    /// What would be observable when the goal is met. Never proof by itself: a
    /// session that exits or goes idle has not thereby met it.
    pub done_criteria: Option<String>,
    /// Instructions the driver has sent since the user last turned Drive on.
    pub nudges_sent: i64,
    /// `nudges_sent` when the driver last answered a finished turn; `None` after
    /// anyone but the driver sent the session something.
    pub last_wake_nudges: Option<i64>,
    /// Finished turns in a row that showed no new tool activity.
    pub no_progress_streak: i64,
    /// Tool-activity lines the last finished turn left on screen, newline separated.
    pub progress_mark: Option<String>,
    /// Why Drive was last switched off by a guard or the gate, for the Watching panel.
    pub drive_off_reason: Option<String>,
    /// Key presses the driver has sent since the user last turned Drive on.
    pub keys_sent: i64,
    /// `user`, `mcp` or `ask`: see `ORIGIN_*`.
    pub origin: String,
    /// Known harmless prompts the supervisor dismissed since the user last turned Drive on.
    pub dismissals: i64,
    /// Hash of the screen tail at the last dismissal; see `harness_session::dismiss`.
    pub last_dismiss_tail: Option<String>,
    /// The session this one was started for (see `agent_delegate`).
    pub parent_session_id: Option<String>,
    /// 0 for a session a person or HQ started, one more than its parent's otherwise.
    pub spawn_depth: i64,
}

pub const ORIGIN_USER: &str = "user";
pub const ORIGIN_MCP: &str = "mcp";
pub const ORIGIN_ASK: &str = "ask";

const COLS: &str = "id, harness, label, host, agent_name, workspace_id, pane_id, cwd, status, resume_token, mission_id, created_at, updated_at, owner_thread, drive, pm_wake, last_driven_at, last_agent_status, last_seen_at, goal, done_criteria, nudges_sent, last_wake_nudges, no_progress_streak, progress_mark, drive_off_reason, keys_sent, origin, dismissals, last_dismiss_tail, parent_session_id, spawn_depth";

type ChangeHook = Box<dyn Fn(&str) + Send + Sync>;

static CHANGE_HOOKS: std::sync::RwLock<Vec<ChangeHook>> = std::sync::RwLock::new(Vec::new());

/// Run `hook` with the session id after a write a watching chat cares about, so
/// the web server reacts at once. Writes from other processes are not seen.
pub fn on_change(hook: impl Fn(&str) + Send + Sync + 'static) {
    CHANGE_HOOKS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(Box::new(hook));
}

fn changed(id: &str) {
    for hook in CHANGE_HOOKS.read().unwrap_or_else(|e| e.into_inner()).iter() {
        hook(id);
    }
}

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<HarnessSessionRow> {
    Ok(HarnessSessionRow {
        id: row.get(0)?,
        harness: row.get(1)?,
        label: row.get(2)?,
        host: row.get(3)?,
        agent_name: row.get(4)?,
        workspace_id: row.get(5)?,
        pane_id: row.get(6)?,
        cwd: row.get(7)?,
        status: row.get(8)?,
        resume_token: row.get(9)?,
        mission_id: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        owner_thread: row.get(13)?,
        drive: row.get::<_, i64>(14)? != 0,
        pm_wake: row.get(15)?,
        last_driven_at: row.get(16)?,
        last_agent_status: row.get(17)?,
        last_seen_at: row.get(18)?,
        goal: row.get(19)?,
        done_criteria: row.get(20)?,
        nudges_sent: row.get(21)?,
        last_wake_nudges: row.get(22)?,
        no_progress_streak: row.get(23)?,
        progress_mark: row.get(24)?,
        drive_off_reason: row.get(25)?,
        keys_sent: row.get(26)?,
        origin: row.get(27)?,
        dismissals: row.get(28)?,
        last_dismiss_tail: row.get(29)?,
        parent_session_id: row.get(30)?,
        spawn_depth: row.get(31)?,
    })
}

/// Where a session's agent lives on its host.
#[derive(Debug, Clone, Copy)]
pub struct Placement<'a> {
    pub host: &'a str,
    pub agent_name: &'a str,
    pub workspace_id: &'a str,
    pub pane_id: &'a str,
}

pub struct NewSession<'a> {
    pub id: &'a str,
    pub harness: &'a str,
    pub label: &'a str,
    pub cwd: &'a str,
    pub mission_id: Option<&'a str>,
    pub placement: Placement<'a>,
}

pub fn insert(conn: &Connection, s: &NewSession) -> Result<()> {
    // `logfile` predates the host and is NOT NULL; nothing writes a log any more.
    conn.execute(
        "INSERT INTO harness_sessions
             (id, harness, label, host, agent_name, workspace_id, pane_id, cwd, logfile, mission_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '', ?9)",
        params![
            s.id,
            s.harness,
            s.label,
            s.placement.host,
            s.placement.agent_name,
            s.placement.workspace_id,
            s.placement.pane_id,
            s.cwd,
            s.mission_id
        ],
    )?;
    Ok(())
}

/// Point an existing session at a freshly launched agent (resume) and mark it
/// running again. The new agent counts its state changes from scratch, so the
/// alert claim is reset; otherwise its first `done` could lose to the old seq.
pub fn relaunch(conn: &Connection, id: &str, p: &Placement) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions
         SET host = ?1, agent_name = ?2, workspace_id = ?3, pane_id = ?4,
             status = 'running', blocked_notified_seq = NULL, updated_at = datetime('now')
         WHERE id = ?5",
        params![p.host, p.agent_name, p.workspace_id, p.pane_id, id],
    )?;
    changed(id);
    Ok(())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<HarnessSessionRow>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM harness_sessions WHERE id = ?1"),
            params![id],
            row_to_session,
        )
        .optional()?)
}

/// List sessions, optionally filtered by status. Newest first.
pub fn list(
    conn: &Connection,
    status: Option<&str>,
    limit: usize,
) -> Result<Vec<HarnessSessionRow>> {
    let (sql, has_status) = match status {
        Some(_) => (
            format!(
                "SELECT {COLS} FROM harness_sessions WHERE status = ?1 ORDER BY created_at DESC LIMIT ?2"
            ),
            true,
        ),
        None => (
            format!("SELECT {COLS} FROM harness_sessions ORDER BY created_at DESC LIMIT ?1"),
            false,
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = if has_status {
        stmt.query_map(params![status.unwrap(), limit as i64], row_to_session)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map(params![limit as i64], row_to_session)?
            .filter_map(|r| r.ok())
            .collect()
    };
    Ok(rows)
}

/// Every session launched for one mission (an HQ task id), newest first.
pub fn list_for_mission(conn: &Connection, mission_id: &str) -> Result<Vec<HarnessSessionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM harness_sessions WHERE mission_id = ?1 ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map(params![mission_id], row_to_session)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Sessions of a mission still marked running, not counting `except_id`. A
/// session on an unreachable host stays running, so it counts as working.
pub fn count_running_for_mission(conn: &Connection, mission_id: &str, except_id: &str) -> Result<usize> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM harness_sessions WHERE mission_id = ?1 AND status = 'running' AND id <> ?2",
        params![mission_id, except_id],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// Let a web thread watch a session, or stop watching (`None`, which also
/// ends driving and drops any pending wake). Returns whether the session exists.
pub fn set_owner(conn: &Connection, id: &str, thread: Option<&str>) -> Result<bool> {
    let n = conn.execute(
        "UPDATE harness_sessions SET owner_thread = ?1,
             drive = CASE WHEN ?1 IS NULL THEN 0 ELSE drive END,
             pm_wake = CASE WHEN ?1 IS NULL THEN NULL ELSE pm_wake END
         WHERE id = ?2",
        params![thread, id],
    )?;
    if n == 1 {
        changed(id);
    }
    Ok(n == 1)
}

/// A web chat starts watching a session. Only a session no chat watched takes
/// `drive_new`: one taken from another chat starts with Drive off, and a chat
/// watching it again keeps its switch as the user left it. The SET expressions
/// read the row before the update. Returns whether the session exists.
pub fn watch_from_chat(conn: &Connection, id: &str, thread: &str, drive_new: bool) -> Result<bool> {
    let n = conn.execute(
        "UPDATE harness_sessions SET
             drive = CASE WHEN owner_thread = ?1 THEN drive WHEN owner_thread IS NULL THEN ?2 ELSE 0 END,
             owner_thread = ?1
         WHERE id = ?3",
        params![thread, drive_new, id],
    )?;
    if n == 1 {
        changed(id);
    }
    Ok(n == 1)
}

/// Turn driving on or off for a watched session. False when it is not watched.
pub fn set_drive(conn: &Connection, id: &str, drive: bool) -> Result<bool> {
    let n = conn.execute(
        "UPDATE harness_sessions SET drive = ?1, drive_off_reason = NULL
         WHERE id = ?2 AND owner_thread IS NOT NULL",
        params![drive, id],
    )?;
    if n == 1 {
        changed(id);
    }
    Ok(n == 1)
}

/// Who made a drive or goal change, for the audit trail.
pub const ACTOR_USER: &str = "user";
pub const ACTOR_HQ: &str = "hq";
pub const ACTOR_GATE: &str = "gate";
/// A deterministic limit in the driver (budget, no progress, task or session ended).
pub const ACTOR_GUARD: &str = "guard";
/// A change made by an MCP client with no chat, not by the owner.
pub const ACTOR_MCP: &str = "mcp";

pub const EVENT_GOAL_SET: &str = "goal_set";
pub const EVENT_DRIVE_ON: &str = "drive_on";
pub const EVENT_DRIVE_OFF: &str = "drive_off";
pub const EVENT_DRIVE_REFUSED: &str = "drive_refused";
pub const EVENT_ATTACHED: &str = "attached";
pub const EVENT_SENT: &str = "sent";
/// The driver sent the session an instruction (the text is never stored).
pub const EVENT_NUDGE: &str = "nudge";
/// The supervisor dismissed a known harmless prompt (detail names it).
pub const EVENT_PROMPT_DISMISSED: &str = "prompt_dismissed";
/// The dismissal cap was reached; the supervisor stopped dismissing and notified.
pub const EVENT_DISMISS_CAP: &str = "dismiss_cap";

/// Dismissals the supervisor may make in one session before it stops and notifies.
pub const DISMISSAL_CAP: i64 = 5;

/// What a claim on one dismissal got.
#[derive(Debug, PartialEq, Eq)]
pub enum DismissClaim {
    /// Go ahead; the value is how many have now been claimed.
    Granted(i64),
    /// The cap was just exceeded: stop and notify (returned once only).
    CapReached,
    /// Not a driven running session, or the cap was already reported.
    Refused,
}

/// Claim one dismissal in a single conditional write. Counts one past `cap` exactly once, which
/// is the signal to notify, so a harness that keeps showing the prompt cannot loop on it.
pub fn claim_dismissal(conn: &Connection, id: &str, cap: i64) -> Result<DismissClaim> {
    let n = conn.execute(
        "UPDATE harness_sessions SET dismissals = dismissals + 1
         WHERE id = ?1 AND drive = 1 AND owner_thread IS NOT NULL AND status = 'running' AND dismissals <= ?2",
        params![id, cap],
    )?;
    if n == 0 {
        return Ok(DismissClaim::Refused);
    }
    let count: i64 = conn.query_row(
        "SELECT dismissals FROM harness_sessions WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    Ok(if count > cap {
        DismissClaim::CapReached
    } else {
        DismissClaim::Granted(count)
    })
}

/// Give back a claim whose key was never sent.
pub fn refund_dismissal(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions SET dismissals = MAX(dismissals - 1, 0) WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}

pub fn set_dismiss_tail(conn: &Connection, id: &str, hash: &str) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions SET last_dismiss_tail = ?2 WHERE id = ?1",
        params![id, hash],
    )?;
    Ok(())
}

/// Stop dismissing because the last key did nothing: moves the counter past `cap` so every later
/// claim is refused. True exactly once, which is the signal to notify.
pub fn mark_dismiss_stuck(conn: &Connection, id: &str, cap: i64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE harness_sessions SET dismissals = ?2 + 1 WHERE id = ?1 AND dismissals <= ?2",
        params![id, cap],
    )?;
    Ok(n == 1)
}

pub fn count_events(conn: &Connection, id: &str, kind: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM harness_session_events WHERE session_id = ?1 AND kind = ?2",
        params![id, kind],
        |r| r.get(0),
    )?)
}

/// One audit record: what changed and the goal and criteria in force then.
#[derive(Debug, Clone, Serialize)]
pub struct SessionEvent {
    pub kind: String,
    pub actor: String,
    pub goal: Option<String>,
    pub done_criteria: Option<String>,
    pub detail: Option<String>,
    pub created_at: String,
}

/// Append an audit record, snapshotting the session's current goal and criteria.
pub fn record_event(conn: &Connection, id: &str, kind: &str, actor: &str, detail: Option<&str>) -> Result<()> {
    conn.execute(
        "INSERT INTO harness_session_events (session_id, kind, actor, goal, done_criteria, detail)
         SELECT id, ?2, ?3, goal, done_criteria, ?4 FROM harness_sessions WHERE id = ?1",
        params![id, kind, actor, detail],
    )?;
    Ok(())
}

/// The newest audit records of a session, oldest first.
pub fn list_events(conn: &Connection, id: &str, limit: usize) -> Result<Vec<SessionEvent>> {
    let mut stmt = conn.prepare(
        "SELECT kind, actor, goal, done_criteria, detail, created_at FROM
           (SELECT * FROM harness_session_events WHERE session_id = ?1 ORDER BY id DESC LIMIT ?2)
         ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![id, limit as i64], |r| {
            Ok(SessionEvent {
                kind: r.get(0)?,
                actor: r.get(1)?,
                goal: r.get(2)?,
                done_criteria: r.get(3)?,
                detail: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Why the goal and definition of done cannot license driving yet.
pub fn goal_gaps(row: &HarnessSessionRow) -> Vec<String> {
    crate::harness_drive_gate::gaps(row.goal.as_deref(), row.done_criteria.as_deref())
}

fn clean(text: &str) -> Option<String> {
    Some(text.trim().to_string()).filter(|t| !t.is_empty())
}

/// What a goal edit did to driving.
#[derive(Debug, Clone, PartialEq)]
pub struct GoalUpdate {
    /// Drive was on and the new text no longer passes the gate, so it went off.
    pub drive_stopped: bool,
    /// Gaps in the goal now in force; empty when driving is allowed.
    pub gaps: Vec<String>,
}

/// Set the goal and/or definition of done (`None` keeps the stored value, empty
/// text clears it). A change that leaves a driven session failing the gate
/// stops driving. `Ok(None)` when the session does not exist.
pub fn set_goal(
    conn: &Connection,
    id: &str,
    goal: Option<&str>,
    done_criteria: Option<&str>,
    actor: &str,
) -> Result<Option<GoalUpdate>> {
    let Some(row) = get(conn, id)? else { return Ok(None) };
    let goal = goal.map_or(row.goal.clone(), clean);
    let criteria = done_criteria.map_or(row.done_criteria.clone(), clean);
    conn.execute(
        "UPDATE harness_sessions SET goal = ?1, done_criteria = ?2 WHERE id = ?3",
        params![goal, criteria, id],
    )?;
    record_event(conn, id, EVENT_GOAL_SET, actor, None)?;
    let gaps = crate::harness_drive_gate::gaps(goal.as_deref(), criteria.as_deref());
    let drive_stopped = row.drive && !gaps.is_empty();
    if drive_stopped {
        stop_drive(conn, id, ACTOR_GATE, &gaps.join("; "))?;
    }
    changed(id);
    Ok(Some(GoalUpdate { drive_stopped, gaps }))
}

/// What asking for a drive mode did.
#[derive(Debug, Clone, PartialEq)]
pub enum DriveChange {
    NotWatched,
    Changed(bool),
    /// Drive stays off for these reasons.
    Refused(Vec<String>),
}

/// Switch HQ's steering of a watched session. Turning it off always works;
/// turning it on needs a running session and a goal that passes the gate, and
/// otherwise leaves it observation-only. Both outcomes are audited. Never
/// touches the agent itself.
pub fn request_drive(conn: &Connection, id: &str, on: bool, actor: &str) -> Result<DriveChange> {
    request_drive_capped(conn, id, on, actor, None)
}

/// `request_drive` where anyone but the user is held to `cap` running driven sessions.
pub fn request_drive_capped(conn: &Connection, id: &str, on: bool, actor: &str, cap: Option<i64>) -> Result<DriveChange> {
    let Some(row) = get(conn, id)?.filter(|r| r.owner_thread.is_some()) else {
        return Ok(DriveChange::NotWatched);
    };
    let mut blockers = if on { goal_gaps(&row) } else { Vec::new() };
    if on && row.status != STATUS_RUNNING {
        blockers.push(format!("session is {}, not running; resume it first", row.status));
    }
    if let (true, false, Some(cap)) = (on, row.drive, cap)
        && actor != ACTOR_USER
        && count_driven_running(conn)? >= cap
    {
        blockers.push(format!("HQ already drives {cap} running sessions (agent_host.max_driven_sessions); only the user can add another"));
    }
    if !blockers.is_empty() {
        record_event(conn, id, EVENT_DRIVE_REFUSED, actor, Some(&blockers.join("; ")))?;
        if row.drive {
            set_drive(conn, id, false)?;
        }
        return Ok(DriveChange::Refused(blockers));
    }
    if row.drive != on {
        set_drive(conn, id, on)?;
        record_event(conn, id, if on { EVENT_DRIVE_ON } else { EVENT_DRIVE_OFF }, actor, None)?;
    }
    if on && actor == ACTOR_USER {
        // The person who switches Drive on grants a fresh budget; HQ cannot top itself up.
        conn.execute(
            "UPDATE harness_sessions SET nudges_sent = 0, keys_sent = 0, dismissals = 0, last_dismiss_tail = NULL, last_wake_nudges = NULL, no_progress_streak = 0, progress_mark = NULL
             WHERE id = ?1",
            params![id],
        )?;
    }
    Ok(DriveChange::Changed(on))
}

/// Re-check a driven session against the gate, for a goal that changed behind
/// the switch or a row from before the gate existed. Stops driving when it
/// fails. Returns whether the session is still driven.
pub fn enforce_gate(conn: &Connection, id: &str) -> Result<bool> {
    let Some(row) = get(conn, id)?.filter(|r| r.drive) else { return Ok(false) };
    let gaps = goal_gaps(&row);
    if gaps.is_empty() {
        return Ok(true);
    }
    stop_drive(conn, id, ACTOR_GATE, &gaps.join("; "))?;
    Ok(false)
}

/// Switch Drive off and keep `reason` for the panel and the audit trail. A pending
/// wake is left for the driver to answer as a plain update. Returns whether the
/// session was driven.
pub fn stop_drive(conn: &Connection, id: &str, actor: &str, reason: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE harness_sessions SET drive = 0, drive_off_reason = ?2 WHERE id = ?1 AND drive = 1",
        params![id, reason],
    )?;
    if n == 1 {
        record_event(conn, id, EVENT_DRIVE_OFF, actor, Some(reason))?;
        changed(id);
    }
    Ok(n == 1)
}

/// The only two counter columns the nudge statements may interpolate. Anything
/// that reaches those statements as an identifier must come through here.
fn nudge_column(keys: bool) -> &'static str {
    if keys { "keys_sent" } else { "nudges_sent" }
}

/// Claim one driver instruction in a single conditional write, so parallel tool calls cannot
/// overshoot `limit`. Text prompts and key presses have separate counters. False when the
/// session is not driven by `thread` or the allowance is used up.
pub fn reserve_nudge(conn: &Connection, id: &str, thread: &str, keys: bool, limit: i64) -> Result<bool> {
    let column = nudge_column(keys);
    let n = conn.execute(
        &format!(
            "UPDATE harness_sessions SET {column} = {column} + 1
             WHERE id = ?1 AND owner_thread = ?2 AND drive = 1 AND {column} < ?3"
        ),
        params![id, thread, limit],
    )?;
    Ok(n == 1)
}

/// Give back a reservation whose send failed.
pub fn refund_nudge(conn: &Connection, id: &str, keys: bool) -> Result<()> {
    let column = nudge_column(keys);
    conn.execute(&format!("UPDATE harness_sessions SET {column} = MAX({column} - 1, 0) WHERE id = ?1"), params![id])?;
    Ok(())
}

/// Record a send that went through. The driver's was already counted by `reserve_nudge`;
/// any other send re-arms the one-wake-per-nudge rule, so a user steering the session is never ignored.
pub fn note_send(conn: &Connection, id: &str, by_driver: bool, keys: bool) -> Result<()> {
    if by_driver {
        record_event(conn, id, EVENT_NUDGE, ACTOR_HQ, Some(if keys { "keys" } else { "text" }))?;
    } else {
        conn.execute("UPDATE harness_sessions SET last_wake_nudges = NULL WHERE id = ?1", params![id])?;
    }
    Ok(())
}

pub fn set_origin(conn: &Connection, id: &str, origin: &str) -> Result<()> {
    conn.execute("UPDATE harness_sessions SET origin = ?2 WHERE id = ?1", params![id, origin])?;
    Ok(())
}

/// Remember that the driver answered a finished turn at this nudge count.
pub fn set_last_wake_nudges(conn: &Connection, id: &str, nudges: i64) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions SET last_wake_nudges = ?2 WHERE id = ?1",
        params![id, nudges],
    )?;
    Ok(())
}

/// Store the outcome of comparing a finished turn's tool activity with the last one.
pub fn set_progress(conn: &Connection, id: &str, streak: i64, mark: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions SET no_progress_streak = ?2, progress_mark = COALESCE(?3, progress_mark) WHERE id = ?1",
        params![id, streak, mark],
    )?;
    Ok(())
}

/// Running sessions HQ currently drives.
pub fn count_driven_running(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM harness_sessions WHERE drive = 1 AND status = 'running'",
        [],
        |r| r.get(0),
    )?)
}

/// Running sessions started with this origin (`ORIGIN_MCP` or `ORIGIN_ASK`).
pub fn count_running_with_origin(conn: &Connection, origin: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM harness_sessions WHERE status = 'running' AND origin = ?1",
        [origin],
        |r| r.get(0),
    )?)
}

/// Mark that the owning thread owes the session a look, replacing an older
/// reason still pending. False for a session no thread watches.
pub fn set_wake(conn: &Connection, id: &str, reason: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE harness_sessions SET pm_wake = ?1 WHERE id = ?2 AND owner_thread IS NOT NULL",
        params![reason, id],
    )?;
    if n == 1 {
        changed(id);
    }
    Ok(n == 1)
}

/// Take a pending wake, returning whether this caller won it. The reason is
/// part of the test, so a newer wake set meanwhile is not swallowed.
pub fn claim_wake(conn: &Connection, id: &str, reason: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE harness_sessions SET pm_wake = NULL, last_driven_at = datetime('now')
         WHERE id = ?1 AND pm_wake = ?2",
        params![id, reason],
    )?;
    Ok(n == 1)
}

/// A check-in only makes sense while the supervisor still reaches the host; a
/// laptop asleep overnight would otherwise get a useless turn every interval.
const CHECKIN_SEEN_WITHIN: &str = "-5 minutes";

/// Take a due check-in on a driven session with no wake pending.
pub fn claim_checkin(conn: &Connection, id: &str, every_minutes: u64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE harness_sessions SET last_driven_at = datetime('now')
         WHERE id = ?1 AND drive = 1 AND pm_wake IS NULL AND status = 'running'
           AND last_seen_at >= datetime('now', ?3)
           AND (last_driven_at IS NULL OR last_driven_at <= datetime('now', ?2))",
        params![id, format!("-{every_minutes} minutes"), CHECKIN_SEEN_WITHIN],
    )?;
    Ok(n == 1)
}

/// Watched sessions the web driver owes a look: a pending wake, or a driven
/// running session whose check-in is due.
pub fn list_due_for_driver(conn: &Connection, every_minutes: u64) -> Result<Vec<HarnessSessionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM harness_sessions
         WHERE owner_thread IS NOT NULL AND (pm_wake IS NOT NULL OR (drive = 1 AND status = 'running'
           AND last_seen_at >= datetime('now', ?2)
           AND (last_driven_at IS NULL OR last_driven_at <= datetime('now', ?1))))
         ORDER BY created_at"
    ))?;
    let rows = stmt
        .query_map(params![format!("-{every_minutes} minutes"), CHECKIN_SEEN_WITHIN], row_to_session)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Every session a web thread watches, newest first.
pub fn list_for_thread(conn: &Connection, thread: &str) -> Result<Vec<HarnessSessionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM harness_sessions WHERE owner_thread = ?1 ORDER BY created_at DESC"
    ))?;
    let rows = stmt
        .query_map(params![thread], row_to_session)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Record the agent status a sweep saw. Only a change in status counts as a
/// change for watchers; `last_seen_at` moves every sweep.
pub fn set_seen(conn: &Connection, id: &str, agent_status: &str) -> Result<()> {
    let moved = conn.execute(
        "UPDATE harness_sessions SET last_agent_status = ?1
         WHERE id = ?2 AND last_agent_status IS NOT ?1",
        params![agent_status, id],
    )?;
    conn.execute(
        "UPDATE harness_sessions SET last_seen_at = datetime('now') WHERE id = ?1",
        params![id],
    )?;
    if moved == 1 {
        changed(id);
    }
    Ok(())
}

/// Point a session at a mission (an HQ task id). Returns whether the session exists.
pub fn set_mission(conn: &Connection, id: &str, mission_id: &str) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE harness_sessions SET mission_id = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![mission_id, id],
    )?;
    Ok(changed == 1)
}

/// Hides a stopped or exited session from the default list, or brings it back. A running
/// session is refused, so the list never loses something that is still working.
pub fn set_archived(conn: &Connection, id: &str, archived: bool) -> Result<bool> {
    let sql = if archived {
        "UPDATE harness_sessions SET archived_at = datetime('now') WHERE id = ?1 AND status != 'running'"
    } else {
        "UPDATE harness_sessions SET archived_at = NULL WHERE id = ?1"
    };
    Ok(conn.execute(sql, [id])? == 1)
}

/// Ids of the sessions the user archived that are not running again, so a resumed one is listed.
pub fn archived_ids(conn: &Connection) -> Result<std::collections::HashSet<String>> {
    let mut stmt =
        conn.prepare("SELECT id FROM harness_sessions WHERE archived_at IS NOT NULL AND status != 'running'")?;
    let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// Renames a session; the label is what lists show.
pub fn set_label(conn: &Connection, id: &str, label: &str) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE harness_sessions SET label = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![label, id],
    )?;
    Ok(changed == 1)
}

pub fn set_status(conn: &Connection, id: &str, status: &str) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![status, id],
    )?;
    if status != STATUS_RUNNING {
        stop_drive(conn, id, ACTOR_GUARD, SESSION_ENDED)?;
    }
    changed(id);
    Ok(())
}

/// Why Drive goes off when the agent stops or exits: HQ cannot steer what is not running.
pub const SESSION_ENDED: &str = "The session ended, so there is nothing left to drive.";

/// Claim the running -> exited transition, returning whether this caller won it.
///
/// The supervisor sends one completion message per exit. With a plain
/// `set_status` two overlapping sweeps both see `running`, both write `exited`,
/// and the operator gets the message twice. The status test lives inside the
/// UPDATE so the winner is whoever SQLite serializes first.
pub fn set_status_exited_if_running(conn: &Connection, id: &str) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE harness_sessions SET status = 'exited', updated_at = datetime('now')
         WHERE id = ?1 AND status = 'running'",
        params![id],
    )?;
    if changed == 1 {
        stop_drive(conn, id, ACTOR_GUARD, SESSION_ENDED)?;
        self::changed(id);
    }
    Ok(changed == 1)
}

/// Records that `id` was started for `parent`, `depth` levels down.
pub fn set_parent(conn: &Connection, id: &str, parent: &str, depth: i64) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions SET parent_session_id = ?2, spawn_depth = ?3 WHERE id = ?1",
        params![id, parent, depth],
    )?;
    Ok(())
}

/// The sessions started for `parent` that are still running.
pub fn running_children(conn: &Connection, parent: &str) -> Result<Vec<HarnessSessionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM harness_sessions \
         WHERE parent_session_id = ?1 AND status = 'running' ORDER BY created_at"
    ))?;
    let rows = stmt
        .query_map(params![parent], row_to_session)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// How many sessions `parent` started in the last `minutes`, running or not.
pub fn children_started_since(conn: &Connection, parent: &str, minutes: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM harness_sessions \
         WHERE parent_session_id = ?1 AND created_at >= datetime('now', ?2)",
        params![parent, format!("-{minutes} minutes")],
        |r| r.get(0),
    )?)
}

/// Claim the alert for an agent's `state_change_seq`, returning whether this
/// caller won it. The supervisor sweeps every minute while an agent sits blocked
/// or done; the seq only advances when its state changes, so one alert goes out
/// per state, not one per sweep. The column keeps its original `blocked` name.
pub fn claim_state_alert(conn: &Connection, id: &str, seq: u64) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE harness_sessions SET blocked_notified_seq = ?2
         WHERE id = ?1 AND (blocked_notified_seq IS NULL OR blocked_notified_seq < ?2)",
        params![id, seq as i64],
    )?;
    Ok(changed == 1)
}

/// Store the newest screen snapshot for a session.
///
/// `updated_at` deliberately stays put: it means "last state change", and a snapshot written every sweep would make every live
/// session look permanently fresh.
pub fn set_last_snapshot(conn: &Connection, id: &str, snapshot: &str) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions SET last_snapshot = ?1 WHERE id = ?2",
        params![snapshot, id],
    )?;
    Ok(())
}

/// Newest stored screen snapshot, if the supervisor ever caught the session alive.
///
/// Kept off `HarnessSessionRow` on purpose: the row is serialized into
/// `harness_session_list` output 50 at a time, and a 200-line snapshot per row
/// would swamp it.
pub fn last_snapshot(conn: &Connection, id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT last_snapshot FROM harness_sessions WHERE id = ?1",
            params![id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

pub fn set_resume_token(conn: &Connection, id: &str, token: &str) -> Result<()> {
    conn.execute(
        "UPDATE harness_sessions SET resume_token = ?1, updated_at = datetime('now') WHERE id = ?2",
        params![token, id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
