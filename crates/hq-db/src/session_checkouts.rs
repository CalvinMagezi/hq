//! Where a coding session writes (`sql/104_session_checkouts.sql`): its own git worktree, or
//! paths it owns in a folder it shares, and the last commit seen there. Kept apart from
//! `harness_sessions` so the session row stays what it was.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// A session's checkout, as recorded when it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub session_id: String,
    /// The folder the session was asked to start in (the repository's folder).
    pub folder: String,
    /// Its own worktree, when it has one.
    pub worktree: Option<String>,
    /// The repository's main folder, for a worktree.
    pub repo: Option<String>,
    /// A writer's branch.
    pub branch: Option<String>,
    /// A reviewer's commit.
    pub detached: Option<String>,
    /// Paths it owns in a shared folder; empty means the whole folder.
    pub owns: Vec<String>,
    /// The last commit seen at a finished turn.
    pub last_head: Option<String>,
    /// The commit it started at; a resume keeps the first one.
    pub start_head: Option<String>,
    /// When its worktree was removed, if it was.
    pub removed_at: Option<String>,
}

pub struct NewCheckout<'a> {
    pub session_id: &'a str,
    pub folder: &'a str,
    pub worktree: Option<&'a str>,
    pub repo: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub detached: Option<&'a str>,
    pub owns: &'a [String],
    pub head: Option<&'a str>,
}

const COLS: &str =
    "session_id, folder, worktree, repo, branch, detached, owns, last_head, removed_at, start_head";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Checkout> {
    let owns: String = r.get(6)?;
    Ok(Checkout {
        session_id: r.get(0)?,
        folder: r.get(1)?,
        worktree: r.get(2)?,
        repo: r.get(3)?,
        branch: r.get(4)?,
        detached: r.get(5)?,
        owns: serde_json::from_str(&owns).unwrap_or_default(),
        last_head: r.get(7)?,
        removed_at: r.get(8)?,
        start_head: r.get(9)?,
    })
}

pub fn record(c: &Connection, n: &NewCheckout<'_>) -> Result<()> {
    c.execute(
        "INSERT INTO session_checkouts \
         (session_id, folder, worktree, repo, branch, detached, owns, last_head, start_head) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
         ON CONFLICT(session_id) DO UPDATE SET folder = excluded.folder, \
         worktree = excluded.worktree, repo = excluded.repo, branch = excluded.branch, \
         detached = excluded.detached, owns = excluded.owns, removed_at = NULL, \
         start_head = COALESCE(start_head, excluded.start_head)",
        params![
            n.session_id,
            n.folder,
            n.worktree,
            n.repo,
            n.branch,
            n.detached,
            serde_json::to_string(n.owns)?,
            n.head,
        ],
    )?;
    Ok(())
}

pub fn get(c: &Connection, session_id: &str) -> Result<Option<Checkout>> {
    Ok(c.query_row(
        &format!("SELECT {COLS} FROM session_checkouts WHERE session_id = ?1"),
        params![session_id],
        row,
    )
    .optional()?)
}

/// Stores `sha` as the session's last seen commit. Returns the one it replaces, or `None` when
/// nothing changed (or nothing was recorded for the session).
pub fn note_head(c: &Connection, session_id: &str, sha: &str) -> Result<Option<Option<String>>> {
    let before: Option<Option<String>> = c
        .query_row(
            "SELECT last_head FROM session_checkouts WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(before) = before else {
        return Ok(None);
    };
    if before.as_deref() == Some(sha) {
        return Ok(None);
    }
    let n = c.execute(
        "UPDATE session_checkouts SET last_head = ?2 WHERE session_id = ?1 \
         AND last_head IS ?3",
        params![session_id, sha, before],
    )?;
    Ok((n == 1).then_some(before))
}

pub fn mark_removed(c: &Connection, session_id: &str) -> Result<()> {
    c.execute(
        "UPDATE session_checkouts SET removed_at = datetime('now') WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// A running coding session that writes straight into a folder (no worktree of its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    pub session_id: String,
    /// Its agent's name on the host.
    pub agent_name: String,
    pub label: String,
    pub parent_session_id: Option<String>,
    /// Empty: the whole folder, which is also what a session from before `owns` existed holds.
    pub owns: Vec<String>,
}

/// The running `code` sessions on `host` whose working folder is `folder`.
pub fn writers_in(c: &Connection, host: &str, folder: &str) -> Result<Vec<Writer>> {
    let mut stmt = c.prepare(
        "SELECT s.id, s.label, s.parent_session_id, k.owns, s.agent_name FROM harness_sessions s \
         LEFT JOIN session_checkouts k ON k.session_id = s.id \
         WHERE s.status = 'running' AND s.host = ?1 AND s.cwd = ?2 AND s.purpose = 'code' \
         ORDER BY s.created_at",
    )?;
    let rows = stmt
        .query_map(params![host, folder], |r| {
            let owns: Option<String> = r.get(3)?;
            Ok(Writer {
                session_id: r.get(0)?,
                agent_name: r.get(4)?,
                label: r.get(1)?,
                parent_session_id: r.get(2)?,
                owns: owns
                    .and_then(|o| serde_json::from_str(&o).ok())
                    .unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::harness_sessions_registry as registry;

    fn session(c: &Connection, id: &str, cwd: &str, purpose: &str, parent: Option<&str>) {
        registry::insert(
            c,
            &registry::NewSession {
                id,
                harness: "claude-code",
                label: id,
                cwd,
                mission_id: None,
                placement: registry::Placement {
                    host: "native",
                    agent_name: id,
                    workspace_id: "w1",
                    pane_id: "w1:p1",
                },
                purpose: Some(purpose),
            },
        )
        .unwrap();
        if let Some(p) = parent {
            c.execute(
                "UPDATE harness_sessions SET parent_session_id = ?2 WHERE id = ?1",
                params![id, p],
            )
            .unwrap();
        }
    }

    #[test]
    fn writers_are_the_running_coding_sessions_in_that_folder() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            session(c, "hs-a", "/w/app", "code", None);
            session(c, "hs-b", "/w/app", "review", None);
            session(c, "hs-c", "/w/other", "code", None);
            session(c, "hs-d", "/w/app", "code", Some("hs-a"));
            record(
                c,
                &NewCheckout {
                    session_id: "hs-d",
                    folder: "/w/app",
                    worktree: None,
                    repo: None,
                    branch: None,
                    detached: None,
                    owns: &["src/api".to_string()],
                    head: None,
                },
            )?;
            let writers = writers_in(c, "native", "/w/app")?;
            let ids: Vec<&str> = writers.iter().map(|w| w.session_id.as_str()).collect();
            assert_eq!(ids, ["hs-a", "hs-d"]);
            assert!(writers[0].owns.is_empty(), "no record: the whole folder");
            assert_eq!(writers[1].owns, ["src/api"]);
            assert_eq!(writers[1].parent_session_id.as_deref(), Some("hs-a"));
            registry::set_status(c, "hs-a", registry::STATUS_STOPPED)?;
            assert_eq!(writers_in(c, "native", "/w/app")?.len(), 1);
            assert!(writers_in(c, "other-host", "/w/app")?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_new_commit_is_noted_once_and_a_removal_recorded() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(
                c,
                &NewCheckout {
                    session_id: "hs-w",
                    folder: "/w/app",
                    worktree: Some("/h/HQ/worktrees/app/hs-w"),
                    repo: Some("/w/app"),
                    branch: Some("feature"),
                    detached: None,
                    owns: &[],
                    head: Some("aaa"),
                },
            )?;
            assert_eq!(note_head(c, "hs-w", "aaa")?, None, "same commit");
            assert_eq!(note_head(c, "hs-w", "bbb")?, Some(Some("aaa".into())));
            assert_eq!(note_head(c, "hs-w", "bbb")?, None, "noted once");
            assert_eq!(note_head(c, "nobody", "ccc")?, None);
            mark_removed(c, "hs-w")?;
            let got = get(c, "hs-w")?.unwrap();
            assert!(got.removed_at.is_some());
            assert_eq!(got.branch.as_deref(), Some("feature"));
            assert_eq!(got.last_head.as_deref(), Some("bbb"));
            assert_eq!(
                got.start_head.as_deref(),
                Some("aaa"),
                "the start stays put"
            );
            // A resume records again at a later commit; the start is still the first one.
            record(
                c,
                &NewCheckout {
                    session_id: "hs-w",
                    folder: "/w/app",
                    worktree: Some("/h/HQ/worktrees/app/hs-w"),
                    repo: Some("/w/app"),
                    branch: Some("feature"),
                    detached: None,
                    owns: &[],
                    head: Some("bbb"),
                },
            )?;
            assert_eq!(get(c, "hs-w")?.unwrap().start_head.as_deref(), Some("aaa"));
            Ok(())
        })
        .unwrap();
    }
}
