//! What pairing set up with each peer (`hq/setup/1`, `docs/security/PAIRING.md` section 15), and
//! the hub's per-device MCP keys. A row is written only for a peer that is paired at that moment,
//! and `bridge::revoke` deletes it and revokes the keys in the same transaction, so every caller of
//! revoke undoes the setup without knowing about it.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::Database;

/// The peer is a hub (this side is a machine).
pub const ROLE_HUB: &str = "hub";
/// The peer is a machine (this side is a hub).
pub const ROLE_MACHINE: &str = "machine";
/// Recognizable prefix, so a leaked device key is easy to spot and scan for.
pub const DEVICE_KEY_PREFIX: &str = "hqd_";
/// Hex characters after the prefix.
pub const DEVICE_KEY_HEX: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub peer_id: String,
    /// [`ROLE_HUB`] or [`ROLE_MACHINE`].
    pub peer_role: String,
    pub peer_name: String,
    pub local_port: u16,
    pub peer_offers: bool,
    pub peer_port: Option<u16>,
    pub mcp_key: Option<String>,
    pub offered: bool,
}

impl Setup {
    pub fn is_hub(&self) -> bool {
        self.peer_role == ROLE_HUB
    }
}

fn now() -> i64 {
    Utc::now().timestamp()
}

/// The sha256 of a device key, hex, as the hub stores it.
pub fn key_hash(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A new per-device key: the prefix and 64 hex characters (256 random bits).
pub fn new_device_key() -> String {
    format!(
        "{DEVICE_KEY_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Whether `text` has the shape of a device key. Says nothing about whether it is live.
pub fn is_device_key(text: &str) -> bool {
    text.strip_prefix(DEVICE_KEY_PREFIX).is_some_and(|hex| {
        hex.len() == DEVICE_KEY_HEX && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

const COLUMNS: &str =
    "peer_id, peer_role, peer_name, local_port, peer_offers, peer_port, mcp_key, offered";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Setup> {
    Ok(Setup {
        peer_id: r.get(0)?,
        peer_role: r.get(1)?,
        peer_name: r.get(2)?,
        local_port: r.get(3)?,
        peer_offers: r.get(4)?,
        peer_port: r.get(5)?,
        mcp_key: r.get(6)?,
        offered: r.get(7)?,
    })
}

pub fn get(db: &Database, peer: &str) -> Result<Option<Setup>> {
    db.with_conn(|c| {
        Ok(c.query_row(
            &format!("SELECT {COLUMNS} FROM bridge_setup WHERE peer_id = ?1"),
            params![peer],
            row,
        )
        .optional()?)
    })
}

/// Every setup row of a peer that is still paired, oldest first.
pub fn list(db: &Database) -> Result<Vec<Setup>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {COLUMNS} FROM bridge_setup s WHERE EXISTS \
             (SELECT 1 FROM bridge_peers p WHERE p.id = s.peer_id AND p.revoked_at IS NULL) \
             ORDER BY created_at ASC, peer_id ASC"
        ))?;
        let rows = stmt.query_map([], row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    })
}

/// Records a setup, and on a hub the hash of a key minted for it, in one transaction. Nothing is
/// written unless the peer is paired now: a revoke that won the race leaves no row behind.
/// Returns whether it was written.
pub fn record(db: &Database, setup: &Setup, new_key: Option<&str>) -> Result<bool> {
    db.with_conn(|c| {
        let tx = c.unchecked_transaction()?;
        let paired: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM bridge_peers WHERE id = ?1 AND revoked_at IS NULL)",
            params![setup.peer_id],
            |r| r.get(0),
        )?;
        if !paired {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO bridge_setup (peer_id, peer_role, peer_name, local_port, peer_offers, \
             peer_port, mcp_key, offered, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9) \
             ON CONFLICT(peer_id) DO UPDATE SET peer_role = excluded.peer_role, \
             peer_name = excluded.peer_name, local_port = excluded.local_port, \
             peer_offers = excluded.peer_offers, peer_port = excluded.peer_port, \
             mcp_key = excluded.mcp_key, offered = excluded.offered, \
             updated_at = excluded.updated_at",
            params![
                setup.peer_id,
                setup.peer_role,
                setup.peer_name,
                setup.local_port,
                setup.peer_offers,
                setup.peer_port,
                setup.mcp_key,
                setup.offered,
                now()
            ],
        )?;
        if let Some(key) = new_key {
            tx.execute(
                "INSERT INTO bridge_device_keys (key_hash, peer_id, created_at) VALUES (?1, ?2, ?3)",
                params![key_hash(key), setup.peer_id, now()],
            )?;
        }
        tx.commit()?;
        Ok(true)
    })
}

/// Whether the peer has a device key that is not revoked.
pub fn has_live_key(db: &Database, peer: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM bridge_device_keys WHERE peer_id = ?1 AND revoked_at IS NULL)",
            params![peer],
            |r| r.get(0),
        )?)
    })
}

/// How many device keys were ever minted for the peer, revoked ones included.
pub fn keys_minted(db: &Database, peer: &str) -> Result<usize> {
    db.with_conn(|c| {
        let n: i64 = c.query_row(
            "SELECT COUNT(*) FROM bridge_device_keys WHERE peer_id = ?1",
            params![peer],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    })
}

/// Whether `secret` is a live device key of a paired peer. Anything else, an error included, is
/// "no" to the caller that treats it so.
pub fn device_key_live(db: &Database, secret: &str) -> Result<bool> {
    if !is_device_key(secret) {
        return Ok(false);
    }
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT EXISTS(SELECT 1 FROM bridge_device_keys k JOIN bridge_peers p \
             ON p.id = k.peer_id WHERE k.key_hash = ?1 AND k.revoked_at IS NULL \
             AND p.revoked_at IS NULL)",
            params![key_hash(secret)],
            |r| r.get(0),
        )?)
    })
}

/// Revokes the peer's keys and deletes its setup row; `bridge::revoke` calls it in its transaction.
pub(crate) fn forget(conn: &Connection, peer: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE bridge_device_keys SET revoked_at = ?2 WHERE peer_id = ?1 AND revoked_at IS NULL",
        params![peer, now()],
    )?;
    conn.execute("DELETE FROM bridge_setup WHERE peer_id = ?1", params![peer])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge as peers;

    const PEER: &str = "aa11";

    fn machine_row() -> Setup {
        Setup {
            peer_id: PEER.into(),
            peer_role: ROLE_MACHINE.into(),
            peer_name: "laptop".into(),
            local_port: 47_401,
            peer_offers: true,
            peer_port: Some(47_402),
            mcp_key: None,
            offered: false,
        }
    }

    #[test]
    fn device_keys_have_a_fixed_shape() {
        let key = new_device_key();
        assert!(is_device_key(&key), "{key}");
        assert_ne!(key, new_device_key());
        for bad in ["", "hqd_", "hqs_00", &format!("hqd_{}", "z".repeat(64))] {
            assert!(!is_device_key(bad), "{bad}");
        }
    }

    #[test]
    fn a_setup_is_recorded_only_for_a_paired_peer() {
        let db = Database::open_memory().unwrap();
        assert!(!record(&db, &machine_row(), Some(&new_device_key())).unwrap());
        assert_eq!(get(&db, PEER).unwrap(), None);
        assert_eq!(keys_minted(&db, PEER).unwrap(), 0);
        peers::add_peer(&db, PEER, "laptop", &[], &[]).unwrap();
        assert!(record(&db, &machine_row(), None).unwrap());
        assert_eq!(list(&db).unwrap(), vec![machine_row()]);
    }

    #[test]
    fn revoking_the_peer_revokes_its_key_and_deletes_its_setup() {
        let db = Database::open_memory().unwrap();
        peers::add_peer(&db, PEER, "laptop", &[], &[]).unwrap();
        let key = new_device_key();
        assert!(record(&db, &machine_row(), Some(&key)).unwrap());
        assert!(device_key_live(&db, &key).unwrap());
        assert!(has_live_key(&db, PEER).unwrap());
        assert!(!device_key_live(&db, &new_device_key()).unwrap());

        assert!(peers::revoke(&db, PEER).unwrap());
        assert!(!device_key_live(&db, &key).unwrap());
        assert!(!has_live_key(&db, PEER).unwrap());
        assert_eq!(get(&db, PEER).unwrap(), None);
        assert_eq!(keys_minted(&db, PEER).unwrap(), 1, "the history stays");

        // Pairing again brings the peer back, not its old key.
        peers::add_peer(&db, PEER, "laptop", &[], &[]).unwrap();
        assert!(!device_key_live(&db, &key).unwrap());
    }
}
