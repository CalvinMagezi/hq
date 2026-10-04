//! Vault database snapshot, restore and migration count.
//!
//! These run inside `hq update-db`, which the updater starts as the service
//! user, so root never opens or writes a path the service user controls.

use crate::error::Result;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn open_read_only(db: &Path) -> rusqlite::Result<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Writes a consistent copy of `src` to `dest` with `VACUUM INTO`. Returns
/// `false` when there is no database to copy.
pub fn db_snapshot(src: &Path, dest: &Path) -> Result<bool> {
    if !src.is_file() {
        return Ok(false);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = open_read_only(src).map_err(|e| anyhow::anyhow!("open {}: {e}", src.display()))?;
    let dest_str = dest
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("snapshot path is not UTF-8"))?;
    conn.execute("VACUUM INTO ?1", [dest_str])
        .map_err(|e| anyhow::anyhow!("VACUUM INTO {}: {e}", dest.display()))?;
    drop(conn);
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600))?;
    Ok(true)
}

/// Replaces `db` with `snapshot`. The database being replaced is kept as
/// `<db>.pre-restore` (newest only), so writes made after the snapshot was
/// taken are recoverable by hand.
pub fn db_restore(db: &Path, snapshot: &Path) -> Result<()> {
    let tmp = with_suffix(db, ".restore");
    let _ = std::fs::remove_file(&tmp);
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(0o600)
        .open(&tmp)?;
    std::io::copy(&mut std::fs::File::open(snapshot)?, &mut out)?;
    out.sync_all()?;
    drop(out);
    // Keep the replaced database with its WAL, which can hold committed writes
    // not yet checkpointed into the main file.
    let kept = with_suffix(db, ".pre-restore");
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(with_suffix(&kept, suffix));
    }
    if db.exists() {
        std::fs::rename(db, &kept)?;
    }
    for suffix in ["-wal", "-shm"] {
        let live = with_suffix(db, suffix);
        if live.exists() {
            std::fs::rename(&live, with_suffix(&kept, suffix))?;
        }
    }
    std::fs::rename(&tmp, db)?;
    Ok(())
}

/// Number of applied schema migrations (rows of `schema_version`); `None`
/// when the database or the table is missing.
pub fn db_migrations(db: &Path) -> Result<Option<u64>> {
    if !db.is_file() {
        return Ok(None);
    }
    let conn = open_read_only(db).map_err(|e| anyhow::anyhow!("open {}: {e}", db.display()))?;
    let count = conn.query_row("SELECT COUNT(*) FROM schema_version", [], |r| {
        r.get::<_, i64>(0)
    });
    Ok(count.ok().map(|c| c as u64))
}

pub const SNAPSHOT_PREFIX: &str = "vault-";

/// Deletes the oldest `vault-*.db` files in `dir` past `keep`, never one in
/// `protected`. Runs as the service user; never follows a symlinked dir or entry.
pub fn prune_snapshots(dir: &Path, keep: usize, protected: &[PathBuf]) -> usize {
    if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(SNAPSHOT_PREFIX) && n.ends_with(".db"))
        })
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(keep);
    let doomed: Vec<PathBuf> = files
        .into_iter()
        .take(excess)
        .filter(|p| !protected.contains(p))
        .collect();
    doomed
        .iter()
        .filter(|p| std::fs::remove_file(p).is_ok())
        .count()
}

/// Entry point of the hidden `hq update-db <op> <args...>` command; the
/// result is printed for the updater to parse.
pub fn run_command(args: &[String]) -> Result<String> {
    let arg = |i: usize| {
        args.get(i)
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("update-db: missing argument"))
    };
    match args.first().map(String::as_str) {
        Some("snapshot") => Ok(if db_snapshot(&arg(1)?, &arg(2)?)? {
            "created".into()
        } else {
            "absent".into()
        }),
        Some("count") => Ok(match db_migrations(&arg(1)?)? {
            Some(n) => n.to_string(),
            None => "none".into(),
        }),
        Some("prune") => {
            let keep: usize = args
                .get(2)
                .and_then(|k| k.parse().ok())
                .ok_or_else(|| anyhow::anyhow!("update-db prune: keep must be a number"))?;
            let protected: Vec<PathBuf> = args.iter().skip(3).map(PathBuf::from).collect();
            Ok(format!(
                "pruned {}",
                prune_snapshots(&arg(1)?, keep, &protected)
            ))
        }
        Some("restore") => {
            db_restore(&arg(1)?, &arg(2)?)?;
            Ok("restored".into())
        }
        other => Err(anyhow::anyhow!("update-db: unknown operation {other:?}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(path: &Path, rows: usize) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE schema_version (version TEXT); CREATE TABLE t (v TEXT);",
        )
        .unwrap();
        for i in 0..rows {
            conn.execute("INSERT INTO schema_version VALUES (?1)", [i.to_string()])
                .unwrap();
        }
        conn.execute("INSERT INTO t VALUES ('hello')", []).unwrap();
        // keep the connection (and its WAL) alive across the snapshot, like the daemon does
        std::mem::forget(conn);
    }

    #[test]
    fn snapshot_of_live_wal_db_restores_keeps_the_old_copy_and_counts_migrations() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("vault.db");
        make_db(&db, 3);
        assert_eq!(db_migrations(&db).unwrap(), Some(3));

        let snap = d.path().join("snaps/snap.db");
        assert!(db_snapshot(&db, &snap).unwrap());
        assert_eq!(
            std::fs::metadata(&snap).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("INSERT INTO t VALUES ('later')", []).unwrap();
        conn.execute("INSERT INTO schema_version VALUES ('x')", [])
            .unwrap();
        drop(conn);
        assert_eq!(db_migrations(&db).unwrap(), Some(4));

        db_restore(&db, &snap).unwrap();
        assert_eq!(db_migrations(&db).unwrap(), Some(3));
        let conn = rusqlite::Connection::open(&db).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            db_migrations(&with_suffix(&db, ".pre-restore")).unwrap(),
            Some(4),
            "post-snapshot writes stay recoverable"
        );
    }

    #[test]
    fn restore_refuses_a_planted_symlink_for_its_temp_file() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("vault.db");
        make_db(&db, 1);
        let snap = d.path().join("snap.db");
        db_snapshot(&db, &snap).unwrap();
        let victim = d.path().join("victim");
        std::fs::write(&victim, "keep me").unwrap();
        std::os::unix::fs::symlink(&victim, with_suffix(&db, ".restore")).unwrap();
        // the stale link is replaced, never written through
        db_restore(&db, &snap).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
    }

    #[test]
    fn missing_db_is_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("none.db");
        assert!(!db_snapshot(&missing, &d.path().join("s.db")).unwrap());
        assert_eq!(db_migrations(&missing).unwrap(), None);
    }

    #[test]
    fn prune_keeps_newest_and_protected_and_skips_symlinked_dirs() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("snaps");
        std::fs::create_dir(&dir).unwrap();
        for n in 1..=5 {
            std::fs::write(dir.join(format!("vault-{n}.db")), "x").unwrap();
        }
        std::fs::write(dir.join("other.txt"), "x").unwrap();
        let protected = vec![dir.join("vault-1.db")];
        assert_eq!(prune_snapshots(&dir, 2, &protected), 2);
        // oldest three are over the limit; the protected one survives
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["other.txt", "vault-1.db", "vault-4.db", "vault-5.db"]
        );

        let link = d.path().join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert_eq!(
            prune_snapshots(&link, 0, &[]),
            0,
            "symlinked dir is left alone"
        );
    }

    #[test]
    fn command_protocol() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("vault.db");
        make_db(&db, 2);
        let snap = d.path().join("s.db");
        let sv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let (db_s, snap_s) = (db.to_str().unwrap(), snap.to_str().unwrap());
        assert_eq!(run_command(&sv(&["count", db_s])).unwrap(), "2");
        assert_eq!(
            run_command(&sv(&["snapshot", db_s, snap_s])).unwrap(),
            "created"
        );
        assert_eq!(
            run_command(&sv(&["restore", db_s, snap_s])).unwrap(),
            "restored"
        );
        assert!(run_command(&sv(&["bogus"])).is_err());
        assert!(run_command(&sv(&["count"])).is_err());
    }
}
