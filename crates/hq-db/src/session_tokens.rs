//! Per-session secrets that let HQ tell which launched session is calling.
//! The agent holds the secret; the database holds only its hash, and a secret
//! stops working when its session is no longer running.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

/// Recognizable prefix, so a leaked secret is easy to spot and scan for.
pub const TOKEN_PREFIX: &str = "hqs_";

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// A new secret for `session_id`, replacing any earlier one. Returned once;
/// only its hash is kept.
pub fn mint(conn: &Connection, session_id: &str) -> Result<String> {
    let token = new_token();
    conn.execute(
        "INSERT INTO harness_session_tokens (session_id, token_hash) VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET token_hash = excluded.token_hash,
                                               created_at = datetime('now')",
        params![session_id, hash(&token)],
    )?;
    Ok(token)
}

/// Drops the secret of a session, for a launch that failed.
pub fn revoke(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM harness_session_tokens WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// The running session a secret belongs to, or None for an unknown secret or
/// one whose session has ended.
pub fn session_for_token(conn: &Connection, token: &str) -> Result<Option<String>> {
    if !token.starts_with(TOKEN_PREFIX) {
        return Ok(None);
    }
    Ok(conn
        .query_row(
            "SELECT t.session_id FROM harness_session_tokens t
             JOIN harness_sessions s ON s.id = t.session_id
             WHERE t.token_hash = ?1 AND s.status = 'running'",
            params![hash(token)],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::harness_sessions_registry::{self as registry, NewSession, Placement};

    fn session(conn: &Connection, id: &str) {
        registry::insert(
            conn,
            &NewSession {
                id,
                harness: "claude-code",
                label: "t",
                cwd: "/t",
                mission_id: None,
                placement: Placement {
                    host: "native",
                    agent_name: id,
                    workspace_id: id,
                    pane_id: id,
                },
            },
        )
        .unwrap();
    }

    #[test]
    fn a_minted_secret_names_its_running_session() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            session(c, "hs-a");
            session(c, "hs-b");
            let (a, b) = (mint(c, "hs-a")?, mint(c, "hs-b")?);
            assert!(a.starts_with(TOKEN_PREFIX) && a != b);
            assert_eq!(session_for_token(c, &a)?.as_deref(), Some("hs-a"));
            assert_eq!(session_for_token(c, &b)?.as_deref(), Some("hs-b"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn only_the_hash_is_stored() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            session(c, "hs-a");
            let token = mint(c, "hs-a")?;
            let stored: String = c.query_row(
                "SELECT token_hash FROM harness_session_tokens",
                [],
                |r| r.get(0),
            )?;
            assert_ne!(stored, token);
            assert!(!stored.contains(&token));
            assert_eq!(stored.len(), 64);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn unknown_forged_and_prefixless_secrets_name_nobody() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            session(c, "hs-a");
            let real = mint(c, "hs-a")?;
            assert_eq!(session_for_token(c, "hqs_nope")?, None);
            assert_eq!(session_for_token(c, "")?, None);
            assert_eq!(session_for_token(c, &real[TOKEN_PREFIX.len()..])?, None);
            assert_eq!(session_for_token(c, &format!("{real}x"))?, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_secret_stops_working_when_its_session_ends() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            session(c, "hs-a");
            let token = mint(c, "hs-a")?;
            assert!(session_for_token(c, &token)?.is_some());
            registry::set_status_exited_if_running(c, "hs-a")?;
            assert_eq!(session_for_token(c, &token)?, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn minting_again_replaces_the_old_secret() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            session(c, "hs-a");
            let (old, new) = (mint(c, "hs-a")?, mint(c, "hs-a")?);
            assert_eq!(session_for_token(c, &old)?, None);
            assert_eq!(session_for_token(c, &new)?.as_deref(), Some("hs-a"));
            Ok(())
        })
        .unwrap();
    }
}
