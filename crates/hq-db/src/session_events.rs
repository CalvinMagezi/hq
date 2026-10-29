//! The sessions' event log read as one stream (docs/plans/remote-driven-teams.md, phase 3):
//! `harness_session_events` rows across a set of sessions, after a cursor. The table's
//! autoincrement id is the cursor, so a reader that keeps the last id it saw misses nothing.

use anyhow::Result;
use rusqlite::{Connection, params_from_iter};
use serde::Serialize;

/// Most sessions one read covers.
pub const MAX_SESSIONS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamEvent {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub actor: String,
    pub detail: Option<String>,
    pub created_at: String,
}

fn placeholders(n: usize, from: usize) -> String {
    (0..n)
        .map(|i| format!("?{}", i + from))
        .collect::<Vec<_>>()
        .join(", ")
}

fn read(c: &Connection, sql: &str, args: Vec<rusqlite::types::Value>) -> Result<Vec<StreamEvent>> {
    let mut stmt = c.prepare(sql)?;
    let rows = stmt
        .query_map(params_from_iter(args), |r| {
            Ok(StreamEvent {
                id: r.get(0)?,
                session_id: r.get(1)?,
                kind: r.get(2)?,
                actor: r.get(3)?,
                detail: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Events of `sessions` after `after`, oldest first, at most `limit`.
pub fn after(
    c: &Connection,
    after: i64,
    sessions: &[String],
    limit: usize,
) -> Result<Vec<StreamEvent>> {
    if sessions.is_empty() {
        return Ok(Vec::new());
    }
    let sessions = &sessions[..sessions.len().min(MAX_SESSIONS)];
    let sql = format!(
        "SELECT id, session_id, kind, actor, detail, created_at FROM harness_session_events \
         WHERE id > ?1 AND session_id IN ({}) ORDER BY id LIMIT ?2",
        placeholders(sessions.len(), 3)
    );
    let mut args: Vec<rusqlite::types::Value> = vec![after.into(), (limit as i64).into()];
    args.extend(sessions.iter().map(|s| s.clone().into()));
    read(c, &sql, args)
}

/// The newest `limit` events of `sessions`, oldest first.
pub fn newest(c: &Connection, sessions: &[String], limit: usize) -> Result<Vec<StreamEvent>> {
    if sessions.is_empty() {
        return Ok(Vec::new());
    }
    let sessions = &sessions[..sessions.len().min(MAX_SESSIONS)];
    let sql = format!(
        "SELECT * FROM (SELECT id, session_id, kind, actor, detail, created_at \
         FROM harness_session_events WHERE session_id IN ({}) ORDER BY id DESC LIMIT ?1) ORDER BY id",
        placeholders(sessions.len(), 2)
    );
    let mut args: Vec<rusqlite::types::Value> = vec![(limit as i64).into()];
    args.extend(sessions.iter().map(|s| s.clone().into()));
    read(c, &sql, args)
}

/// The id of the newest event of any session: a cursor that starts from now.
pub fn last_id(c: &Connection) -> Result<i64> {
    Ok(c.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM harness_session_events",
        [],
        |r| r.get(0),
    )?)
}

/// Milliseconds since `session_id`'s newest `blocked` or `question` event, if it has one.
pub fn blocked_for_ms(c: &Connection, session_id: &str) -> Result<Option<i64>> {
    use rusqlite::OptionalExtension;
    Ok(c.query_row(
        "SELECT CAST((julianday('now') - julianday(created_at)) * 86400000 AS INTEGER) \
         FROM harness_session_events WHERE session_id = ?1 AND kind IN ('blocked', 'question') \
         ORDER BY id DESC LIMIT 1",
        [session_id],
        |r| r.get(0),
    )
    .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::harness_sessions_registry as registry;

    fn session(c: &Connection, id: &str) {
        registry::insert(
            c,
            &registry::NewSession {
                id,
                harness: "claude-code",
                label: id,
                cwd: "/w",
                mission_id: None,
                placement: registry::Placement {
                    host: "native",
                    agent_name: id,
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn a_cursor_reads_each_event_once_across_the_sessions_asked_for() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for id in ["hs-a", "hs-b", "hs-c"] {
                session(c, id);
            }
            let start = last_id(c)?;
            registry::record_event(c, "hs-a", "blocked", "supervisor", Some("{}"))?;
            registry::record_event(c, "hs-c", "finished", "supervisor", None)?;
            registry::record_event(c, "hs-b", "exited", "supervisor", None)?;
            let ab = ["hs-a".to_string(), "hs-b".to_string()];
            let first = after(c, start, &ab, 10)?;
            let kinds: Vec<&str> = first.iter().map(|e| e.kind.as_str()).collect();
            assert_eq!(kinds, ["blocked", "exited"], "hs-c is not asked for");
            assert!(after(c, first.last().unwrap().id, &ab, 10)?.is_empty());
            assert_eq!(after(c, start, &ab, 1)?.len(), 1, "limit");
            assert_eq!(newest(c, &ab, 1)?[0].kind, "exited");
            assert!(after(c, start, &[], 10)?.is_empty());
            assert!(blocked_for_ms(c, "hs-a")?.is_some_and(|ms| ms >= 0));
            assert_eq!(blocked_for_ms(c, "hs-b")?, None);
            Ok(())
        })
        .unwrap();
    }
}
