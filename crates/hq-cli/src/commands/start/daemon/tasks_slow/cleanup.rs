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

/// Daily vault cleanup: purge expired trash and clear dead presence files.
pub async fn run_vault_cleanup(vault_path: &Path, _db: &Database) -> Result<()> {
    if !has_run_today(vault_path, "vault-cleanup") {
        let mut actions = Vec::new();

        // Purge _trash/ folders past the 30-day retention window
        match hq_vault::reorg::purge_trash(vault_path, 30) {
            Ok(0) => {}
            Ok(n) => actions.push(format!("Purged {n} expired _trash day-folders")),
            Err(e) => tracing::warn!(%e, "vault-cleanup: trash purge failed"),
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
