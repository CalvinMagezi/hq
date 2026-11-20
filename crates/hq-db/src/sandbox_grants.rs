//! Sandbox grants a helper asked HQ for (`sql/148_sandbox_grants.sql`). A row is one request and
//! how it was decided. `delta_for_session` is what a launch of that session reads back.

use anyhow::Result;
use hq_core::grants::SandboxDelta;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::Database;

pub const PENDING: &str = "pending";
pub const ACTIVE: &str = "active";
pub const DECLINED: &str = "declined";
pub const REFUSED: &str = "refused";
pub const REVOKED: &str = "revoked";

pub const BY_POLICY: &str = "policy";
pub const BY_OWNER: &str = "owner";
pub const BY_SYSTEM: &str = "system";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Grant {
    pub id: String,
    pub session_id: String,
    pub task_id: Option<String>,
    pub host: String,
    pub repo: String,
    pub capability: String,
    pub resource: String,
    pub state: String,
    pub decided_by: String,
    pub helper_reason: String,
    pub decision_note: String,
    pub requested_at: i64,
    pub decided_at: Option<i64>,
    pub applied_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

/// A request as filed. `state` is pending, active or refused at filing.
#[derive(Debug, Clone)]
pub struct NewGrant<'a> {
    pub session_id: &'a str,
    pub task_id: Option<&'a str>,
    pub host: &'a str,
    pub repo: &'a str,
    pub capability: &'a str,
    pub resource: &'a str,
    pub delta: &'a SandboxDelta,
    pub state: &'a str,
    pub decided_by: &'a str,
    pub helper_reason: &'a str,
    pub decision_note: &'a str,
    pub expires_at: Option<i64>,
}

const COLUMNS: &str = "id, session_id, task_id, host, repo, capability, resource, state, decided_by, \
     helper_reason, decision_note, requested_at, decided_at, applied_at, expires_at, revoked_at";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Grant> {
    Ok(Grant {
        id: r.get(0)?,
        session_id: r.get(1)?,
        task_id: r.get(2)?,
        host: r.get(3)?,
        repo: r.get(4)?,
        capability: r.get(5)?,
        resource: r.get(6)?,
        state: r.get(7)?,
        decided_by: r.get(8)?,
        helper_reason: r.get(9)?,
        decision_note: r.get(10)?,
        requested_at: r.get(11)?,
        decided_at: r.get(12)?,
        applied_at: r.get(13)?,
        expires_at: r.get(14)?,
        revoked_at: r.get(15)?,
    })
}

pub fn file(db: &Database, g: &NewGrant, now: i64) -> Result<String> {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let decided = (g.state != PENDING).then_some(now);
    let applied = (g.state == ACTIVE).then_some(now);
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO sandbox_grants (id, session_id, task_id, host, repo, capability, resource,
                delta_json, state, decided_by, helper_reason, decision_note, requested_at,
                decided_at, applied_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                id,
                g.session_id,
                g.task_id,
                g.host,
                g.repo,
                g.capability,
                g.resource,
                serde_json::to_string(g.delta)?,
                g.state,
                g.decided_by,
                g.helper_reason,
                g.decision_note,
                now,
                decided,
                applied,
                g.expires_at
            ],
        )?;
        Ok(())
    })?;
    Ok(id)
}

pub fn get(db: &Database, id: &str) -> Result<Option<Grant>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!("SELECT {COLUMNS} FROM sandbox_grants WHERE id = ?1"),
            [id],
            row,
        )
        .optional()?)
    })
}

/// Moves a pending request to `state` (active or declined) when the owner answers. Anything
/// not pending is left alone, so a late or repeated answer cannot revive a revoked grant.
pub fn decide(db: &Database, id: &str, state: &str, now: i64) -> Result<bool> {
    let applied = (state == ACTIVE).then_some(now);
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE sandbox_grants SET state = ?2, decided_by = ?3, decided_at = ?4, applied_at = ?5
             WHERE id = ?1 AND state = ?6",
            params![id, state, BY_OWNER, now, applied, PENDING],
        )?;
        Ok(n == 1)
    })
}

pub fn revoke(db: &Database, id: &str, now: i64) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE sandbox_grants SET state = ?2, revoked_at = ?3 WHERE id = ?1 AND state IN (?4, ?5)",
            params![id, REVOKED, now, ACTIVE, PENDING],
        )?;
        Ok(n == 1)
    })
}

pub fn revoke_session(db: &Database, session_id: &str, now: i64) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE sandbox_grants SET state = ?2, revoked_at = ?3 WHERE session_id = ?1 AND state IN (?4, ?5)",
            params![session_id, REVOKED, now, ACTIVE, PENDING],
        )?)
    })
}

/// The `capability` of a row that binds a session to a named access profile.
pub const PROFILE: &str = "profile";

/// The access profile this session opted into, if any: the newest active binding.
pub fn profile_for_session(db: &Database, session_id: &str) -> Result<Option<String>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT resource FROM sandbox_grants
             WHERE session_id = ?1 AND capability = ?2 AND state = ?3
             ORDER BY requested_at DESC, rowid DESC LIMIT 1",
            params![session_id, PROFILE, ACTIVE],
            |r| r.get(0),
        )
        .optional()?)
    })
}

/// A request of this session for exactly this resource that is still waiting or in force, so
/// asking twice never files twice.
pub fn open_for(
    db: &Database,
    session_id: &str,
    capability: &str,
    resource: &str,
) -> Result<Option<Grant>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!(
                "SELECT {COLUMNS} FROM sandbox_grants
                 WHERE session_id = ?1 AND capability = ?2 AND resource = ?3 AND state IN (?4, ?5)
                 ORDER BY requested_at DESC, rowid DESC LIMIT 1"
            ),
            params![session_id, capability, resource, PENDING, ACTIVE],
            row,
        )
        .optional()?)
    })
}

/// Whether the owner already declined this exact request from this session.
pub fn declined_for(
    db: &Database,
    session_id: &str,
    capability: &str,
    resource: &str,
) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM sandbox_grants
             WHERE session_id = ?1 AND capability = ?2 AND resource = ?3 AND state = ?4)",
            params![session_id, capability, resource, DECLINED],
            |r| r.get(0),
        )?)
    })
}

/// Requests of this session still waiting for the owner.
pub fn pending_for_session(db: &Database, session_id: &str) -> Result<Vec<Grant>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {COLUMNS} FROM sandbox_grants WHERE session_id = ?1 AND state = ?2
             ORDER BY requested_at, rowid"
        ))?;
        let rows = stmt.query_map(params![session_id, PENDING], row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Requests this session made, newest first.
pub fn for_session(db: &Database, session_id: &str, limit: usize) -> Result<Vec<Grant>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {COLUMNS} FROM sandbox_grants WHERE session_id = ?1
             ORDER BY requested_at DESC, rowid DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![session_id, limit as i64], row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Everything recent across sessions, for the owner's Access view and `hq grants list`.
pub fn recent(db: &Database, limit: usize) -> Result<Vec<Grant>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {COLUMNS} FROM sandbox_grants ORDER BY requested_at DESC, rowid DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([limit as i64], row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Requests this session made that were declined or refused, for the stop after repeated no's.
pub fn setbacks(db: &Database, session_id: &str) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT COUNT(*) FROM sandbox_grants WHERE session_id = ?1 AND state IN (?2, ?3)",
            params![session_id, DECLINED, REFUSED],
            |r| r.get::<_, i64>(0),
        )? as usize)
    })
}

/// The sandbox change a launch of `session_id` must carry: every active grant that has not
/// expired, merged.
pub fn delta_for_session(db: &Database, session_id: &str, now: i64) -> Result<SandboxDelta> {
    let deltas: Vec<String> = db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT delta_json FROM sandbox_grants
             WHERE session_id = ?1 AND state = ?2 AND (expires_at IS NULL OR expires_at > ?3)
             ORDER BY requested_at, rowid",
        )?;
        let rows = stmt.query_map(params![session_id, ACTIVE, now], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
    })?;
    let mut merged = SandboxDelta::default();
    for text in deltas {
        // A row this build cannot read adds nothing: fail closed.
        let Ok(d) = serde_json::from_str::<SandboxDelta>(&text) else {
            continue;
        };
        merged.writable.extend(d.writable);
        merged.readable.extend(d.readable);
        merged.loopback_ports.extend(d.loopback_ports);
        merged.allow.extend(d.allow);
    }
    merged.writable.sort();
    merged.writable.dedup();
    merged.readable.sort();
    merged.readable.dedup();
    merged.loopback_ports.sort_unstable();
    merged.loopback_ports.dedup();
    merged.allow.sort();
    merged.allow.dedup();
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn delta(path: &str) -> SandboxDelta {
        SandboxDelta {
            writable: vec![PathBuf::from(path)],
            ..Default::default()
        }
    }

    fn new<'a>(session: &'a str, d: &'a SandboxDelta, state: &'a str) -> NewGrant<'a> {
        NewGrant {
            session_id: session,
            task_id: Some("t1"),
            host: "local",
            repo: "/r",
            capability: "fs_write",
            resource: "/r/x",
            delta: d,
            state,
            decided_by: BY_POLICY,
            helper_reason: "build",
            decision_note: "",
            expires_at: None,
        }
    }

    #[test]
    fn only_active_unexpired_grants_of_the_session_reach_its_launch() {
        let db = Database::open_memory().unwrap();
        let (a, b, c) = (delta("/a"), delta("/b"), delta("/c"));
        file(&db, &new("s1", &a, ACTIVE), 10).unwrap();
        file(&db, &new("s1", &b, PENDING), 10).unwrap();
        file(&db, &new("s2", &c, ACTIVE), 10).unwrap();
        let mut expired = new("s1", &c, ACTIVE);
        expired.expires_at = Some(50);
        file(&db, &expired, 10).unwrap();
        let d = delta_for_session(&db, "s1", 100).unwrap();
        assert_eq!(d.writable, vec![PathBuf::from("/a")]);
        assert!(delta_for_session(&db, "nobody", 100).unwrap().is_empty());
    }

    #[test]
    fn an_owner_answer_activates_once_and_cannot_revive_a_revoked_grant() {
        let db = Database::open_memory().unwrap();
        let d = delta("/a");
        let id = file(&db, &new("s1", &d, PENDING), 10).unwrap();
        assert!(decide(&db, &id, ACTIVE, 20).unwrap());
        assert!(
            !decide(&db, &id, ACTIVE, 21).unwrap(),
            "not pending any more"
        );
        assert_eq!(delta_for_session(&db, "s1", 30).unwrap().writable.len(), 1);
        assert!(revoke(&db, &id, 40).unwrap());
        assert!(delta_for_session(&db, "s1", 50).unwrap().is_empty());
        assert!(!decide(&db, &id, ACTIVE, 60).unwrap());
        assert_eq!(get(&db, &id).unwrap().unwrap().state, REVOKED);
    }

    #[test]
    fn a_session_opts_into_one_profile_and_the_newest_binding_wins() {
        let db = Database::open_memory().unwrap();
        let d = SandboxDelta::default();
        let bind = |name: &'static str| NewGrant {
            capability: PROFILE,
            resource: name,
            ..new("s1", &d, ACTIVE)
        };
        assert_eq!(profile_for_session(&db, "s1").unwrap(), None);
        file(&db, &bind("mobile"), 1).unwrap();
        file(&db, &bind("web"), 2).unwrap();
        assert_eq!(
            profile_for_session(&db, "s1").unwrap().as_deref(),
            Some("web")
        );
        assert_eq!(profile_for_session(&db, "s2").unwrap(), None);
    }

    #[test]
    fn revoking_a_session_clears_every_live_grant_and_counts_setbacks() {
        let db = Database::open_memory().unwrap();
        let d = delta("/a");
        file(&db, &new("s1", &d, ACTIVE), 1).unwrap();
        file(&db, &new("s1", &d, PENDING), 2).unwrap();
        file(&db, &new("s1", &d, REFUSED), 3).unwrap();
        file(&db, &new("s1", &d, DECLINED), 4).unwrap();
        assert_eq!(revoke_session(&db, "s1", 9).unwrap(), 2);
        assert_eq!(setbacks(&db, "s1").unwrap(), 2);
        assert_eq!(for_session(&db, "s1", 10).unwrap().len(), 4);
    }
}
