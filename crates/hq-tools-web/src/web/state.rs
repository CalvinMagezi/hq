//! What web search remembers across restarts: the Mojeek verification cookie
//! and which engines are suspended. Small key-value rows in their own SQLite
//! file (`~/.hq/search_state.db`), so a wiped or corrupt file only costs a
//! cold start. Queries and results are never written here.

use super::*;
use rusqlite::{Connection, params};

const FILE_NAME: &str = "search_state.db";
/// `HQ_SEARCH_STATE=off` keeps everything in memory.
const DISABLE_ENV: &str = "HQ_SEARCH_STATE";

pub(super) struct Store {
    conn: Mutex<Connection>,
}

static PRODUCTION: std::sync::LazyLock<Option<Store>> = std::sync::LazyLock::new(|| {
    // Tests must never write to the developer's own file.
    if cfg!(test) || std::env::var(DISABLE_ENV).is_ok_and(|v| v == "off") {
        return None;
    }
    let dir = hq_core::config::HqConfig::hq_dir();
    std::fs::create_dir_all(&dir).ok()?;
    Store::open(&dir.join(FILE_NAME))
        .map_err(|e| debug!(error = %e, "search state unavailable, continuing in memory"))
        .ok()
});

/// The production store, or `None` when disabled or unusable. Only keys for
/// real endpoints (https) are persisted, so tests against mock servers never
/// write to a user's file.
pub(super) fn production() -> Option<&'static Store> {
    PRODUCTION.as_ref()
}

pub(super) fn is_persistable(key: &str) -> bool {
    key.starts_with("https://")
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Store {
    pub(super) fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                expires_at INTEGER NOT NULL
            )",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(super) fn put(&self, key: &str, value: &str, ttl: Duration) {
        let expires = now_secs().saturating_add(ttl.as_secs() as i64);
        self.exec(
            "INSERT INTO state (key, value, expires_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, expires_at = ?3",
            params![key, value, expires],
        );
    }

    pub(super) fn get(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT value FROM state WHERE key = ?1 AND expires_at > ?2",
            params![key, now_secs()],
            |row| row.get(0),
        )
        .ok()
    }

    pub(super) fn remove(&self, key: &str) {
        self.exec("DELETE FROM state WHERE key = ?1", params![key]);
    }

    /// Unexpired rows whose key starts with `prefix`, as `(key, value, seconds left)`.
    pub(super) fn live_with_prefix(&self, prefix: &str) -> Vec<(String, String, u64)> {
        let Ok(conn) = self.conn.lock() else {
            return Vec::new();
        };
        let now = now_secs();
        let Ok(mut stmt) = conn.prepare(
            "SELECT key, value, expires_at FROM state WHERE key LIKE ?1 ESCAPE '\\' AND expires_at > ?2",
        ) else {
            return Vec::new();
        };
        let pattern = format!(
            "{}%",
            prefix
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        stmt.query_map(params![pattern, now], |row| {
            let left: i64 = row.get(2)?;
            Ok((row.get(0)?, row.get(1)?, (left - now).max(0) as u64))
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    fn exec(&self, sql: &str, args: impl rusqlite::Params) {
        if let Ok(conn) = self.conn.lock()
            && let Err(e) = conn.execute(sql, args)
        {
            debug!(error = %e, "search state write failed");
        }
    }
}

const SUSPENSION_PREFIX: &str = "suspended:";

/// Persist or clear an endpoint's suspension after a call.
pub(super) fn save_health(key: &str, failures: u32, remaining: Option<Duration>) {
    if let (true, Some(store)) = (is_persistable(key), production()) {
        store.save_health(key, failures, remaining);
    }
}

/// Suspensions still running when the process started.
pub(super) fn load_health() -> Vec<(String, u32, Duration)> {
    production().map(Store::load_health).unwrap_or_default()
}

impl Store {
    fn save_health(&self, key: &str, failures: u32, remaining: Option<Duration>) {
        let row = format!("{SUSPENSION_PREFIX}{key}");
        match remaining {
            Some(left) if !left.is_zero() => self.put(&row, &failures.to_string(), left),
            _ => self.remove(&row),
        }
    }

    fn load_health(&self) -> Vec<(String, u32, Duration)> {
        self.live_with_prefix(SUSPENSION_PREFIX)
            .into_iter()
            .filter_map(|(key, value, left)| {
                let key = key.strip_prefix(SUSPENSION_PREFIX)?.to_string();
                Some((key, value.parse().unwrap_or(1), Duration::from_secs(left)))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join(FILE_NAME)).unwrap();
        (dir, store)
    }

    #[test]
    fn values_survive_a_reopen_until_they_expire() {
        let (dir, store) = temp_store();
        store.put("a", "1", Duration::from_secs(60));
        store.put("gone", "x", Duration::ZERO);
        drop(store);
        let again = Store::open(&dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(again.get("a").as_deref(), Some("1"));
        assert_eq!(again.get("gone"), None);
        again.put("a", "2", Duration::from_secs(60));
        assert_eq!(again.get("a").as_deref(), Some("2"));
        again.remove("a");
        assert_eq!(again.get("a"), None);
    }

    #[test]
    fn prefix_listing_reports_time_left_and_treats_wildcards_literally() {
        let (_dir, store) = temp_store();
        store.put(
            "suspended:https://x#native:a_b",
            "3",
            Duration::from_secs(100),
        );
        store.put(
            "suspended:https://x#native:aXb",
            "1",
            Duration::from_secs(100),
        );
        store.put("other", "1", Duration::from_secs(100));
        let rows = store.live_with_prefix("suspended:https://x#native:a_");
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows[0].2 > 90 && rows[0].2 <= 100);
        assert_eq!(rows[0].1, "3");
    }

    #[test]
    fn a_suspension_is_restored_after_a_restart_and_cleared_by_a_success() {
        let (dir, store) = temp_store();
        store.save_health(
            "https://e.example#native:x",
            3,
            Some(Duration::from_secs(120)),
        );
        store.save_health(
            "https://f.example#native:y",
            1,
            Some(Duration::from_secs(120)),
        );
        store.save_health("https://f.example#native:y", 0, None);
        drop(store);
        let again = Store::open(&dir.path().join(FILE_NAME)).unwrap();
        let restored = again.load_health();
        assert_eq!(restored.len(), 1, "{restored:?}");
        assert_eq!(restored[0].0, "https://e.example#native:x");
        assert_eq!(restored[0].1, 3);
        assert!(restored[0].2 > Duration::from_secs(100));
    }

    #[test]
    fn only_real_endpoints_are_persisted() {
        assert!(is_persistable("https://www.mojeek.com#native:mojeek"));
        assert!(!is_persistable("http://127.0.0.1:4000#native:mojeek"));
    }
}
