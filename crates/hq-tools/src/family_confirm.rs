//! Pending action store for Discord family guest confirmations.
//!
//! When a family guest invokes a remote MCP tool (e.g. a billing or CRM server),
//! the action is paused, recorded here with a short token, and a mailbox message
//! is sent asking the owner for approval.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const PENDING_FILE: &str = "_system/pending-family-actions.json";
pub const TTL_HOURS: i64 = 24;

/// Serializes `take_pending`'s read-modify-write against the pending-family-actions
/// file.
static TAKE_PENDING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingFamilyAction {
    pub token: String,
    pub requester_name: String,
    pub origin_channel_id: u64,
    pub server_name: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub summary: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PendingStore {
    #[serde(default)]
    items: Vec<PendingFamilyAction>,
}

impl PendingStore {
    fn load(vault_path: &Path, now: DateTime<Utc>) -> Self {
        let mut s: PendingStore = std::fs::read_to_string(vault_path.join(PENDING_FILE))
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default();
        s.items
            .retain(|p| now.signed_duration_since(p.created_at).num_hours() < TTL_HOURS);
        s
    }

    fn save(&self, vault_path: &Path) -> Result<()> {
        let path = vault_path.join(PENDING_FILE);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Store a pending family action.
pub fn add_pending(vault_path: &Path, action: PendingFamilyAction) -> Result<()> {
    let mut store = PendingStore::load(vault_path, Utc::now());
    store.items.retain(|p| p.token != action.token);
    store.items.push(action);
    store.save(vault_path)
}

/// Take (remove + return) a pending family action by token. None if missing/expired.
/// Thread-safe and race-safe within one process.
pub fn take_pending(vault_path: &Path, token: &str) -> Option<PendingFamilyAction> {
    let _guard = TAKE_PENDING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = PendingStore::load(vault_path, Utc::now());
    let idx = store.items.iter().position(|p| p.token == token)?;
    let action = store.items.remove(idx);
    let _ = store.save(vault_path);
    Some(action)
}
