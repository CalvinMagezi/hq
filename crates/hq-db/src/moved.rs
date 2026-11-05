//! A vault that moved to a new phone (docs/plans/ux-run/mobile/wave-0/03-data-transfer.md rule 3):
//! the sign-ins, pairings and grants it holds belonged to the old phone, so the new one forgets them
//! before HQ first starts there. Notes, tasks and chats stay. Every step is safe to run again.

use anyhow::Result;
use serde::Serialize;

use crate::Database;

/// What a moved vault forgot.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Forgotten {
    pub device_sign_ins: usize,
    pub paired_devices: usize,
    pub edge: bool,
    pub connector_grants: usize,
    pub push_subscriptions: usize,
}

/// Why the connector grants were revoked, as their audit records it.
const REVOKE_REASON: &str = "the vault moved to a new phone";

pub fn forget_old_phone(db: &Database, now: i64) -> Result<Forgotten> {
    let mut gone = Forgotten::default();
    for device in crate::web_auth::list(db, now)? {
        gone.device_sign_ins += usize::from(crate::web_auth::delete(db, &device.id)?);
    }
    for peer in crate::bridge::list(db, false)? {
        gone.paired_devices += usize::from(crate::bridge::revoke(db, &peer.id)?);
    }
    gone.edge = crate::bridge::edge_clear(db)?;
    gone.connector_grants =
        db.with_conn(|c| crate::mcp_connector::revoke_all(c, REVOKE_REASON, now))?;
    gone.push_subscriptions = crate::push_subscriptions::delete_all(db)?;
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moved_vault_forgets_the_old_phones_sign_ins_and_pairings_and_twice_is_harmless() {
        let db = Database::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        crate::web_auth::mint(&db, "Chrome on Windows", now, None).unwrap();
        crate::push_subscriptions::upsert(
            &db,
            "https://fcm.googleapis.com/fcm/send/x",
            "k",
            "a",
            None,
            None,
        )
        .unwrap();
        crate::bridge::add_peer(&db, &"ab".repeat(32), "Laptop", &[], &[]).unwrap();

        let gone = forget_old_phone(&db, now).unwrap();
        assert_eq!(gone.device_sign_ins, 1);
        assert_eq!(gone.push_subscriptions, 1);
        assert_eq!(gone.paired_devices, 1);
        assert!(crate::web_auth::list(&db, now).unwrap().is_empty());
        assert!(crate::bridge::list(&db, false).unwrap().is_empty());

        assert_eq!(forget_old_phone(&db, now).unwrap(), Forgotten::default());
    }
}
