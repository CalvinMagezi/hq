//! Vault cleanup and maintenance tasks: thread log rotation, DB vacuuming,
//! and cap enforcement.

use anyhow::Result;
use hq_db::Database;
use std::path::Path;
use tracing::info;

use super::super::helpers::{has_run_today, mark_run_today};

/// Bound `_threads/*.jsonl` growth. Per-turn memory ingestion
/// (`hq_agent::native_hq::wire_post_turn_ingestion`) already captures every
/// substantive turn durably at write time, so trimming these logs costs no
/// continuity beyond the short-term rolling window
/// (`load_merged_thread`/`MAX_MERGED_MESSAGES`) — the trimmed-off lines are
/// discarded here, not re-ingested, to avoid double-ingesting the same
/// conversation through two paths.
pub async fn run_thread_log_rotation(vault_path: &Path) -> Result<()> {
    const KEEP_LINES: usize = 200;
    let threads_dir = vault_path.join("_threads");
    if !threads_dir.exists() {
        return Ok(());
    }

    let mut rotated = 0u32;
    if let Ok(entries) = std::fs::read_dir(&threads_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            match hq_agent::threads::rotate_thread_file(&path, KEEP_LINES) {
                Ok(trimmed) if !trimmed.is_empty() => {
                    rotated += 1;
                    info!(
                        file = %path.display(),
                        trimmed_lines = trimmed.len(),
                        "thread-log-rotation: trimmed"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(%e, file = %path.display(), "thread-log-rotation: failed")
                }
            }
        }
    }
    if rotated > 0 {
        info!(files = rotated, "thread-log-rotation: complete");
    }
    Ok(())
}

/// How long the record of who wrote a deleted note is kept, in days.
const DELETED_PROVENANCE_DAYS: i64 = 90;

/// Daily vault cleanup: purge expired trash and provenance records of deleted notes, and clear
/// dead presence files.
pub async fn run_vault_cleanup(vault_path: &Path, db: &Database) -> Result<()> {
    if !has_run_today(vault_path, "vault-cleanup") {
        let mut actions = Vec::new();

        // Purge _trash/ folders past the 30-day retention window
        match hq_vault::reorg::purge_trash(vault_path, 30) {
            Ok(0) => {}
            Ok(n) => actions.push(format!("Purged {n} expired _trash day-folders")),
            Err(e) => tracing::warn!(%e, "vault-cleanup: trash purge failed"),
        }

        // Who wrote a note is kept for a while after the note is deleted, then dropped.
        match db.with_conn(|conn| hq_db::provenance::purge_deleted(conn, DELETED_PROVENANCE_DAYS)) {
            Ok(0) => {}
            Ok(n) => actions.push(format!(
                "Dropped {n} provenance record(s) of notes deleted over {DELETED_PROVENANCE_DAYS} days ago"
            )),
            Err(e) => tracing::warn!(%e, "vault-cleanup: provenance purge failed"),
        }

        // Raw skill invocations past the retention window become monthly counts; the skills and
        // their audit trail are not touched.
        match db.with_conn(|conn| {
            hq_db::skill_invocations::apply_retention(
                conn,
                hq_db::skill_invocations::RETENTION_DAYS,
            )
        }) {
            Ok(kept) if kept.rolled_up == 0 => {}
            Ok(kept) => actions.push(format!(
                "Rolled {} skill invocation(s) older than {} days into monthly counts",
                kept.rolled_up,
                hq_db::skill_invocations::RETENTION_DAYS
            )),
            Err(e) => tracing::warn!(%e, "vault-cleanup: skill telemetry retention failed"),
        }

        // Garbage-collect presence files whose process is long gone.
        match hq_core::heartbeat::detect_dead_harnesses(vault_path, 3600) {
            Ok(dead) if !dead.is_empty() => {
                let mut cleared = 0;
                for h in &dead {
                    if hq_core::heartbeat::clear_heartbeat(vault_path, &h.0).is_ok() {
                        cleared += 1;
                    }
                }
                if cleared > 0 {
                    actions.push(format!("Cleared {cleared} dead presence files"));
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(%e, "vault-cleanup: presence sweep failed"),
        }

        if !actions.is_empty() {
            info!(?actions, "vault-cleanup: completed");
        }

        mark_run_today(vault_path, "vault-cleanup");
    }
    Ok(())
}

/// Weekly VACUUM and WAL checkpoint for all vault SQLite databases.
/// Reclaims fragmented pages from vault.db and checkpoints large WAL files.
pub async fn run_db_vacuum(vault_path: &Path, db: &Database) -> Result<()> {
    // 1. PRAGMA incremental_vacuum on the main vault.db
    db.with_conn(|conn| {
        conn.execute_batch("PRAGMA incremental_vacuum(2000);")?;
        Ok(())
    })?;
    info!("vault.db incremental vacuum complete");

    // 2. Walk all .db files in _embeddings/ and checkpoint WAL if >50MB
    let embeddings_dir = vault_path.join("_embeddings");
    if embeddings_dir.exists() {
        let entries = std::fs::read_dir(&embeddings_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("db") {
                continue;
            }
            let wal_path = path.with_extension("db-wal");
            let wal_size = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
            if wal_size > 50 * 1024 * 1024 {
                // WAL > 50MB — force checkpoint and truncate
                match rusqlite::Connection::open(&path) {
                    Ok(conn) => {
                        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
                        info!(
                            db = %path.display(),
                            wal_mb = wal_size / 1024 / 1024,
                            "WAL truncated"
                        );
                    }
                    Err(e) => {
                        // DB may be locked — non-fatal, log and skip
                        info!(db = %path.display(), error = %e, "skipped WAL checkpoint (locked)");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Hourly vault size governance. Enforces soft (3 GB) and hard (5 GB) caps.
/// Under soft cap: logs metrics only.
/// Over soft cap: vacuums the databases. Memory pruning belongs to memory-forgetting.
/// Over hard cap: also logs a warning.
pub async fn run_vault_cap_enforcer(vault_path: &Path, db: &Database) -> Result<()> {
    // 1. Measure vault size
    let vault_size_bytes = measure_dir_size(vault_path);
    let vault_size_gb = vault_size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

    const SOFT_CAP_GB: f64 = 3.0;
    const HARD_CAP_GB: f64 = 5.0;

    info!(
        vault_gb = vault_size_gb,
        soft_cap = SOFT_CAP_GB,
        "vault cap check"
    );

    if vault_size_gb < SOFT_CAP_GB {
        // Under soft cap — nothing to do
        return Ok(());
    }

    // 2. Over soft cap: trigger all cleanup tasks
    info!(
        vault_gb = vault_size_gb,
        "vault over soft cap — triggering cleanup cascade"
    );

    let _ = run_db_vacuum(vault_path, db).await;

    // 3. Over hard cap: CRITICAL alert
    if vault_size_gb >= HARD_CAP_GB {
        tracing::warn!(
            vault_gb = vault_size_gb,
            hard_cap = HARD_CAP_GB,
            "vault HARD CAP exceeded"
        );
    }

    Ok(())
}

/// Recursively measure total size of a directory in bytes.
/// Excludes the Rust target/ directory to avoid counting build artifacts.
fn measure_dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            // Skip build artifact directories
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            if name == "target"
                || name == "node_modules"
                || name == ".git"
                || name == "_embeddings"
                || name == "_data"
            {
                continue;
            }
            if p.is_file() {
                total += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            } else if p.is_dir() {
                total += measure_dir_size(&p);
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_daily_cleanup_ages_out_skill_telemetry_and_provenance_of_deleted_notes() {
        let vault = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            use hq_db::skill_invocations::{InvocationTrigger, log_invocation};
            log_invocation(c, "sheets", "s-old", InvocationTrigger::LoadSkill)?;
            log_invocation(c, "sheets", "s-new", InvocationTrigger::LoadSkill)?;
            c.execute(
                "UPDATE skill_invocations SET loaded_at = datetime('now', '-200 days') WHERE session_id = 's-old'",
                [],
            )?;
            // A note deleted long ago keeps its author record only for the retention period.
            let who = hq_db::provenance::Attribution::default();
            hq_db::provenance::record(c, "Notebooks/gone.md", "h1", &who)?;
            hq_db::provenance::record(c, "Notebooks/recent.md", "h2", &who)?;
            hq_db::provenance::mark_deleted(c, "Notebooks/gone.md")?;
            hq_db::provenance::mark_deleted(c, "Notebooks/recent.md")?;
            c.execute(
                "UPDATE note_provenance SET deleted_at = '2020-01-01T00:00:00+00:00' WHERE path = 'Notebooks/gone.md'",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        run_vault_cleanup(vault.path(), &db).await.unwrap();

        let (raw, counted, rollups, provenance): (i64, u32, i64, i64) = db
            .with_conn(|c| {
                Ok((
                    c.query_row("SELECT COUNT(*) FROM skill_invocations", [], |r| r.get(0))?,
                    hq_db::skill_invocations::count_invocations(c, "sheets")?,
                    c.query_row("SELECT COUNT(*) FROM skill_invocation_rollups", [], |r| {
                        r.get(0)
                    })?,
                    c.query_row("SELECT COUNT(*) FROM note_provenance", [], |r| r.get(0))?,
                ))
            })
            .unwrap();
        assert_eq!(raw, 1, "the old raw row is gone");
        assert_eq!(counted, 2, "the count still includes it");
        assert_eq!(rollups, 1);
        assert_eq!(
            provenance, 1,
            "the long-deleted note's record is dropped, the recent one kept"
        );
    }
}
