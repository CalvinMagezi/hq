//! Driven-session slots: which observe-only sessions take a slot a finished one gives up, and
//! which driven sessions have sat idle long enough to give theirs back. Kept apart from the
//! registry so the cap rules read in one place.

use super::*;

/// What a session is told when the cap held it back at its start; `promotable` matches on it so
/// only a session the cap held back is ever promoted, never one the owner switched off.
pub const CAP_HOLD_PREFIX: &str = "HQ already drives ";

/// Why Drive was released for a session that sat idle; it is not promotable again by itself.
pub const IDLE_RELEASE_REASON: &str = "The session sat idle or done past agent_host.driver_idle_release_minutes, so its Drive slot was released.";

/// Observe-only sessions the cap held back and that could take a slot, oldest first. The origin
/// is `user` only: a session an MCP client, an ask or a connector started never gains Drive on
/// its own (`harness_session::origin`).
pub fn promotable(conn: &Connection) -> Result<Vec<HarnessSessionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM harness_sessions
         WHERE owner_thread IS NOT NULL AND drive = 0 AND status = 'running' AND origin = ?1
           AND drive_off_reason LIKE ?2
         ORDER BY created_at, id"
    ))?;
    let rows = stmt
        .query_map(
            params![ORIGIN_USER, format!("{CAP_HOLD_PREFIX}%")],
            row_to_session,
        )?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>();
    Ok(rows
        .into_iter()
        .filter(|r| goal_gaps(r).is_empty())
        .collect())
}

/// Give this session a slot if one is free. The cap check inside `request_drive_capped` is the
/// one that counts, so two passes cannot overshoot it.
pub fn promote(conn: &Connection, id: &str, cap: i64) -> Result<bool> {
    if count_driven_running(conn)? >= cap {
        return Ok(false);
    }
    Ok(matches!(
        request_drive_capped(conn, id, true, ACTOR_HQ, Some(cap))?,
        DriveChange::Changed(true)
    ))
}

/// Switch Drive off for driven sessions that report idle or done and have had no event, wake,
/// driver turn or state change for `idle_minutes`. Only the slot goes: the session keeps
/// running and the owner can switch Drive back on. Returns the ids released.
pub fn release_idle(conn: &Connection, idle_minutes: u64) -> Result<Vec<String>> {
    let cutoff = format!("-{idle_minutes} minutes");
    let mut stmt = conn.prepare(
        "SELECT s.id FROM harness_sessions s
         WHERE s.drive = 1 AND s.owner_thread IS NOT NULL AND s.status = 'running'
           AND s.pm_wake IS NULL AND s.last_agent_status IN ('idle', 'done')
           AND s.created_at <= datetime('now', ?1) AND s.updated_at <= datetime('now', ?1)
           AND (s.last_driven_at IS NULL OR s.last_driven_at <= datetime('now', ?1))
           AND NOT EXISTS (SELECT 1 FROM harness_session_events e
                           WHERE e.session_id = s.id AND e.created_at > datetime('now', ?1))",
    )?;
    let ids: Vec<String> = stmt
        .query_map(params![cutoff], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    let mut released = Vec::new();
    for id in ids {
        if stop_drive(conn, &id, ACTOR_GUARD, IDLE_RELEASE_REASON)? {
            released.push(id);
        }
    }
    Ok(released)
}

/// How many sessions sit in each registry status, whatever the list limit shows.
pub fn count_by_status(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn
        .prepare("SELECT status, COUNT(*) FROM harness_sessions GROUP BY status ORDER BY status")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Every running session, oldest first, with no limit.
pub fn list_running(conn: &Connection) -> Result<Vec<HarnessSessionRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM harness_sessions WHERE status = 'running' ORDER BY created_at, id"
    ))?;
    let rows = stmt
        .query_map([], row_to_session)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}

/// Seconds since the newest event of this kind for a session, or `None` when it has none.
pub fn seconds_since_event(conn: &Connection, id: &str, kind: &str) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT CAST((julianday('now') - julianday(MAX(created_at))) * 86400 AS INTEGER)
         FROM harness_session_events WHERE session_id = ?1 AND kind = ?2",
        params![id, kind],
        |r| r.get(0),
    )?)
}
