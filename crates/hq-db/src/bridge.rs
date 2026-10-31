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

/// Records a pairing. Pairing a key that was revoked brings it back with the new details. Empty
/// relay or address lists keep what is already known: a pairing that learned nothing new must not
/// forget how to dial the peer. A new pairing starts setup over (`bridge_setup`): whatever was set
/// up before, and its device key, go in the same transaction.
pub fn add_peer(
    db: &Database,
    id: &str,
    label: &str,
    relays: &[String],
    addrs: &[String],
) -> Result<()> {
    db.with_conn(|c| {
        let tx = c.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO bridge_peers (id, label, relays, addrs, paired_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET label = excluded.label, \
             relays = CASE WHEN excluded.relays = '[]' THEN bridge_peers.relays ELSE excluded.relays END, \
             addrs = CASE WHEN excluded.addrs = '[]' THEN bridge_peers.addrs ELSE excluded.addrs END, \
             paired_at = excluded.paired_at, revoked_at = NULL",
            params![
                id,
                label,
                serde_json::to_string(relays)?,
                serde_json::to_string(addrs)?,
                now()
            ],
        )?;
        crate::bridge_setup::forget(&tx, id)?;
        tx.commit()?;
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

/// Revokes a peer, and in the same transaction undoes what pairing set up with it: its device
/// keys are revoked and its setup row is deleted (`bridge_setup`). Files and listeners that the
/// setup made are swept by whoever owns them. Returns whether it was paired and active.
pub fn revoke(db: &Database, id: &str) -> Result<bool> {
    db.with_conn(|c| {
        let tx = c.unchecked_transaction()?;
        let n = tx.execute(
            "UPDATE bridge_peers SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            params![id, now()],
        )?;
        crate::bridge_setup::forget(&tx, id)?;
        tx.commit()?;
        Ok(n > 0)
    })
}

/// Records how to dial an active peer back; an empty list keeps what is known. A peer revoked in
/// between is left alone.
pub fn set_dial_info(db: &Database, id: &str, relays: &[String], addrs: &[String]) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "UPDATE bridge_peers SET \
             relays = CASE WHEN ?2 = '[]' THEN relays ELSE ?2 END, \
             addrs = CASE WHEN ?3 = '[]' THEN addrs ELSE ?3 END \
             WHERE id = ?1 AND revoked_at IS NULL",
            params![
                id,
                serde_json::to_string(relays)?,
                serde_json::to_string(addrs)?
            ],
        )?;
        Ok(())
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

/// The `hq edge` this device is registered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeRecord {
    /// The edge's endpoint id, lowercase hex.
    pub id: String,
    pub domain: String,
    /// The name this device is reached at through the edge.
    pub hostname: String,
    pub relay: Option<String>,
    pub registered_at: i64,
}

/// Records the edge this device registered with, replacing any other: a device has one.
pub fn edge_set(
    db: &Database,
    id: &str,
    domain: &str,
    hostname: &str,
    relay: Option<&str>,
) -> Result<()> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO bridge_edge (slot, id, domain, hostname, relay, registered_at) \
             VALUES (1, ?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(slot) DO UPDATE SET id = excluded.id, domain = excluded.domain, \
             hostname = excluded.hostname, relay = excluded.relay, \
             registered_at = excluded.registered_at",
            params![id, domain, hostname, relay, now()],
        )?;
        Ok(())
    })
}

pub fn edge_get(db: &Database) -> Result<Option<EdgeRecord>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT id, domain, hostname, relay, registered_at FROM bridge_edge WHERE slot = 1",
            [],
            |r| {
                Ok(EdgeRecord {
                    id: r.get(0)?,
                    domain: r.get(1)?,
                    hostname: r.get(2)?,
                    relay: r.get(3)?,
                    registered_at: r.get(4)?,
                })
            },
        )
        .optional()?)
    })
}

/// Whether `id` is the registered edge. The gate treats an error as "no".
pub fn is_edge(db: &Database, id: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM bridge_edge WHERE slot = 1 AND id = ?1)",
            params![id],
            |r| r.get(0),
        )?)
    })
}

/// Forgets the edge. Returns whether there was one.
pub fn edge_clear(db: &Database) -> Result<bool> {
    db.with_conn(|c| Ok(c.execute("DELETE FROM bridge_edge WHERE slot = 1", [])? > 0))
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
    fn a_device_has_one_edge_and_registering_again_replaces_it() {
        let db = Database::open_memory().unwrap();
        assert_eq!(edge_get(&db).unwrap(), None);
        assert!(!is_edge(&db, "ed01").unwrap());
        assert!(!edge_clear(&db).unwrap(), "nothing to forget");

        edge_set(&db, "ed01", "edge.example.com", "aa.edge.example.com", None).unwrap();
        assert!(is_edge(&db, "ed01").unwrap());
        assert!(!is_edge(&db, "ed02").unwrap());
        let first = edge_get(&db).unwrap().unwrap();
        assert_eq!(first.hostname, "aa.edge.example.com");
        assert_eq!(first.relay, None);

        edge_set(
            &db,
            "ed02",
            "other.example.org",
            "aa.other.example.org",
            Some("https://relay.other.example.org"),
        )
        .unwrap();
        assert!(
            !is_edge(&db, "ed01").unwrap(),
            "the old edge lost its place"
        );
        assert!(is_edge(&db, "ed02").unwrap());
        assert_eq!(
            edge_get(&db).unwrap().unwrap().relay.as_deref(),
            Some("https://relay.other.example.org")
        );

        assert!(edge_clear(&db).unwrap());
        assert_eq!(edge_get(&db).unwrap(), None);
        assert!(!is_edge(&db, "ed02").unwrap());
    }

    #[test]
    fn an_edge_is_not_a_paired_peer_and_a_peer_is_not_the_edge() {
        let db = Database::open_memory().unwrap();
        edge_set(&db, "ed01", "edge.example.com", "aa.edge.example.com", None).unwrap();
        add_peer(&db, "aa11", "phone", &[], &[]).unwrap();
        assert!(!is_paired(&db, "ed01").unwrap());
        assert!(!is_edge(&db, "aa11").unwrap());
    }

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
    fn dial_info_is_set_for_an_active_peer_only() {
        let db = Database::open_memory().unwrap();
        add_peer(&db, ID, "phone", &[], &[]).unwrap();
        set_dial_info(
            &db,
            ID,
            &["https://relay.example.com".into()],
            &["192.0.2.1:7000".into()],
        )
        .unwrap();
        let peer = get(&db, ID).unwrap().unwrap();
        assert_eq!(peer.relays, ["https://relay.example.com"]);
        assert_eq!(peer.addrs, ["192.0.2.1:7000"]);
        revoke(&db, ID).unwrap();
        set_dial_info(&db, ID, &[], &[]).unwrap();
        assert_eq!(
            get(&db, ID).unwrap().unwrap().addrs.len(),
            1,
            "a revoked peer is not touched"
        );
    }

    #[test]
    fn pairing_again_with_nothing_new_keeps_how_to_dial_the_peer() {
        let db = Database::open_memory().unwrap();
        add_peer(
            &db,
            ID,
            "phone",
            &["https://relay.example.com".into()],
            &["192.0.2.1:7000".into()],
        )
        .unwrap();
        add_peer(&db, ID, "phone again", &[], &[]).unwrap();
        let peer = get(&db, ID).unwrap().unwrap();
        assert_eq!(peer.label, "phone again");
        assert_eq!((peer.relays.len(), peer.addrs.len()), (1, 1), "{peer:?}");
        set_dial_info(&db, ID, &[], &[]).unwrap();
        let peer = get(&db, ID).unwrap().unwrap();
        assert_eq!(
            (peer.relays.len(), peer.addrs.len()),
            (1, 1),
            "an empty update keeps them"
        );
        add_peer(&db, ID, "phone", &[], &["198.51.100.2:7000".into()]).unwrap();
        assert_eq!(get(&db, ID).unwrap().unwrap().addrs, ["198.51.100.2:7000"]);
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
