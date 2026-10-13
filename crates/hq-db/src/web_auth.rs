//! Web sign-ins beyond the configured web token (`docs/security/WEB_AUTH.md`). Each signed-in
//! device holds its own token, shown once; the database keeps only its SHA-256. A configured
//! token the owner signed out is remembered by its hash, so a new value set by the operator
//! works again.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::Database;

/// Recognizable prefix, so a leaked token is easy to spot and other secrets skip the lookup.
pub const DEVICE_TOKEN_PREFIX: &str = "hqw_";

/// `last_used_at` moves at most this often per device, so every request is not a write.
pub const LAST_USED_RESOLUTION_SECS: i64 = 5 * 60;

/// A signed-in device as the owner sees it. Never carries the token or its hash.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeviceToken {
    pub id: String,
    pub label: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
}

/// A token just minted: the only time the secret exists outside the device holding it.
pub struct Minted {
    pub id: String,
    pub token: String,
}

impl std::fmt::Debug for Minted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Minted").field("id", &self.id).finish()
    }
}

/// Hex SHA-256 of a secret, the form both tables store.
pub fn hash_secret(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn new_token() -> String {
    format!(
        "{DEVICE_TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn non_blank(token: Option<&str>) -> Option<&str> {
    token.filter(|t| !t.trim().is_empty())
}

fn immediate(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
}

fn insert(conn: &Connection, label: &str, now: i64, expires_at: Option<i64>) -> Result<Minted> {
    conn.execute(
        "DELETE FROM web_device_tokens WHERE expires_at IS NOT NULL AND expires_at <= ?1",
        params![now],
    )?;
    let minted = Minted {
        id: uuid::Uuid::new_v4().simple().to_string(),
        token: new_token(),
    };
    conn.execute(
        "INSERT INTO web_device_tokens (id, token_hash, label, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            minted.id,
            hash_secret(&minted.token),
            label,
            now,
            expires_at
        ],
    )?;
    Ok(minted)
}

/// A new device token labelled `label`. `expires_at` is for a token a local command needs for
/// one call; a device's sign-in never expires on its own.
pub fn mint(db: &Database, label: &str, now: i64, expires_at: Option<i64>) -> Result<Minted> {
    db.with_conn(|c| insert(c, label, now, expires_at))
}

/// The device a token belongs to, or None for an unknown, signed-out or expired token. Notes
/// when the device was last used, at most once per [`LAST_USED_RESOLUTION_SECS`].
pub fn verify(db: &Database, secret: &str, now: i64) -> Result<Option<String>> {
    if !secret.starts_with(DEVICE_TOKEN_PREFIX) {
        return Ok(None);
    }
    let hash = hash_secret(secret);
    db.with_conn(|c| {
        let id: Option<String> = c
            .query_row(
                "SELECT id FROM web_device_tokens
                 WHERE token_hash = ?1 AND (expires_at IS NULL OR expires_at > ?2)",
                params![hash, now],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            return Ok(None);
        };
        let touched = c.execute(
            "UPDATE web_device_tokens SET last_used_at = ?2
             WHERE id = ?1 AND (last_used_at IS NULL OR last_used_at <= ?3)",
            params![id, now, now - LAST_USED_RESOLUTION_SECS],
        );
        if let Err(e) = touched {
            tracing::warn!("web sign-in: could not note when a device was last used: {e}");
        }
        Ok(Some(id))
    })
}

/// Every signed-in device that has not expired, oldest first.
pub fn list(db: &Database, now: i64) -> Result<Vec<DeviceToken>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT id, label, created_at, last_used_at, expires_at FROM web_device_tokens
             WHERE expires_at IS NULL OR expires_at > ?1
             ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![now], |r| {
                Ok(DeviceToken {
                    id: r.get(0)?,
                    label: r.get(1)?,
                    created_at: r.get(2)?,
                    last_used_at: r.get(3)?,
                    expires_at: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Signs one device out. Returns whether there was one.
pub fn delete(db: &Database, id: &str) -> Result<bool> {
    db.with_conn(|c| {
        let n = c.execute("DELETE FROM web_device_tokens WHERE id = ?1", params![id])?;
        Ok(n > 0)
    })
}

/// Whether the owner signed out this configured token.
pub fn is_revoked(db: &Database, configured: &str) -> Result<bool> {
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT COUNT(*) > 0 FROM web_revoked_tokens WHERE token_hash = ?1",
            params![hash_secret(configured)],
            |r| r.get(0),
        )?)
    })
}

/// Whether the web app asks for a sign-in at all: a configured token exists (signed out or
/// not) or any device token does. A signed-out configured token still counts, so signing
/// every device out never leaves the server open.
pub fn sign_in_enabled(db: &Database, configured: Option<&str>) -> Result<bool> {
    if non_blank(configured).is_some() {
        return Ok(true);
    }
    db.with_conn(|c| {
        Ok(
            c.query_row("SELECT EXISTS (SELECT 1 FROM web_device_tokens)", [], |r| {
                r.get(0)
            })?,
        )
    })
}

/// "Sign out everywhere": in one transaction, drops every device token, signs out the
/// configured token, and mints the one token the caller keeps.
pub fn rotate(db: &Database, configured: Option<&str>, label: &str, now: i64) -> Result<Minted> {
    db.with_conn(|c| {
        let tx = immediate(c)?;
        tx.execute("DELETE FROM web_device_tokens", [])?;
        if let Some(token) = non_blank(configured) {
            tx.execute(
                "INSERT OR IGNORE INTO web_revoked_tokens (token_hash, revoked_at) VALUES (?1, ?2)",
                params![hash_secret(token), now],
            )?;
        }
        let minted = insert(&tx, label, now, None)?;
        tx.commit()?;
        Ok(minted)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_800_000_000;
    const ENV_TOKEN: &str = "operator-env-token";

    #[test]
    fn a_minted_token_verifies_and_only_its_hash_is_stored() {
        let db = Database::open_memory().unwrap();
        let minted = mint(&db, "Signed in with a code", T0, None).unwrap();
        assert!(minted.token.starts_with(DEVICE_TOKEN_PREFIX));
        assert!(!format!("{minted:?}").contains(&minted.token));
        assert_eq!(
            verify(&db, &minted.token, T0).unwrap(),
            Some(minted.id.clone())
        );
        let stored: Vec<String> = db
            .with_conn(|c| {
                let mut s = c.prepare("SELECT id || token_hash || label FROM web_device_tokens")?;
                let rows = s
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(rows)
            })
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert!(!stored[0].contains(&minted.token), "plaintext never stored");
        assert!(stored[0].contains(&hash_secret(&minted.token)));
    }

    #[test]
    fn unknown_unprefixed_and_signed_out_tokens_are_refused() {
        let db = Database::open_memory().unwrap();
        let minted = mint(&db, "Telegram /login", T0, None).unwrap();
        assert_eq!(verify(&db, "hqw_nope", T0).unwrap(), None);
        assert_eq!(verify(&db, ENV_TOKEN, T0).unwrap(), None);
        assert!(delete(&db, &minted.id).unwrap());
        assert!(!delete(&db, &minted.id).unwrap());
        assert_eq!(verify(&db, &minted.token, T0).unwrap(), None);
    }

    #[test]
    fn an_expiring_token_stops_working_and_is_purged() {
        let db = Database::open_memory().unwrap();
        let short = mint(&db, "Restart notice", T0, Some(T0 + 60)).unwrap();
        assert!(verify(&db, &short.token, T0 + 59).unwrap().is_some());
        assert_eq!(verify(&db, &short.token, T0 + 60).unwrap(), None);
        assert!(list(&db, T0 + 60).unwrap().is_empty());
        mint(&db, "Operator link", T0 + 61, None).unwrap();
        let ids: Vec<String> = db
            .with_conn(|c| {
                let mut s = c.prepare("SELECT id FROM web_device_tokens")?;
                let rows = s
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                Ok(rows)
            })
            .unwrap();
        assert!(!ids.contains(&short.id), "expired rows are purged on mint");
    }

    #[test]
    fn last_used_moves_at_most_once_per_resolution() {
        let db = Database::open_memory().unwrap();
        let minted = mint(&db, "Signed in with a code", T0, None).unwrap();
        let last_used = |db: &Database| list(db, T0).unwrap()[0].last_used_at;
        assert_eq!(last_used(&db), None);
        verify(&db, &minted.token, T0 + 1).unwrap();
        assert_eq!(last_used(&db), Some(T0 + 1));
        verify(&db, &minted.token, T0 + LAST_USED_RESOLUTION_SECS).unwrap();
        assert_eq!(last_used(&db), Some(T0 + 1), "throttled");
        verify(&db, &minted.token, T0 + 1 + LAST_USED_RESOLUTION_SECS).unwrap();
        assert_eq!(last_used(&db), Some(T0 + 1 + LAST_USED_RESOLUTION_SECS));
    }

    #[test]
    fn rotate_signs_everything_out_and_returns_one_working_token() {
        let db = Database::open_memory().unwrap();
        let a = mint(&db, "Signed in with a code", T0, None).unwrap();
        let b = mint(&db, "Discord /login", T0, None).unwrap();
        assert!(!is_revoked(&db, ENV_TOKEN).unwrap());
        let kept = rotate(&db, Some(ENV_TOKEN), "Sign out everywhere", T0 + 5).unwrap();
        assert!(is_revoked(&db, ENV_TOKEN).unwrap());
        assert!(!is_revoked(&db, "a-new-operator-token").unwrap());
        assert_eq!(verify(&db, &a.token, T0 + 5).unwrap(), None);
        assert_eq!(verify(&db, &b.token, T0 + 5).unwrap(), None);
        assert_eq!(verify(&db, &kept.token, T0 + 5).unwrap(), Some(kept.id));
        let devices = list(&db, T0 + 5).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].label, "Sign out everywhere");
        rotate(&db, Some(ENV_TOKEN), "Sign out everywhere", T0 + 6).unwrap();
    }

    #[test]
    fn sign_in_stays_enabled_once_the_configured_token_is_signed_out() {
        let db = Database::open_memory().unwrap();
        assert!(!sign_in_enabled(&db, None).unwrap());
        assert!(!sign_in_enabled(&db, Some("  ")).unwrap());
        assert!(sign_in_enabled(&db, Some(ENV_TOKEN)).unwrap());
        let kept = rotate(&db, Some(ENV_TOKEN), "Sign out everywhere", T0).unwrap();
        delete(&db, &kept.id).unwrap();
        assert!(
            sign_in_enabled(&db, Some(ENV_TOKEN)).unwrap(),
            "a signed-out configured token still asks for a sign-in"
        );
        let orphan = mint(&db, "Operator link", T0, None).unwrap();
        assert!(sign_in_enabled(&db, None).unwrap());
        delete(&db, &orphan.id).unwrap();
        assert!(!sign_in_enabled(&db, None).unwrap());
    }
}
