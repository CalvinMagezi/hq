//! A consistent copy of the live database.
//!
//! A raw copy of a SQLite file in WAL mode can miss committed writes that still sit in the `-wal`
//! file, or catch it half way through a write. `VACUUM INTO` reads one consistent snapshot and
//! writes a fresh, compact database, without blocking the process that owns the original. The
//! updater takes its pre-update snapshots the same way (`crates/hq-update/src/dbops.rs`).

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::{BackupError, Result, refused};

const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

fn database(e: impl std::fmt::Display) -> BackupError {
    BackupError::Database(e.to_string())
}

fn open_read_only(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| BackupError::Database(format!("open {}: {e}", path.display())))?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(database)?;
    Ok(conn)
}

/// Writes a consistent copy of `src` to `dest`, which must not exist, and checks the copy.
pub(crate) fn snapshot(src: &Path, dest: &Path) -> Result<()> {
    if !src.is_file() {
        return refused(format!("there is no database at {}", src.display()));
    }
    let dest_text = dest
        .to_str()
        .ok_or_else(|| BackupError::Refused("the snapshot path is not valid text".into()))?;
    {
        let conn = open_read_only(src)?;
        conn.execute("VACUUM INTO ?1", [dest_text])
            .map_err(|e| BackupError::Database(format!("copying {}: {e}", src.display())))?;
    }
    check_integrity(dest)
}

/// Fails unless SQLite reports the database at `path` healthy.
pub(crate) fn check_integrity(path: &Path) -> Result<()> {
    let conn = open_read_only(path)?;
    let verdict: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(database)?;
    if verdict == "ok" {
        Ok(())
    } else {
        Err(BackupError::Corrupt(format!(
            "the database failed its integrity check: {verdict}"
        )))
    }
}

/// Migrations recorded in the database at `path` that this build does not know: a database from
/// a newer HQ, which this build must not open.
pub(crate) fn newer_migrations(path: &Path) -> Result<Vec<String>> {
    let conn = open_read_only(path)?;
    hq_db::migrations::newer_migrations(&conn).map_err(database)
}
