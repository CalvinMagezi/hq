use chrono::{DateTime, NaiveDateTime, Utc};
use hq_db::{Database, context_grants, session_tokens};

use super::{
    Caps, DEFAULT_MAX_CHARS, DEFAULT_MAX_GRAPH_DEPTH, DEFAULT_MAX_RESULTS, GrantResolver,
    SessionGrant, SourceId, SourceScope, Unavailable,
};

/// SQLite stores the deadline in UTC without a zone.
const STORED_TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// Resolves a grant from the database: the session's secret row, its registry row and the
/// work lease it holds. Nothing the caller sends can name a task.
pub struct DbGrantResolver<'a> {
    db: &'a Database,
}

impl<'a> DbGrantResolver<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// The session behind a bearer secret, then its grant. An unknown secret is `Revoked`.
    pub fn resolve_token(&self, token: &str) -> Result<SessionGrant, Unavailable> {
        let session = self
            .db
            .with_conn(|conn| session_tokens::session_for_token(conn, token))
            .map_err(|_| Unavailable::Backend)?;
        match session {
            Some(id) => self.resolve(&id),
            None => Err(Unavailable::Revoked),
        }
    }
}

impl GrantResolver for DbGrantResolver<'_> {
    fn resolve(&self, session_id: &str) -> Result<SessionGrant, Unavailable> {
        if session_id.is_empty() {
            return Err(Unavailable::NoIdentity);
        }
        self.db
            .with_conn(|conn| Ok(resolve_with(conn, session_id)))
            .map_err(|_| Unavailable::Backend)?
    }
}

fn resolve_with(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<SessionGrant, Unavailable> {
    let backend = |_| Unavailable::Backend;
    let standing = session_tokens::standing(conn, session_id)
        .map_err(backend)?
        .ok_or(Unavailable::Revoked)?;
    if standing.expired {
        return Err(Unavailable::Expired);
    }
    let session = context_grants::session(conn, session_id)
        .map_err(backend)?
        .filter(|s| s.running)
        .ok_or(Unavailable::NotRunning)?;
    let task_id = context_grants::open_task_for_session(conn, session_id)
        .map_err(backend)?
        .ok_or(Unavailable::NoTask)?;
    let stored = context_grants::allow_prefixes(conn, &task_id)
        .map_err(backend)?
        .ok_or(Unavailable::Unmarked)?;
    // An unreadable prefix is dropped, never widened to something that parses.
    let allow_prefixes: Vec<SourceId> = stored
        .iter()
        .filter_map(|raw| SourceId::parse(raw).ok())
        .collect();
    if allow_prefixes.is_empty() {
        return Err(Unavailable::Unmarked);
    }
    let expires_at = standing
        .expires_at
        .map(|raw| parse_deadline(&raw))
        .transpose()?;
    Ok(SessionGrant {
        session_id: session_id.to_string(),
        task_id,
        mission_id: session.mission_id,
        scope: SourceScope {
            allow_prefixes,
            max_graph_depth: DEFAULT_MAX_GRAPH_DEPTH,
            max_results: DEFAULT_MAX_RESULTS,
            max_chars: DEFAULT_MAX_CHARS,
        },
        caps: Caps::READ,
        expires_at,
    })
}

fn parse_deadline(raw: &str) -> Result<DateTime<Utc>, Unavailable> {
    NaiveDateTime::parse_from_str(raw, STORED_TIME_FORMAT)
        .map(|naive| naive.and_utc())
        .map_err(|_| Unavailable::Backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_gateway::{PrefixScope, ScopeCheck};
    use hq_db::harness_sessions_registry::{self as registry, NewSession, Placement};
    use hq_db::tasks::{self, NewTask};
    use hq_memory::context_packet::GapKind;

    const SESSION: &str = "hs-a";
    const TASK: &str = "t-1";
    const FOLDER: &str = "Notebooks/Projects/Demo";

    fn setup(prefixes: Option<&[&str]>) -> (Database, String) {
        let db = Database::open_memory().unwrap();
        let token = db
            .with_conn(|c| {
                tasks::create_initiative(c, "in-1", "personal", None, "Demo", "demo", "DEMO")?;
                tasks::create_task(
                    c,
                    TASK,
                    "in-1",
                    &NewTask {
                        title: "x",
                        created_by: "test",
                        ..Default::default()
                    },
                )?;
                registry::insert(
                    c,
                    &NewSession {
                        id: SESSION,
                        harness: "claude-code",
                        label: "t",
                        cwd: "/t",
                        mission_id: Some("m-1"),
                        placement: Placement {
                            host: "native",
                            agent_name: SESSION,
                            workspace_id: SESSION,
                            pane_id: SESSION,
                        },
                        purpose: None,
                    },
                )?;
                tasks::open_for_session(c, TASK, SESSION, "a", "claude-code", "h", "/t")?;
                if let Some(p) = prefixes {
                    let owned: Vec<String> = p.iter().map(|s| s.to_string()).collect();
                    context_grants::set_allow_prefixes(c, TASK, &owned)?;
                }
                session_tokens::mint_with_ttl(c, SESSION, Some(600))
            })
            .unwrap();
        (db, token)
    }

    fn grant(db: &Database) -> SessionGrant {
        DbGrantResolver::new(db).resolve(SESSION).unwrap()
    }

    #[test]
    fn a_proven_session_gets_its_tasks_grant_and_no_wider_caps() {
        let (db, token) = setup(Some(&[FOLDER]));
        let g = DbGrantResolver::new(&db).resolve_token(&token).unwrap();
        assert_eq!(
            (g.task_id.as_str(), g.mission_id.as_deref()),
            (TASK, Some("m-1"))
        );
        assert!(g.caps.contains(Caps::READ) && !g.caps.contains(Caps::TASK_UPDATE));
        assert!(g.expires_at.is_some() && !g.is_expired(Utc::now()));
    }

    #[test]
    fn missing_identity_is_unavailable() {
        let (db, _) = setup(Some(&[FOLDER]));
        let r = DbGrantResolver::new(&db);
        assert_eq!(r.resolve(""), Err(Unavailable::NoIdentity));
        assert_eq!(r.resolve("hs-unknown"), Err(Unavailable::Revoked));
        assert_eq!(r.resolve_token("hqs_forged"), Err(Unavailable::Revoked));
        assert_eq!(r.resolve_token("not-a-secret"), Err(Unavailable::Revoked));
    }

    #[test]
    fn an_expired_secret_is_unavailable() {
        let (db, token) = setup(Some(&[FOLDER]));
        db.with_conn(|c| {
            c.execute(
                "UPDATE harness_session_tokens SET expires_at = datetime('now', '-1 seconds')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let r = DbGrantResolver::new(&db);
        assert_eq!(r.resolve(SESSION), Err(Unavailable::Expired));
        assert_eq!(r.resolve_token(&token), Err(Unavailable::Revoked));
    }

    #[test]
    fn a_revoked_secret_or_ended_session_is_unavailable() {
        let (db, _) = setup(Some(&[FOLDER]));
        db.with_conn(|c| registry::set_status_exited_if_running(c, SESSION).map(|_| ()))
            .unwrap();
        assert_eq!(
            DbGrantResolver::new(&db).resolve(SESSION),
            Err(Unavailable::NotRunning)
        );

        let (db, _) = setup(Some(&[FOLDER]));
        db.with_conn(|c| session_tokens::revoke(c, SESSION))
            .unwrap();
        assert_eq!(
            DbGrantResolver::new(&db).resolve(SESSION),
            Err(Unavailable::Revoked)
        );
    }

    #[test]
    fn an_unmarked_task_or_a_session_without_a_lease_is_unavailable() {
        let (db, _) = setup(None);
        assert_eq!(
            DbGrantResolver::new(&db).resolve(SESSION),
            Err(Unavailable::Unmarked)
        );

        let (db, _) = setup(Some(&["../etc", "/abs", "_data/x", "skills/y"]));
        assert_eq!(
            DbGrantResolver::new(&db).resolve(SESSION),
            Err(Unavailable::Unmarked),
            "prefixes that do not parse never widen to anything"
        );

        let (db, _) = setup(Some(&[FOLDER]));
        db.with_conn(|c| {
            c.execute(
                "UPDATE task_work_sessions SET ended_at = datetime('now')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            DbGrantResolver::new(&db).resolve(SESSION),
            Err(Unavailable::NoTask)
        );
    }

    #[test]
    fn scope_denies_traversal_absolute_private_and_neighbouring_paths() {
        let (db, _) = setup(Some(&[FOLDER]));
        let g = grant(&db);
        let ok = |raw: &str| {
            SourceId::parse(raw)
                .map(|id| PrefixScope.permits(&g, &id))
                .unwrap_or(false)
        };
        assert!(ok("Notebooks/Projects/Demo/plan.md"));
        assert!(ok("./Notebooks//Projects/Demo/a/b.md"));
        for raw in [
            "Notebooks/Projects/DemoOther/x.md",
            "Notebooks/Projects/Other/x.md",
            "Notebooks/Projects/Demo/../Other/x.md",
            "../Notebooks/Projects/Demo/x.md",
            "/Notebooks/Projects/Demo/x.md",
            "_data/vault.db",
            "_System/SOUL.md",
            "_threads/a.md",
            "Skills/x/SKILL.md",
            "",
            "chat:thread-1",
        ] {
            assert!(!ok(raw), "{raw}");
        }
    }

    #[test]
    fn denied_sources_appear_only_as_a_count() {
        let (db, _) = setup(Some(&[FOLDER]));
        let g = grant(&db);
        let ids = [
            "Notebooks/Projects/Demo/a.md",
            "Private/secret-name.md",
            "Elsewhere/b.md",
        ]
        .map(|raw| SourceId::parse(raw).unwrap())
        .to_vec();
        let (kept, gap) = PrefixScope.partition(&g, ids);
        assert_eq!(kept.len(), 1);
        let gap = gap.unwrap();
        assert_eq!(gap.kind, GapKind::Denied { count: 2 });
        assert!(!gap.detail.contains("secret") && !gap.detail.contains("Elsewhere"));

        let (_, none) = PrefixScope.partition(&g, kept);
        assert!(none.is_none());
    }
}
