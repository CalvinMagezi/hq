//! The machines this one has paired with over the Native Bridge (`docs/security/PAIRING.md`).
//! The bridge gate asks `is_paired` on every handshake and every request, so a revoked key stops
//! working at the next one even when the revoke ran in another process.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// The peer's endpoint id, lowercase hex.
    pub id: String,
    pub label: String,
    pub relays: Vec<String>,
    pub addrs: Vec<String>,
    pub paired_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

impl Peer {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

fn now() -> i64 {
    Utc::now().timestamp()
}

fn strings(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// Records a pairing. Pairing a key that was revoked brings it back with the new details.
pub fn add_peer(
    db: &Database,
    id: &str,
    label: &str,
    relays: &[String],
    addrs: &[String],
) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO bridge_peers (id, label, relays, addrs, paired_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET label = excluded.label, relays = excluded.relays, \
             addrs = excluded.addrs, paired_at = excluded.paired_at, revoked_at = NULL",
            params![
                id,
                label,
                serde_json::to_string(relays)?,
                serde_json::to_string(addrs)?,
                now()
            ],
        )?;
        Ok(())
    })
}

/// Whether `id` is paired and not revoked. The gate treats an error as "no".
pub fn is_paired(db: &Database, id: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM bridge_peers WHERE id = ?1 AND revoked_at IS NULL)",
            params![id],
            |r| r.get(0),
        )?)
    })
}

/// Revokes a peer. Returns whether it was paired and active.
pub fn revoke(db: &Database, id: &str) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute(
            "UPDATE bridge_peers SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            params![id, now()],
        )?;
        Ok(n > 0)
    })
}

pub fn touch(db: &Database, id: &str) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE bridge_peers SET last_seen_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
        Ok(())
    })
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Peer> {
    Ok(Peer {
        id: r.get(0)?,
        label: r.get(1)?,
        relays: strings(&r.get::<_, String>(2)?),
        addrs: strings(&r.get::<_, String>(3)?),
        paired_at: r.get(4)?,
        last_seen_at: r.get(5)?,
        revoked_at: r.get(6)?,
    })
}

const COLUMNS: &str = "id, label, relays, addrs, paired_at, last_seen_at, revoked_at";

pub fn get(db: &Database, id: &str) -> Result<Option<Peer>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!("SELECT {COLUMNS} FROM bridge_peers WHERE id = ?1"),
            params![id],
            row,
        )
        .optional()?)
    })
}

/// Every peer, oldest pairing first; revoked ones only when asked for.
pub fn list(db: &Database, include_revoked: bool) -> Result<Vec<Peer>> {
    db.with_conn(|c| {
        let filter = if include_revoked {
            ""
        } else {
            "WHERE revoked_at IS NULL"
        };
        let mut stmt = c.prepare(&format!(
            "SELECT {COLUMNS} FROM bridge_peers {filter} ORDER BY paired_at ASC, id ASC"
        ))?;
        let rows = stmt.query_map([], row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "aa11";

    #[test]
    fn a_paired_peer_is_known_until_it_is_revoked() {
        let db = Database::open_memory().unwrap();
        assert!(!is_paired(&db, ID).unwrap());
        add_peer(&db, ID, "phone", &["https://relay.example.com".into()], &[]).unwrap();
        assert!(is_paired(&db, ID).unwrap());
        let peer = get(&db, ID).unwrap().unwrap();
        assert_eq!(peer.label, "phone");
        assert_eq!(peer.relays, ["https://relay.example.com"]);
        assert!(peer.is_active());

        assert!(revoke(&db, ID).unwrap());
        assert!(!is_paired(&db, ID).unwrap());
        assert!(!revoke(&db, ID).unwrap(), "a second revoke changes nothing");
        assert!(list(&db, false).unwrap().is_empty());
        let history = list(&db, true).unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].revoked_at.is_some(), "the history stays");
    }

    #[test]
    fn pairing_a_revoked_key_again_brings_it_back() {
        let db = Database::open_memory().unwrap();
        add_peer(&db, ID, "old", &[], &[]).unwrap();
        revoke(&db, ID).unwrap();
        add_peer(&db, ID, "new", &[], &["192.0.2.1:7000".into()]).unwrap();
        assert!(is_paired(&db, ID).unwrap());
        let peer = get(&db, ID).unwrap().unwrap();
        assert_eq!((peer.label.as_str(), peer.addrs.len()), ("new", 1));
    }

    #[test]
    fn touching_records_when_a_peer_was_last_seen() {
        let db = Database::open_memory().unwrap();
        add_peer(&db, ID, "", &[], &[]).unwrap();
        assert_eq!(get(&db, ID).unwrap().unwrap().last_seen_at, None);
        touch(&db, ID).unwrap();
        assert!(get(&db, ID).unwrap().unwrap().last_seen_at.is_some());
    }
}
