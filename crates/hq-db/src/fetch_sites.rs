//! Which sites a session that has read outside text may fetch without asking again. Only the
//! owner's approvals are written here. Trust is per exact host, so one person's page on a shared
//! host does not cover the others.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

/// Approved fetches of one site in one chat that allow the rest of that site in that chat.
const APPROVALS_FOR_WHOLE_SITE: i64 = 3;
/// Different chats that must have approved a site before it stops asking in new chats.
const CHATS_FOR_TRUST: i64 = 3;

/// Records one approved fetch. True when the site is now allowed throughout this chat.
pub fn record_approval(conn: &Connection, host: &str, scope: &str) -> Result<bool> {
    conn.execute(
        "INSERT INTO fetch_site_grants (host, scope, approvals, updated_at) VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(host, scope) DO UPDATE SET approvals = approvals + 1,
           updated_at = excluded.updated_at",
        params![host, scope, Utc::now().to_rfc3339()],
    )?;
    conn.execute(
        "UPDATE fetch_site_grants SET whole_site = 1
         WHERE host = ?1 AND scope = ?2 AND approvals >= ?3",
        params![host, scope, APPROVALS_FOR_WHOLE_SITE],
    )?;
    Ok(whole_site(conn, host, scope)?)
}

/// The owner chose to allow the whole site in this chat.
pub fn allow_whole_site(conn: &Connection, host: &str, scope: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO fetch_site_grants (host, scope, approvals, whole_site, updated_at)
         VALUES (?1, ?2, 1, 1, ?3)
         ON CONFLICT(host, scope) DO UPDATE SET approvals = approvals + 1, whole_site = 1,
           updated_at = excluded.updated_at",
        params![host, scope, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn whole_site(conn: &Connection, host: &str, scope: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT whole_site FROM fetch_site_grants WHERE host = ?1 AND scope = ?2",
            params![host, scope],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        == Some(1))
}

/// A declined fetch undoes what this chat had earned for the site.
pub fn forget_in_chat(conn: &Connection, host: &str, scope: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM fetch_site_grants WHERE host = ?1 AND scope = ?2",
        params![host, scope],
    )?;
    Ok(())
}

pub fn is_allowed(conn: &Connection, host: &str, scope: &str) -> Result<bool> {
    if whole_site(conn, host, scope)? {
        return Ok(true);
    }
    let chats: i64 = conn.query_row(
        "SELECT COUNT(*) FROM fetch_site_grants WHERE host = ?1",
        params![host],
        |r| r.get(0),
    )?;
    Ok(chats >= CHATS_FOR_TRUST)
}

/// Sites that no longer ask in new chats, for the owner to review.
pub fn trusted(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT host FROM fetch_site_grants GROUP BY host HAVING COUNT(*) >= ?1 ORDER BY host",
    )?;
    let rows = stmt.query_map(params![CHATS_FOR_TRUST], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The owner removed a site: it must be earned again.
pub fn forget(conn: &Connection, host: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM fetch_site_grants WHERE host = ?1",
        params![host],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_db(check: impl FnOnce(&Connection) -> Result<()>) {
        crate::Database::open_memory()
            .unwrap()
            .with_conn(check)
            .unwrap();
    }

    #[test]
    fn three_approvals_allow_the_site_in_that_chat_only() {
        with_db(|c| {
            assert!(!record_approval(c, "a.example", "chat-1")?);
            assert!(!record_approval(c, "a.example", "chat-1")?);
            assert!(!is_allowed(c, "a.example", "chat-1")?);
            assert!(record_approval(c, "a.example", "chat-1")?);
            assert!(is_allowed(c, "a.example", "chat-1")?);
            assert!(!is_allowed(c, "a.example", "chat-2")?);
            assert!(!is_allowed(c, "b.example", "chat-1")?);
            Ok(())
        });
    }

    #[test]
    fn a_site_approved_in_three_chats_stops_asking_until_removed() {
        with_db(|c| {
            for chat in ["chat-1", "chat-2"] {
                record_approval(c, "a.example", chat)?;
            }
            assert!(!is_allowed(c, "a.example", "chat-new")?);
            assert!(trusted(c)?.is_empty());
            record_approval(c, "a.example", "chat-3")?;
            assert!(is_allowed(c, "a.example", "chat-new")?);
            assert_eq!(trusted(c)?, ["a.example"]);
            forget(c, "a.example")?;
            assert!(!is_allowed(c, "a.example", "chat-new")?);
            Ok(())
        });
    }

    #[test]
    fn allowing_the_whole_site_counts_at_once_and_a_decline_undoes_it() {
        with_db(|c| {
            allow_whole_site(c, "a.example", "chat-1")?;
            assert!(is_allowed(c, "a.example", "chat-1")?);
            forget_in_chat(c, "a.example", "chat-1")?;
            assert!(!is_allowed(c, "a.example", "chat-1")?);
            Ok(())
        });
    }
}
