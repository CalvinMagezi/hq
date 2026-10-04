use anyhow::Result;
use r2d2::{ManageConnection, Pool};
use rusqlite::Connection;
use std::path::Path;
use tracing::{info, warn};

use crate::migrations;

/// Performance pragmas applied to every new connection in the pool.
///
/// `busy_timeout` is deliberately generous: the vault DB is legitimately shared
/// with long-lived peers (Hermes' `hq mcp-serve`, CLI invocations, the web app).
/// A WAL checkpoint against a large WAL routinely exceeds 5s, and giving up that
/// early was the direct cause of the launchd crash loop on 2026-07-29.
const PRAGMAS: &str = "
    PRAGMA busy_timeout=30000;
    PRAGMA synchronous=NORMAL;
    PRAGMA mmap_size=67108864;
    PRAGMA cache_size=2000;
    PRAGMA foreign_keys=ON;
    PRAGMA auto_vacuum=INCREMENTAL;
";

/// A simple r2d2 connection manager for rusqlite.
/// We implement this ourselves to avoid version conflicts with r2d2_sqlite.
pub struct SqliteConnectionManager {
    connection_string: String,
    is_memory: bool,
}

impl SqliteConnectionManager {
    pub fn file(path: impl AsRef<Path>) -> Self {
        Self {
            connection_string: path.as_ref().to_string_lossy().to_string(),
            is_memory: false,
        }
    }

    pub fn memory() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static ID_GEN: AtomicU64 = AtomicU64::new(0);
        let id = ID_GEN.fetch_add(1, Ordering::SeqCst);
        Self {
            connection_string: format!("file:memdb-{}?mode=memory&cache=shared", id),
            is_memory: true,
        }
    }
}

/// Number of attempts made when a connection or migration is blocked by a
/// concurrent writer.
const BUSY_RETRIES: u32 = 5;

/// Returns true when the error is transient lock contention rather than a real
/// failure. Only these are worth retrying — anything else should surface now.
fn is_busy(err: &rusqlite::Error) -> bool {
    use rusqlite::ErrorCode;
    match err {
        rusqlite::Error::SqliteFailure(e, _) => {
            matches!(e.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
        }
        _ => false,
    }
}

/// Retry `f` with exponential backoff while it fails with BUSY/LOCKED.
/// Backoff is ~100ms, 200ms, 400ms, 800ms between the 5 attempts.
fn retry_busy<T, F>(what: &str, mut f: F) -> Result<T, rusqlite::Error>
where
    F: FnMut() -> Result<T, rusqlite::Error>,
{
    let mut delay = std::time::Duration::from_millis(100);
    for attempt in 1..=BUSY_RETRIES {
        match f() {
            Ok(v) => {
                if attempt > 1 {
                    info!(what, attempt, "database contention cleared");
                }
                return Ok(v);
            }
            Err(e) if is_busy(&e) && attempt < BUSY_RETRIES => {
                warn!(
                    what,
                    attempt,
                    backoff_ms = delay.as_millis() as u64,
                    "database busy — retrying"
                );
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("retry_busy loop always returns on the final attempt")
}

impl ManageConnection for SqliteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> Result<Connection, rusqlite::Error> {
        // Opening and applying pragmas can both hit SQLITE_BUSY when another
        // process holds a write lock. Previously any such blip failed the
        // connection, starved the pool, and surfaced as the misleading
        // "timed out waiting for connection".
        retry_busy("connect", || {
            let conn = Connection::open(&self.connection_string)?;

            // WAL is not supported for in-memory DBs
            if !self.is_memory {
                conn.execute_batch("PRAGMA journal_mode=WAL;")?;
            }

            conn.execute_batch(PRAGMAS)?;
            Ok(conn)
        })
    }

    fn is_valid(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch("")
    }

    fn has_broken(&self, _conn: &mut Connection) -> bool {
        false
    }
}

/// Thread-safe SQLite database wrapper backed by an r2d2 connection pool.
///
/// WAL mode allows true parallel reads; the pool hands out independent
/// connections so readers never block each other. Writes are still
/// serialized by SQLite at the filesystem level (which is correct).
#[derive(Clone)]
pub struct Database {
    pool: Pool<SqliteConnectionManager>,
}

impl Database {
    /// Open (or create) the database at the given path.
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let manager = SqliteConnectionManager::file(db_path);
        let pool = Pool::builder()
            .max_size(8) // 8 connections: ample for daemon + CLI + web
            // No min_idle: eagerly opening idle connections created a burst of
            // simultaneous attempts at cold start — exactly when the DB is most
            // contended. Connections are opened lazily on demand instead.
            .connection_timeout(std::time::Duration::from_secs(60))
            .build(manager)?;

        // Run migrations on one connection. Migrations take a write lock, so
        // they get the same backoff treatment as connection setup.
        let conn = pool.get()?;
        retry_busy("migrations", || {
            migrations::run(&conn).map_err(|e| {
                // Preserve a genuine rusqlite error so is_busy() can classify it;
                // anything else is wrapped as non-retryable.
                match e.downcast::<rusqlite::Error>() {
                    Ok(sqlite_err) => sqlite_err,
                    Err(other) => rusqlite::Error::ModuleError(other.to_string()),
                }
            })
        })?;

        info!(path = %db_path.display(), "database opened (pooled, max_size=8)");
        Ok(Self { pool })
    }

    /// Open an in-memory database (for testing).
    pub fn open_memory() -> Result<Self> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder()
            .max_size(8) // Shared cache allows multiple pooled connections for in-memory DBs
            .build(manager)?;

        let conn = pool.get()?;
        migrations::run(&conn)?;

        Ok(Self { pool })
    }

    /// Execute a closure with a database connection from the pool.
    /// The connection is returned to the pool when the closure finishes.
    pub fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self
            .pool
            .get()
            .map_err(|e| anyhow::anyhow!("pool error: {}", e))?;
        f(&conn)
    }
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("pool_size", &self.pool.max_size())
            .finish()
    }
}
