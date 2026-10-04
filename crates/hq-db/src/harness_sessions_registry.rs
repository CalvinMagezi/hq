//! Registry of long-lived external harness sessions. One row per Herdr agent:
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
    /// Herdr host the session runs on (`local` or a configured remote).
    pub host: String,
    /// Herdr agent name; unique among live agents on that host.
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
    /// Herdr agent status at the supervisor's last sweep.
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
}

pub const ORIGIN_USER: &str = "user";
pub const ORIGIN_MCP: &str = "mcp";
pub const ORIGIN_ASK: &str = "ask";

const COLS: &str = "id, harness, label, host, agent_name, workspace_id, pane_id, cwd, status, resume_token, mission_id, created_at, updated_at, owner_thread, drive, pm_wake, last_driven_at, last_agent_status, last_seen_at, goal, done_criteria, nudges_sent, last_wake_nudges, no_progress_streak, progress_mark, drive_off_reason, keys_sent, origin, dismissals, last_dismiss_tail";

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
    // `logfile` predates Herdr and is NOT NULL; nothing writes a log any more.
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
        blockers.push(format!("HQ already drives {cap} running sessions (herdr.max_driven_sessions); only the user can add another"));
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
mod tests {
    #[test]
    fn nudge_column_is_one_of_two_schema_columns() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        for keys in [false, true] {
            let column = nudge_column(keys);
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM pragma_table_info('harness_sessions') WHERE name = ?1",
                    [column],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(exists, "{column} is not a harness_sessions column");
        }
        assert_ne!(nudge_column(true), nudge_column(false));
    }

    use super::*;
    use crate::pool::Database;

    fn add(c: &Connection, id: &str, harness: &str) -> Result<()> {
        insert(
            c,
            &NewSession {
                id,
                harness,
                label: "auth refactor",
                cwd: "/tmp",
                mission_id: None,
                placement: Placement {
                    host: "local",
                    agent_name: id,
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
            },
        )
    }

    const GOAL: &str = "Add rate limiting to the login endpoint";
    const DONE: &str = "Login returns 429 after 5 failed attempts and cargo test passes";

    fn watched(c: &Connection, id: &str) -> Result<()> {
        add(c, id, "pi")?;
        watch_from_chat(c, id, "th-1", false)?;
        Ok(())
    }

    #[test]
    fn drive_needs_a_goal_that_passes_the_gate_and_refusals_are_audited() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-g")?;
            assert_eq!(request_drive(c, "hs-missing", true, ACTOR_USER)?, DriveChange::NotWatched);

            let DriveChange::Refused(gaps) = request_drive(c, "hs-g", true, ACTOR_USER)? else {
                panic!("a session with no goal must not be driven");
            };
            assert!(gaps.iter().any(|g| g.starts_with("goal is missing")), "{gaps:?}");
            assert!(!get(c, "hs-g")?.unwrap().drive);

            set_goal(c, "hs-g", Some("TBD"), Some("when done"), ACTOR_USER)?;
            assert!(matches!(request_drive(c, "hs-g", true, ACTOR_HQ)?, DriveChange::Refused(_)), "ambiguous");

            set_goal(c, "hs-g", Some(GOAL), Some(DONE), ACTOR_USER)?;
            assert_eq!(request_drive(c, "hs-g", true, ACTOR_USER)?, DriveChange::Changed(true));
            assert!(get(c, "hs-g")?.unwrap().drive);

            let kinds: Vec<String> = list_events(c, "hs-g", 20)?.into_iter().map(|e| e.kind).collect();
            assert_eq!(
                kinds,
                [EVENT_DRIVE_REFUSED, EVENT_GOAL_SET, EVENT_DRIVE_REFUSED, EVENT_GOAL_SET, EVENT_DRIVE_ON]
            );
            let last = list_events(c, "hs-g", 1)?.pop().unwrap();
            assert_eq!(last.goal.as_deref(), Some(GOAL), "the audit record carries the goal in force");
            assert_eq!(last.done_criteria.as_deref(), Some(DONE));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_changed_goal_that_fails_the_gate_stops_driving() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-c")?;
            set_goal(c, "hs-c", Some(GOAL), Some(DONE), ACTOR_USER)?;
            request_drive(c, "hs-c", true, ACTOR_USER)?;

            let same = set_goal(c, "hs-c", Some("Add rate limiting to the signup endpoint too"), None, ACTOR_USER)?.unwrap();
            assert!(!same.drive_stopped && same.gaps.is_empty());
            assert!(get(c, "hs-c")?.unwrap().drive, "a still-valid edit keeps driving");

            let update = set_goal(c, "hs-c", None, Some("done"), ACTOR_USER)?.unwrap();
            assert!(update.drive_stopped, "{update:?}");
            assert!(!get(c, "hs-c")?.unwrap().drive);
            let last = list_events(c, "hs-c", 1)?.pop().unwrap();
            assert_eq!((last.kind.as_str(), last.actor.as_str()), (EVENT_DRIVE_OFF, ACTOR_GATE));
            assert_eq!(set_goal(c, "hs-nope", Some(GOAL), None, ACTOR_USER)?, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn enforce_gate_stops_a_driven_row_that_predates_the_gate() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-old")?;
            set_drive(c, "hs-old", true)?;
            assert!(!enforce_gate(c, "hs-old")?);
            assert!(!get(c, "hs-old")?.unwrap().drive);

            watched(c, "hs-ok")?;
            set_goal(c, "hs-ok", Some(GOAL), Some(DONE), ACTOR_USER)?;
            set_drive(c, "hs-ok", true)?;
            assert!(enforce_gate(c, "hs-ok")?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn drive_off_always_works_and_an_ended_session_cannot_be_driven() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-e")?;
            set_goal(c, "hs-e", Some(GOAL), Some(DONE), ACTOR_USER)?;
            request_drive(c, "hs-e", true, ACTOR_USER)?;
            set_status(c, "hs-e", STATUS_EXITED)?;

            assert_eq!(request_drive(c, "hs-e", false, ACTOR_USER)?, DriveChange::Changed(false));
            let DriveChange::Refused(why) = request_drive(c, "hs-e", true, ACTOR_USER)? else {
                panic!("an exited session must not be driven");
            };
            assert!(why[0].contains("not running"), "{why:?}");

            relaunch(c, "hs-e", &Placement { host: "local", agent_name: "hs-e", workspace_id: "w2", pane_id: "w2:p1" })?;
            assert_eq!(request_drive(c, "hs-e", true, ACTOR_USER)?, DriveChange::Changed(true), "goal survives a resume");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn crud_roundtrip() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            add(c, "hs-1", "claude-code")?;
            let s = get(c, "hs-1")?.unwrap();
            assert_eq!(s.harness, "claude-code");
            assert_eq!(s.status, STATUS_RUNNING);
            assert_eq!(s.host, "local");
            assert_eq!(s.pane_id.as_deref(), Some("w1:p1"));

            set_resume_token(c, "hs-1", "sess-abc")?;
            set_status(c, "hs-1", STATUS_EXITED)?;
            let s = get(c, "hs-1")?.unwrap();
            assert_eq!(s.resume_token.as_deref(), Some("sess-abc"));
            assert_eq!(s.status, STATUS_EXITED);

            assert_eq!(list(c, Some(STATUS_EXITED), 10)?.len(), 1);
            assert_eq!(list(c, Some(STATUS_RUNNING), 10)?.len(), 0);
            assert_eq!(list(c, None, 10)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn mission_queries_see_only_that_missions_sessions() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for (id, mission) in [("hs-a", Some("tk-1")), ("hs-b", Some("tk-1")), ("hs-c", None)] {
                insert(
                    c,
                    &NewSession {
                        id,
                        harness: "pi",
                        label: "",
                        cwd: "/tmp",
                        mission_id: mission,
                        placement: Placement {
                            host: "local",
                            agent_name: id,
                            workspace_id: "w1",
                            pane_id: "w1:p1",
                        },
                    },
                )?;
            }
            assert_eq!(list_for_mission(c, "tk-1")?.len(), 2);
            assert_eq!(count_running_for_mission(c, "tk-1", "hs-a")?, 1);
            set_status(c, "hs-b", STATUS_EXITED)?;
            assert_eq!(count_running_for_mission(c, "tk-1", "hs-a")?, 0);
            assert!(list_for_mission(c, "tk-other")?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_wake_needs_a_watching_thread_and_is_claimed_once() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            add(c, "hs-w", "pi")?;
            assert!(!set_wake(c, "hs-w", "finished")?, "no thread watches it yet");
            assert!(!set_drive(c, "hs-w", true)?);
            assert!(set_owner(c, "hs-w", Some("th-1"))?);
            assert!(set_wake(c, "hs-w", "blocked")?);
            assert!(set_wake(c, "hs-w", "finished")?);
            assert!(!claim_wake(c, "hs-w", "blocked")?, "a newer wake replaced it");
            assert_eq!(list_due_for_driver(c, 30)?.len(), 1);
            assert!(claim_wake(c, "hs-w", "finished")?);
            assert!(!claim_wake(c, "hs-w", "finished")?);
            assert!(list_due_for_driver(c, 30)?.is_empty(), "not driven, nothing due");
            assert_eq!(list_for_thread(c, "th-1")?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn only_a_session_no_chat_watched_takes_the_new_watch_drive() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let drive = |c: &Connection, id: &str| get(c, id).map(|s| s.unwrap().drive);
            assert!(!watch_from_chat(c, "hs-missing", "th-1", true)?);

            add(c, "hs-new", "pi")?;
            assert!(watch_from_chat(c, "hs-new", "th-1", true)?);
            assert!(drive(c, "hs-new")?, "a new watch drives by default");

            add(c, "hs-optout", "pi")?;
            watch_from_chat(c, "hs-optout", "th-1", false)?;
            assert!(!drive(c, "hs-optout")?, "drive=false opts out");

            add(c, "hs-off", "pi")?;
            set_owner(c, "hs-off", Some("th-1"))?;
            watch_from_chat(c, "hs-off", "th-1", true)?;
            assert!(!drive(c, "hs-off")?, "watching again never turns the user's switch on");
            set_drive(c, "hs-off", true)?;
            watch_from_chat(c, "hs-off", "th-1", false)?;
            assert!(drive(c, "hs-off")?, "nor off: the switch stays where the user left it");

            watch_from_chat(c, "hs-off", "th-2", true)?;
            let moved = get(c, "hs-off")?.unwrap();
            assert_eq!(moved.owner_thread.as_deref(), Some("th-2"));
            assert!(!moved.drive, "a session taken from another chat starts with Drive off");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_driven_session_checks_in_when_due_and_unwatching_stops_it() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            add(c, "hs-d", "pi")?;
            set_owner(c, "hs-d", Some("th-1"))?;
            assert!(set_drive(c, "hs-d", true)?);
            assert!(list_due_for_driver(c, 30)?.is_empty(), "never seen by a sweep: host unknown");
            set_seen(c, "hs-d", "working")?;
            assert_eq!(list_due_for_driver(c, 30)?.len(), 1, "never driven: due at once");
            assert!(claim_checkin(c, "hs-d", 30)?);
            assert!(!claim_checkin(c, "hs-d", 30)?);
            assert!(list_due_for_driver(c, 30)?.is_empty());
            c.execute("UPDATE harness_sessions SET last_driven_at = datetime('now', '-31 minutes') WHERE id = 'hs-d'", [])?;
            assert_eq!(list_due_for_driver(c, 30)?.len(), 1);

            c.execute("UPDATE harness_sessions SET last_seen_at = datetime('now', '-2 hours') WHERE id = 'hs-d'", [])?;
            assert!(list_due_for_driver(c, 30)?.is_empty(), "an unreachable host gets no check-in");
            assert!(!claim_checkin(c, "hs-d", 30)?);
            set_wake(c, "hs-d", "exited")?;
            assert_eq!(list_due_for_driver(c, 30)?.len(), 1, "a wake is due whatever the host");
            claim_wake(c, "hs-d", "exited")?;

            set_wake(c, "hs-d", "finished")?;
            set_owner(c, "hs-d", None)?;
            let s = get(c, "hs-d")?.unwrap();
            assert!(!s.drive && s.pm_wake.is_none() && s.owner_thread.is_none());
            assert!(list_due_for_driver(c, 30)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn seen_status_is_stored_and_only_a_change_notifies() {
        let db = Database::open_memory().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = seen.clone();
        on_change(move |id| {
            if id == "hs-seen" {
                sink.lock().unwrap().push(id.to_string());
            }
        });
        db.with_conn(|c| {
            add(c, "hs-seen", "pi")?;
            set_seen(c, "hs-seen", "working")?;
            set_seen(c, "hs-seen", "working")?;
            set_seen(c, "hs-seen", "done")?;
            let s = get(c, "hs-seen")?.unwrap();
            assert_eq!(s.last_agent_status.as_deref(), Some("done"));
            assert!(s.last_seen_at.is_some());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_relaunch_resets_the_alert_claim_for_the_new_agent() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            add(c, "hs-re", "pi")?;
            assert!(claim_state_alert(c, "hs-re", 9)?);
            set_status(c, "hs-re", STATUS_EXITED)?;
            relaunch(c, "hs-re", &Placement { host: "local", agent_name: "hs-re", workspace_id: "w2", pane_id: "w2:p1" })?;
            assert!(claim_state_alert(c, "hs-re", 2)?, "the new agent's first state alerts");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn relaunch_moves_the_session_and_marks_it_running() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            add(c, "hs-r", "pi")?;
            set_status(c, "hs-r", STATUS_EXITED)?;
            relaunch(
                c,
                "hs-r",
                &Placement {
                    host: "laptop",
                    agent_name: "hs-r",
                    workspace_id: "w7",
                    pane_id: "w7:p1",
                },
            )?;
            let s = get(c, "hs-r")?.unwrap();
            assert_eq!(s.status, STATUS_RUNNING);
            assert_eq!(s.host, "laptop");
            assert_eq!(s.workspace_id.as_deref(), Some("w7"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn exit_claim_is_won_once() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            add(c, "hs-2", "pi")?;
            assert!(set_status_exited_if_running(c, "hs-2")?);
            assert!(!set_status_exited_if_running(c, "hs-2")?);
            assert!(!set_status_exited_if_running(c, "hs-missing")?);
            assert_eq!(get(c, "hs-2")?.unwrap().status, STATUS_EXITED);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_state_alert_is_claimed_once_per_state_change() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            add(c, "hs-b", "pi")?;
            assert!(claim_state_alert(c, "hs-b", 5)?);
            assert!(!claim_state_alert(c, "hs-b", 5)?);
            assert!(!claim_state_alert(c, "hs-b", 4)?);
            assert!(claim_state_alert(c, "hs-b", 9)?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn snapshot_roundtrips_without_touching_updated_at() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            add(c, "hs-3", "pi")?;
            assert_eq!(last_snapshot(c, "hs-3")?, None);
            c.execute(
                "UPDATE harness_sessions SET updated_at = '2020-01-01 00:00:00' WHERE id = 'hs-3'",
                [],
            )?;

            set_last_snapshot(c, "hs-3", "pane text")?;
            assert_eq!(last_snapshot(c, "hs-3")?.as_deref(), Some("pane text"));
            assert_eq!(get(c, "hs-3")?.unwrap().updated_at, "2020-01-01 00:00:00");
            assert_eq!(last_snapshot(c, "hs-missing")?, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_herdr_migration_orphans_tmux_era_running_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../sql/031_harness_sessions.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/036_harness_session_snapshot.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO harness_sessions (id, harness, tmux_session, cwd, logfile) VALUES ('old', 'pi', 'hq-old', '/tmp', '/l')",
            [],
        )
        .unwrap();
        conn.execute_batch(include_str!("../sql/043_harness_sessions_herdr.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/061_harness_session_driver.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/064_harness_session_goal.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/070_harness_session_drive_guards.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/071_harness_session_dismissals.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../sql/072_harness_session_dismiss_tail.sql"))
            .unwrap();
        let s = get(&conn, "old").unwrap().unwrap();
        assert_eq!(s.status, STATUS_ORPHANED);
        assert_eq!(s.agent_name, "hq-old");
        assert_eq!(s.host, "local");
    }

    #[test]
    fn a_guard_stop_keeps_its_reason_until_someone_switches_drive_again() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-g")?;
            set_goal(c, "hs-g", Some(GOAL), Some(DONE), ACTOR_USER)?;
            request_drive(c, "hs-g", true, ACTOR_USER)?;
            assert!(stop_drive(c, "hs-g", ACTOR_GUARD, "budget used")?);
            assert!(
                !stop_drive(c, "hs-g", ACTOR_GUARD, "again")?,
                "only a driven session is stopped, once"
            );
            let row = get(c, "hs-g")?.unwrap();
            assert!(!row.drive);
            assert_eq!(row.drive_off_reason.as_deref(), Some("budget used"));
            let events = list_events(c, "hs-g", 10)?;
            let last = events.last().unwrap();
            assert_eq!(
                (
                    last.kind.as_str(),
                    last.actor.as_str(),
                    last.detail.as_deref()
                ),
                (EVENT_DRIVE_OFF, ACTOR_GUARD, Some("budget used"))
            );
            request_drive(c, "hs-g", true, ACTOR_USER)?;
            assert!(
                get(c, "hs-g")?.unwrap().drive_off_reason.is_none(),
                "switching Drive on clears the reason"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn only_the_user_switching_drive_on_refills_the_budget() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-n")?;
            set_goal(c, "hs-n", Some(GOAL), Some(DONE), ACTOR_USER)?;
            request_drive(c, "hs-n", true, ACTOR_USER)?;
            for _ in 0..2 {
                assert!(reserve_nudge(c, "hs-n", "th-1", false, 8)?);
                note_send(c, "hs-n", true, false)?;
            }
            assert!(reserve_nudge(c, "hs-n", "th-1", true, 8)?);
            set_progress(c, "hs-n", 2, Some("Bash(cargo test)"))?;
            request_drive(c, "hs-n", false, ACTOR_USER)?;
            request_drive(c, "hs-n", true, ACTOR_HQ)?;
            let row = get(c, "hs-n")?.unwrap();
            assert_eq!(
                (row.nudges_sent, row.no_progress_streak),
                (2, 2),
                "HQ cannot top itself up"
            );
            request_drive(c, "hs-n", false, ACTOR_USER)?;
            request_drive(c, "hs-n", true, ACTOR_USER)?;
            let row = get(c, "hs-n")?.unwrap();
            assert_eq!(
                (row.nudges_sent, row.no_progress_streak, row.progress_mark),
                (0, 0, None)
            );
            let nudges = list_events(c, "hs-n", 50)?
                .iter()
                .filter(|e| e.kind == EVENT_NUDGE)
                .count();
            assert_eq!(
                nudges, 2,
                "every driver instruction is an event without its text"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_gate_stopping_drive_leaves_a_reason_for_the_panel() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-q")?;
            set_goal(c, "hs-q", Some(GOAL), Some(DONE), ACTOR_USER)?;
            request_drive(c, "hs-q", true, ACTOR_USER)?;
            set_goal(c, "hs-q", None, Some("done"), ACTOR_USER)?;
            let row = get(c, "hs-q")?.unwrap();
            assert!(!row.drive);
            assert!(row.drive_off_reason.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_reservation_is_atomic_per_counter_and_only_for_the_driving_chat() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-a")?;
            set_goal(c, "hs-a", Some(GOAL), Some(DONE), ACTOR_USER)?;
            request_drive(c, "hs-a", true, ACTOR_USER)?;
            assert!(reserve_nudge(c, "hs-a", "th-1", false, 2)?);
            assert!(reserve_nudge(c, "hs-a", "th-1", false, 2)?);
            assert!(!reserve_nudge(c, "hs-a", "th-1", false, 2)?, "the limit holds, not one more");
            assert!(reserve_nudge(c, "hs-a", "th-1", true, 2)?, "keys have their own allowance");
            assert!(!reserve_nudge(c, "hs-a", "th-other", true, 9)?, "not the chat that drives it");
            refund_nudge(c, "hs-a", false)?;
            assert!(reserve_nudge(c, "hs-a", "th-1", false, 2)?, "a failed send gives its slot back");
            stop_drive(c, "hs-a", ACTOR_GUARD, "x")?;
            assert!(!reserve_nudge(c, "hs-a", "th-1", true, 9)?, "Drive is off");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_non_user_cannot_drive_past_the_cap_but_the_user_can() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for id in ["hs-1", "hs-2"] {
                watched(c, id)?;
                set_goal(c, id, Some(GOAL), Some(DONE), ACTOR_USER)?;
            }
            assert_eq!(request_drive_capped(c, "hs-1", true, ACTOR_HQ, Some(1))?, DriveChange::Changed(true));
            assert!(matches!(request_drive_capped(c, "hs-2", true, ACTOR_HQ, Some(1))?, DriveChange::Refused(_)));
            assert_eq!(request_drive_capped(c, "hs-2", true, ACTOR_USER, Some(1))?, DriveChange::Changed(true));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_caps_count_running_driven_sessions_and_sessions_by_origin() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            watched(c, "hs-1")?;
            watched(c, "hs-2")?;
            set_goal(c, "hs-1", Some(GOAL), Some(DONE), ACTOR_USER)?;
            request_drive(c, "hs-1", true, ACTOR_USER)?;
            assert_eq!(count_driven_running(c)?, 1);
            assert_eq!(count_running_with_origin(c, ORIGIN_ASK)?, 0);
            set_origin(c, "hs-2", ORIGIN_ASK)?;
            set_origin(c, "hs-1", ORIGIN_MCP)?;
            assert_eq!((count_running_with_origin(c, ORIGIN_ASK)?, count_running_with_origin(c, ORIGIN_MCP)?), (1, 1));
            set_status(c, "hs-1", STATUS_STOPPED)?;
            assert_eq!(count_driven_running(c)?, 0);
            assert_eq!(count_running_with_origin(c, ORIGIN_MCP)?, 0);
            Ok(())
        })
        .unwrap();
    }
}
