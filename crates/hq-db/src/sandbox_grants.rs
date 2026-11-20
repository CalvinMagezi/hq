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
pub const EXPIRED: &str = "expired";

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
    /// `session`, `task` or `repo`: who a row applies to. Task and repo rows are the owner's
    /// standing rules; a session row is one request's own grant.
    pub scope: String,
}

/// Prefix of the `session_id` column on a standing rule for every session in a repository.
pub const REPO_SCOPE: &str = "repo:";

fn scope_of(session_id: &str) -> &'static str {
    if session_id.starts_with(REPO_SCOPE) {
        "repo"
    } else if session_id.starts_with(TASK_SCOPE) {
        "task"
    } else {
        "session"
    }
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
        scope: scope_of(&r.get::<_, String>(1)?).to_string(),
    })
}

pub fn file(db: &Database, g: &NewGrant, now: i64) -> Result<String> {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let decided = (g.state != PENDING).then_some(now);
    let applied = (g.state == ACTIVE).then_some(now);
    // A grant that takes effect now needs its session restarted; a profile binding applies at launch.
    let due = g.state == ACTIVE && g.capability != PROFILE;
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO sandbox_grants (id, session_id, task_id, host, repo, capability, resource,
                delta_json, state, decided_by, helper_reason, decision_note, requested_at,
                decided_at, applied_at, expires_at, restart_due)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                g.expires_at,
                due
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

/// Moves a pending request to `state` (active or declined) when the owner answers. An approval
/// runs until `expires_at`, counted from now, and makes a restart due. Anything not pending is
/// left alone, so a late or repeated answer cannot revive a revoked or expired grant.
pub fn decide(
    db: &Database,
    id: &str,
    state: &str,
    now: i64,
    expires_at: Option<i64>,
) -> Result<bool> {
    let active = state == ACTIVE;
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE sandbox_grants SET state = ?2, decided_by = ?3, decided_at = ?4,
                applied_at = ?5, expires_at = CASE WHEN ?7 THEN ?8 ELSE expires_at END,
                restart_due = ?7
             WHERE id = ?1 AND state = ?6 AND (expires_at IS NULL OR expires_at > ?4)",
            params![
                id,
                state,
                BY_OWNER,
                now,
                active.then_some(now),
                PENDING,
                active,
                expires_at
            ],
        )?;
        Ok(n == 1)
    })
}

/// Marks every request past its time as expired, so a waiting one stops counting and a lapsed
/// grant stops answering "in force".
pub fn expire_stale(db: &Database, now: i64) -> Result<usize> {
    db.with_conn(|c| {
        Ok(c.execute(
            "UPDATE sandbox_grants SET state = ?1
             WHERE state IN (?2, ?3) AND capability <> ?4 AND expires_at IS NOT NULL AND expires_at <= ?5",
            params![EXPIRED, PENDING, ACTIVE, PROFILE, now],
        )?)
    })
}

/// Whether a grant took effect that this session has not been restarted to carry.
pub fn restart_due(db: &Database, session_id: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM sandbox_grants
             WHERE session_id = ?1 AND state = ?2 AND restart_due = 1)",
            params![session_id, ACTIVE],
            |r| r.get(0),
        )?)
    })
}

pub fn clear_restart_due(db: &Database, session_id: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE sandbox_grants SET restart_due = 0 WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(())
    })
}

/// The `decision_note` a request approved under standing access carries: which rule approved it.
pub fn rule_note(rule_id: &str) -> String {
    format!("rule:{rule_id}")
}

/// Revokes a request or a standing rule. Revoking a rule also revokes what it approved, so the
/// access stops at the next launch instead of lingering for the life of those grants.
pub fn revoke(db: &Database, id: &str, now: i64) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE sandbox_grants SET state = ?2, revoked_at = ?3 WHERE id = ?1 AND state IN (?4, ?5)",
            params![id, REVOKED, now, ACTIVE, PENDING],
        )?;
        if n == 1 {
            c.execute(
                "UPDATE sandbox_grants SET state = ?2, revoked_at = ?3 WHERE decision_note = ?1 AND state IN (?4, ?5)",
                params![rule_note(id), REVOKED, now, ACTIVE, PENDING],
            )?;
        }
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

/// Prefix of the `session_id` column on a binding the owner made for a whole task.
pub const TASK_SCOPE: &str = "task:";

/// The access profile this session opted into, if any: the newest active binding the owner made
/// for the session itself or for its task.
pub fn profile_for(
    db: &Database,
    session_id: &str,
    task_id: Option<&str>,
) -> Result<Option<String>> {
    let task_scope = task_id
        .map(|t| format!("{TASK_SCOPE}{t}"))
        .unwrap_or_default();
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT resource FROM sandbox_grants
             WHERE session_id IN (?1, ?2) AND capability = ?3 AND state = ?4
             ORDER BY requested_at DESC, rowid DESC LIMIT 1",
            params![session_id, task_scope, PROFILE, ACTIVE],
            |r| r.get(0),
        )
        .optional()?)
    })
}

/// Binds a session, or with [`TASK_SCOPE`] a task, to a named profile. Only the owner's own
/// commands and screens call this: no tool a helper can reach does.
pub fn bind_profile(db: &Database, scope: &str, name: &str, now: i64) -> Result<String> {
    file(
        db,
        &NewGrant {
            session_id: scope,
            task_id: None,
            host: "",
            repo: "",
            capability: PROFILE,
            resource: name,
            delta: &SandboxDelta::default(),
            state: ACTIVE,
            decided_by: BY_OWNER,
            helper_reason: "",
            decision_note: "",
            expires_at: None,
        },
        now,
    )
}

/// A request of this session for exactly this resource that is still waiting or in force, so
/// asking twice never files twice.
pub fn open_for(
    db: &Database,
    session_id: &str,
    capability: &str,
    resource: &str,
    now: i64,
) -> Result<Option<Grant>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!(
                "SELECT {COLUMNS} FROM sandbox_grants
                 WHERE session_id = ?1 AND capability = ?2 AND resource = ?3 AND state IN (?4, ?5)
                   AND (expires_at IS NULL OR expires_at > ?6)
                 ORDER BY requested_at DESC, rowid DESC LIMIT 1"
            ),
            params![session_id, capability, resource, PENDING, ACTIVE, now],
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
pub fn pending_for_session(db: &Database, session_id: &str, now: i64) -> Result<Vec<Grant>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {COLUMNS} FROM sandbox_grants WHERE session_id = ?1 AND state = ?2
               AND (expires_at IS NULL OR expires_at > ?3)
             ORDER BY requested_at, rowid"
        ))?;
        let rows = stmt.query_map(params![session_id, PENDING, now], row)?;
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

/// The sandbox change a launch of `session_id` must carry: every active grant of the session
/// that has not expired, merged.
pub fn delta_for_session(db: &Database, session_id: &str, now: i64) -> Result<SandboxDelta> {
    delta_for_scopes(db, &[session_id.to_string()], now)
}

/// The same for every scope that applies to a session: the session itself, its task and its
/// repository. Standing rules for the task and repository come in here.
pub fn delta_for_scopes(db: &Database, scopes: &[String], now: i64) -> Result<SandboxDelta> {
    let mut deltas: Vec<String> = Vec::new();
    for scope in scopes {
        let rows: Vec<String> = db.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT delta_json FROM sandbox_grants
                 WHERE session_id = ?1 AND state = ?2 AND capability <> ?4
                   AND (expires_at IS NULL OR expires_at > ?3)
                 ORDER BY requested_at, rowid",
            )?;
            let rows = stmt.query_map(params![scope, ACTIVE, now, PROFILE], |r| r.get(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
        })?;
        deltas.extend(rows);
    }
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

/// The owner's standing rules for these scopes (task and repository), active and unexpired.
pub fn rules_for(db: &Database, scopes: &[String], now: i64) -> Result<Vec<Grant>> {
    let mut out = Vec::new();
    for scope in scopes {
        let rows: Vec<Grant> = db.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLUMNS} FROM sandbox_grants
                 WHERE session_id = ?1 AND state = ?2 AND capability <> ?4
                   AND (expires_at IS NULL OR expires_at > ?3)
                 ORDER BY requested_at, rowid"
            ))?;
            let rows = stmt.query_map(params![scope, ACTIVE, now, PROFILE], row)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })?;
        out.extend(rows);
    }
    Ok(out)
}

/// Every standing rule in force, newest first, for the owner's Access view.
pub fn all_rules(db: &Database, now: i64) -> Result<Vec<Grant>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {COLUMNS} FROM sandbox_grants
             WHERE state = ?1 AND capability <> ?3 AND (session_id LIKE 'task:%' OR session_id LIKE 'repo:%')
               AND (expires_at IS NULL OR expires_at > ?2)
             ORDER BY requested_at DESC, rowid DESC"
        ))?;
        let rows = stmt.query_map(params![ACTIVE, now, PROFILE], row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// Adds a standing rule for a task or repository scope (`task:<id>` or `repo:<root>`). Asking for
/// one that is already in force changes nothing and returns `None`. Only the owner's own routes
/// call this.
pub fn add_rule(
    db: &Database,
    scope: &str,
    capability: &str,
    resource: &str,
    note: &str,
    now: i64,
    expires_at: Option<i64>,
) -> Result<Option<String>> {
    if open_for(db, scope, capability, resource, now)?.is_some() {
        return Ok(None);
    }
    file(
        db,
        &NewGrant {
            session_id: scope,
            task_id: None,
            host: "",
            repo: "",
            capability,
            resource,
            // The rule carries no delta of its own: what it opens is worked out against the
            // session computer's home each time a session starts or asks.
            delta: &SandboxDelta::default(),
            state: ACTIVE,
            decided_by: BY_OWNER,
            helper_reason: "",
            decision_note: note,
            expires_at,
        },
        now,
    )
    .map(Some)
}

/// What HQ did for the owner since `since`: requests it approved under standing access, ones it
/// took to the owner, and ones it refused. Counts only, never paths.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub approved_by_policy: i64,
    pub approved_by_owner: i64,
    pub asked_owner: i64,
    pub declined: i64,
    pub refused: i64,
    pub standing_rules: i64,
}

pub fn summary(db: &Database, since: i64, now: i64) -> Result<Summary> {
    db.with_conn(|c| {
        let count = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<i64> {
            c.query_row(sql, p, |r| r.get(0))
        };
        let base = "SELECT COUNT(*) FROM sandbox_grants WHERE requested_at >= ?1 AND capability <> 'profile' AND session_id NOT LIKE 'task:%' AND session_id NOT LIKE 'repo:%'";
        Ok(Summary {
            approved_by_policy: count(&format!("{base} AND decided_by = 'policy' AND state IN ('active','revoked','expired')"), &[&since])?,
            approved_by_owner: count(&format!("{base} AND decided_by = 'owner' AND state IN ('active','revoked','expired')"), &[&since])?,
            asked_owner: count(&format!("{base} AND (state = 'pending' OR decided_by = 'owner' OR state = 'declined')"), &[&since])?,
            declined: count(&format!("{base} AND state = 'declined'"), &[&since])?,
            refused: count(&format!("{base} AND state = 'refused'"), &[&since])?,
            standing_rules: count(
                "SELECT COUNT(*) FROM sandbox_grants WHERE state = 'active' AND capability <> 'profile' AND (session_id LIKE 'task:%' OR session_id LIKE 'repo:%') AND (expires_at IS NULL OR expires_at > ?1)",
                &[&now],
            )?,
        })
    })
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

    fn at<'a>(resource: &'a str, g: NewGrant<'a>) -> NewGrant<'a> {
        NewGrant { resource, ..g }
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
        file(&db, &at("/r/b", new("s1", &b, PENDING)), 10).unwrap();
        file(&db, &new("s2", &c, ACTIVE), 10).unwrap();
        let mut expired = at("/r/c", new("s1", &c, ACTIVE));
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
        assert!(decide(&db, &id, ACTIVE, 20, Some(900)).unwrap());
        assert!(
            !decide(&db, &id, ACTIVE, 21, Some(900)).unwrap(),
            "not pending any more"
        );
        assert_eq!(delta_for_session(&db, "s1", 30).unwrap().writable.len(), 1);
        assert!(revoke(&db, &id, 40).unwrap());
        assert!(delta_for_session(&db, "s1", 50).unwrap().is_empty());
        assert!(!decide(&db, &id, ACTIVE, 60, Some(900)).unwrap());
        assert_eq!(get(&db, &id).unwrap().unwrap().state, REVOKED);
    }

    #[test]
    fn a_task_binding_reaches_its_sessions_and_the_newest_binding_wins() {
        let db = Database::open_memory().unwrap();
        bind_profile(&db, &format!("{TASK_SCOPE}t1"), "mobile", 1).unwrap();
        assert_eq!(
            profile_for(&db, "s1", Some("t1")).unwrap().as_deref(),
            Some("mobile")
        );
        assert_eq!(profile_for(&db, "s1", Some("t2")).unwrap(), None);
        assert_eq!(profile_for(&db, "s1", None).unwrap(), None);
        bind_profile(&db, "s1", "web", 2).unwrap();
        assert_eq!(
            profile_for(&db, "s1", Some("t1")).unwrap().as_deref(),
            Some("web")
        );
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
        assert_eq!(profile_for(&db, "s1", None).unwrap(), None);
        file(&db, &bind("mobile"), 1).unwrap();
        file(&db, &bind("web"), 2).unwrap();
        assert_eq!(
            profile_for(&db, "s1", None).unwrap().as_deref(),
            Some("web")
        );
        assert_eq!(profile_for(&db, "s2", None).unwrap(), None);
    }

    #[test]
    fn revoking_a_session_clears_every_live_grant_and_counts_setbacks() {
        let db = Database::open_memory().unwrap();
        let d = delta("/a");
        file(&db, &at("/r/1", new("s1", &d, ACTIVE)), 1).unwrap();
        file(&db, &at("/r/2", new("s1", &d, PENDING)), 2).unwrap();
        file(&db, &at("/r/3", new("s1", &d, REFUSED)), 3).unwrap();
        file(&db, &at("/r/4", new("s1", &d, DECLINED)), 4).unwrap();
        assert_eq!(revoke_session(&db, "s1", 9).unwrap(), 2);
        assert_eq!(setbacks(&db, "s1").unwrap(), 2);
        assert_eq!(for_session(&db, "s1", 10).unwrap().len(), 4);
    }

    #[test]
    fn a_waiting_request_lapses_and_an_approval_runs_from_the_answer_not_the_ask() {
        let db = Database::open_memory().unwrap();
        let d = delta("/a");
        let mut ask = new("s1", &d, PENDING);
        ask.expires_at = Some(100);
        let id = file(&db, &ask, 10).unwrap();
        assert_eq!(pending_for_session(&db, "s1", 50).unwrap().len(), 1);
        assert!(
            open_for(&db, "s1", "fs_write", "/r/x", 50)
                .unwrap()
                .is_some()
        );
        assert!(pending_for_session(&db, "s1", 150).unwrap().is_empty());
        assert!(
            open_for(&db, "s1", "fs_write", "/r/x", 150)
                .unwrap()
                .is_none()
        );
        assert_eq!(expire_stale(&db, 150).unwrap(), 1);
        assert_eq!(get(&db, &id).unwrap().unwrap().state, EXPIRED);
        assert!(
            !decide(&db, &id, ACTIVE, 160, Some(999)).unwrap(),
            "too late"
        );

        let mut ask = new("s1", &d, PENDING);
        ask.expires_at = Some(100);
        let id = file(&db, &ask, 10).unwrap();
        assert!(decide(&db, &id, ACTIVE, 90, Some(5000)).unwrap());
        assert_eq!(get(&db, &id).unwrap().unwrap().expires_at, Some(5000));
        assert!(!delta_for_session(&db, "s1", 4000).unwrap().is_empty());
    }

    #[test]
    fn a_restart_is_due_until_it_is_done_and_a_request_cannot_be_filed_twice() {
        let db = Database::open_memory().unwrap();
        let d = delta("/a");
        assert!(!restart_due(&db, "s1").unwrap());
        file(&db, &new("s1", &d, ACTIVE), 1).unwrap();
        assert!(restart_due(&db, "s1").unwrap());
        assert!(
            file(&db, &new("s1", &d, PENDING), 2).is_err(),
            "same request, one live row"
        );
        clear_restart_due(&db, "s1").unwrap();
        assert!(!restart_due(&db, "s1").unwrap());
        let bind = |n: &'static str| NewGrant {
            capability: PROFILE,
            resource: n,
            ..new("s1", &d, ACTIVE)
        };
        file(&db, &bind("a"), 3).unwrap();
        file(&db, &bind("a"), 4).unwrap();
        assert!(
            !restart_due(&db, "s1").unwrap(),
            "a binding needs no restart"
        );
    }

    #[test]
    fn standing_rules_are_found_by_scope_listed_apart_and_stop_when_revoked() {
        let db = Database::open_memory().unwrap();
        assert!(
            add_rule(
                &db,
                "repo:/repo",
                "fs_write",
                "/repo/cache",
                "always",
                1,
                None
            )
            .unwrap()
            .is_some()
        );
        assert!(
            add_rule(
                &db,
                "repo:/repo",
                "fs_write",
                "/repo/cache",
                "always",
                2,
                None
            )
            .unwrap()
            .is_none(),
            "idempotent"
        );
        add_rule(&db, "task:t1", "fs_write", "/repo/t", "task", 3, None).unwrap();
        let both = ["task:t1".to_string(), "repo:/repo".to_string()];
        let rules = rules_for(&db, &both, 10).unwrap();
        assert_eq!(
            rules.iter().map(|r| r.scope.as_str()).collect::<Vec<_>>(),
            vec!["task", "repo"]
        );
        let other = ["task:t9".to_string(), "repo:/else".to_string()];
        assert!(
            rules_for(&db, &other, 10).unwrap().is_empty(),
            "another repo and task get nothing"
        );
        assert_eq!(all_rules(&db, 10).unwrap().len(), 2);
        assert!(revoke(&db, &rules[1].id, 20).unwrap());
        assert_eq!(rules_for(&db, &both, 30).unwrap().len(), 1);
        let mut lapsing = add_rule(&db, "repo:/r2", "fs_write", "/r2", "x", 1, Some(50)).unwrap();
        assert!(lapsing.take().is_some());
        assert_eq!(
            rules_for(&db, &["repo:/r2".to_string()], 40).unwrap().len(),
            1
        );
        assert!(
            rules_for(&db, &["repo:/r2".to_string()], 60)
                .unwrap()
                .is_empty(),
            "a rule lapses"
        );
    }

    #[test]
    fn the_summary_counts_by_outcome_and_leaves_rules_and_bindings_out_of_requests() {
        let db = Database::open_memory().unwrap();
        let d = SandboxDelta::default();
        file(&db, &at("/r/1", new("s1", &d, ACTIVE)), 100).unwrap();
        let mut owner = new("s1", &d, ACTIVE);
        owner.decided_by = BY_OWNER;
        file(&db, &at("/r/2", owner), 100).unwrap();
        file(&db, &at("/r/3", new("s1", &d, PENDING)), 100).unwrap();
        file(&db, &at("/r/4", new("s1", &d, REFUSED)), 100).unwrap();
        add_rule(&db, "repo:/r", "fs_write", "/r", "x", 100, None).unwrap();
        bind_profile(&db, "task:t", "mobile", 100).unwrap();
        let s = summary(&db, 50, 200).unwrap();
        assert_eq!(
            (
                s.approved_by_policy,
                s.approved_by_owner,
                s.refused,
                s.standing_rules
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(
            s.asked_owner, 2,
            "the pending one and the owner-approved one"
        );
        assert_eq!(summary(&db, 500, 600).unwrap().approved_by_policy, 0);
    }

    #[test]
    fn revoking_a_rule_revokes_what_it_approved_so_the_access_stops_at_the_next_launch() {
        let db = Database::open_memory().unwrap();
        let d = delta("/repo/cache");
        let rule = add_rule(&db, "repo:/repo", "fs_write", "/repo", "x", 1, None)
            .unwrap()
            .unwrap();
        let mut derived = new("s1", &d, ACTIVE);
        let note = rule_note(&rule);
        derived.decision_note = &note;
        file(&db, &at("/repo/cache", derived), 2).unwrap();
        assert!(!delta_for_session(&db, "s1", 5).unwrap().is_empty());
        assert!(revoke(&db, &rule, 6).unwrap());
        assert!(
            delta_for_session(&db, "s1", 7).unwrap().is_empty(),
            "the derived grant went with its rule"
        );
    }
}
