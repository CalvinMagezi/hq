//! MemoryForgetter — implements "Synaptic Homeostasis" for vault memory.
//!
//! Inspired by the Synaptic Homeostasis Hypothesis (SHY): the brain doesn't
//! passively forget — it actively scales down weak synapses during sleep to
//! preserve the signal-to-noise ratio of important memories.
//!
//! Applied here: a daily decay cycle with schema-guided tiered rates:
//!   1. Standard decay (1.5%/day) for unconsolidated memories
//!   2. Accelerated decay (5%/day) for consolidated memories with low vault connectivity
//!   3. Protected decay (0.5%/day) for consolidated memories anchoring well-linked schemas
//!   4. Resist decay for high-access and replayed memories
//!   5. Prune memories below threshold after 60 days
//!
//! Ported from vault-memory/src/forgetter.ts.

use anyhow::Result;
use hq_db::Database;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

use crate::db::{decay_old_memories, get_memory_stats};
use crate::types::MemoryStats;

/// Minimum backlinks on an insight note to qualify as "well-connected schema".
const HIGH_LINK_THRESHOLD: usize = 3;

/// Result of a forgetting cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgetterResult {
    /// Memories whose importance was reduced
    pub decayed: i64,
    /// Memories deleted (importance fell below threshold)
    pub pruned: i64,
    /// Stats after the cycle
    pub stats_after: MemoryStats,
}

/// Memory forgetter with schema-guided tiered decay.
pub struct MemoryForgetter {
    db: Database,
    vault_path: PathBuf,
}

impl MemoryForgetter {
    pub fn new(db: Database, vault_path: PathBuf) -> Self {
        Self { db, vault_path }
    }

    /// Run one forgetting cycle with schema-guided tiered decay.
    ///
    /// Tier 1 (Standard): 1.5%/day for unconsolidated memories >7 days old.
    ///   A 0.5-importance memory takes ~30 days to reach the 0.05 prune threshold.
    ///
    /// Tier 2 (Accelerated): 5%/day for consolidated memories whose insight notes
    ///   have few backlinks (<3). The core insight has been extracted — raw data
    ///   can be cleaned up faster.
    ///
    /// Tier 3 (Protected): 0.5%/day for consolidated memories whose insight notes
    ///   are well-linked (3+ backlinks). These anchor important knowledge schemas
    ///   and should resist decay strongly.
    ///
    /// Protected: memories with access_count > 0 or replay_count > 0 get partial
    /// restore, since being accessed/replayed signals ongoing relevance.
    pub fn run_cycle(&self) -> Result<ForgetterResult> {
        // ── Tier 1: Standard decay — unconsolidated memories ──────────────
        let standard_decayed = decay_old_memories(&self.db, 0.015, 7)?;

        // ── Tiers 2 & 3: Schema-guided decay for consolidated memories ───
        let (low_link, high_link) = self.classify_consolidated_memories()?;
        let cutoff_7d = cutoff_iso(7);

        let mut accelerated_decayed = 0i64;
        let mut protected_decayed = 0i64;

        if !low_link.is_empty() {
            accelerated_decayed = self.db.with_conn(|conn| {
                let placeholders: String = low_link.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "UPDATE memories SET importance = MAX(0.01, importance - 0.05) WHERE id IN ({placeholders}) AND created_at < ?"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = low_link
                    .iter()
                    .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                params.push(Box::new(cutoff_7d.clone()));
                let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                Ok(stmt.execute(param_refs.as_slice())? as i64)
            })?;
        }

        if !high_link.is_empty() {
            protected_decayed = self.db.with_conn(|conn| {
                let placeholders: String = high_link.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "UPDATE memories SET importance = MAX(0.01, importance - 0.005) WHERE id IN ({placeholders}) AND created_at < ?"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = high_link
                    .iter()
                    .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                params.push(Box::new(cutoff_7d.clone()));
                let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                Ok(stmt.execute(param_refs.as_slice())? as i64)
            })?;
        }

        let decayed = standard_decayed + accelerated_decayed + protected_decayed;

        // ── Access-count protection ──────────────────────────────────────
        let cutoff_14d = cutoff_iso(14);
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE memories SET importance = MIN(1.0, importance + 0.0075) WHERE access_count > 0 AND consolidated = 0 AND created_at < ?1",
                [&cutoff_14d],
            )?;
            Ok(())
        })?;

        // ── Replay-count protection ──────────────────────────────────────
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE memories SET importance = MIN(1.0, importance + 0.0075) WHERE replay_count > 0 AND consolidated = 0 AND created_at < ?1",
                [&cutoff_14d],
            )?;
            Ok(())
        })?;

        // ── Prune memories below relevance floor ─────────────────────────
        let pruned = self.archive_and_prune(0.05, 60)?;

        let _ = self.archive_stale_concept_pages(&self.db);

        let stats_after = get_memory_stats(&self.db)?;

        if decayed > 0 || pruned > 0 {
            info!(decayed, pruned, "Forgetter cycle complete");
        }

        Ok(ForgetterResult {
            decayed,
            pruned,
            stats_after,
        })
    }

    /// Archives (never hard-deletes) concept pages that are both stale
    /// (`updated` older than `PRUNE_AGE_DAYS`, the same 60-day threshold
    /// `archive_and_prune` already uses for memories) and disconnected (zero
    /// derived edges after a fresh `derive_entity_index`). Connectivity is
    /// checked, not importance, because concept pages have no importance
    /// score — a well-linked page is presumed relevant regardless of age.
    pub fn archive_stale_concept_pages(&self, db: &Database) -> Result<usize> {
        const PRUNE_AGE_DAYS: i64 = 60;

        let vault = hq_vault::VaultClient::new(self.vault_path.clone())?;
        crate::concept_pages::derive_entity_index(db, &vault)?;

        let cutoff = chrono::Utc::now() - chrono::Duration::days(PRUNE_AGE_DAYS);
        let pages = vault.list_notes_recursive("_graph")?;
        let mut archived = 0usize;

        for rel_path in pages {
            if rel_path.starts_with("_graph/_archive/") {
                continue;
            }
            let Ok(note) = vault.read_note(&rel_path) else {
                continue;
            };

            let updated_str = note
                .frontmatter
                .get("updated")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let Ok(updated) = chrono::DateTime::parse_from_rfc3339(updated_str) else {
                continue;
            };
            if updated.with_timezone(&chrono::Utc) >= cutoff {
                continue;
            }

            // Look up by canonical_key (matches entity_nodes.canonical, per Task 1's
            // fix), NOT canonical_slug — those two diverge for any multi-word title.
            let canonical = crate::concept_pages::canonical_key(&note.title);
            let edge_count: i64 = db.with_conn({
                let canonical = canonical.clone();
                move |conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM entity_edges e
                         JOIN entity_nodes n ON n.id = e.source_id OR n.id = e.target_id
                         WHERE n.canonical = ?1",
                        [&canonical],
                        |r| r.get(0),
                    )
                    .map_err(Into::into)
                }
            })?;
            if edge_count > 0 {
                continue;
            }

            crate::concept_pages::archive_concept_page(
                &vault,
                &note.title,
                "decayed: stale and disconnected",
            )?;
            let full_path = self.vault_path.join(&rel_path);
            std::fs::remove_file(&full_path)?;
            archived += 1;
        }

        Ok(archived)
    }

    /// Archive memories below the importance threshold to `_archive/memories/YYYY-MM.jsonl`
    /// then delete them from SQLite. Preserves knowledge while reclaiming database space.
    fn archive_and_prune(&self, threshold: f64, min_age_days: i64) -> Result<i64> {
        use std::io::Write;

        let cutoff = cutoff_iso(min_age_days);

        // 1. Find memories to prune
        let to_prune: Vec<(i64, String, String, f64)> = self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, summary, topics, importance FROM memories WHERE importance <= ?1 AND created_at < ?2 LIMIT 200"
            )?;
            let rows = stmt.query_map(rusqlite::params![threshold, cutoff], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,   // topics stored as JSON string
                    row.get::<_, f64>(3)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(anyhow::Error::from)
        })?;

        if to_prune.is_empty() {
            return Ok(0);
        }

        // 2. Write to archive file
        let archive_dir = self.vault_path.join("_archive").join("memories");
        std::fs::create_dir_all(&archive_dir)?;
        let month_tag = chrono::Utc::now().format("%Y-%m").to_string();
        let archive_path = archive_dir.join(format!("{}.jsonl", month_tag));

        let mut archive_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&archive_path)?;

        let archived_at = chrono::Utc::now().to_rfc3339();
        let mut ids_to_delete: Vec<i64> = Vec::new();

        for (id, summary, topics_json, importance) in &to_prune {
            let entry = format!(
                "{{\"id\":{},\"importance\":{:.3},\"topics\":{},\"archived_at\":\"{}\",\"summary\":{}}}\n",
                id,
                importance,
                topics_json,
                archived_at,
                serde_json::to_string(summary).unwrap_or_else(|_| "\"\"".to_string()),
            );
            archive_file.write_all(entry.as_bytes())?;
            ids_to_delete.push(*id);
        }

        // 3. Delete archived rows from SQLite
        let deleted = self.db.with_conn(|conn| {
            let mut total = 0i64;
            for id in &ids_to_delete {
                total += conn
                    .execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])?
                    as i64;
            }
            Ok(total)
        })?;

        info!(
            archived = ids_to_delete.len(),
            deleted,
            archive = %archive_path.display(),
            "memory forgetter: archived and pruned weak memories"
        );

        Ok(deleted)
    }

    /// Classify consolidated memories by the link density of their insight notes.
    ///
    /// Queries the consolidations table, derives each insight note's path,
    /// then counts backlinks by scanning for wikilinks in the vault.
    /// Memories whose insight notes have 3+ backlinks are "high link"
    /// (schema anchors); the rest are "low link" (insight extracted, raw data expendable).
    fn classify_consolidated_memories(&self) -> Result<(Vec<i64>, Vec<i64>)> {
        let consolidations = self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT memory_ids, created_at FROM consolidations ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })?;

        if consolidations.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let mut low_link: Vec<i64> = Vec::new();
        let mut high_link: Vec<i64> = Vec::new();

        for (ids_json, created_at) in &consolidations {
            let source_ids: Vec<i64> = serde_json::from_str(ids_json).unwrap_or_default();
            if source_ids.is_empty() {
                continue;
            }

            // Derive insight note path — matches consolidator pattern
            let date = &created_at[..10.min(created_at.len())];
            let time = if created_at.len() >= 19 {
                created_at[11..19].replace(':', "-")
            } else {
                "00-00-00".into()
            };
            let note_path = format!("Notebooks/Memories/{date}-{time}-insight.md");

            // Count backlinks in the vault by scanning for [[note_path]] references
            let link_count = count_backlinks(&self.vault_path, &note_path);

            let bucket = if link_count >= HIGH_LINK_THRESHOLD {
                &mut high_link
            } else {
                &mut low_link
            };
            for id in &source_ids {
                bucket.push(*id);
            }
        }

        Ok((low_link, high_link))
    }
}

/// Count files in the vault that contain a wikilink to the given note path.
/// This is a simplified version — in production, you'd use the vault graph.
fn count_backlinks(vault_path: &Path, note_path: &str) -> usize {
    let note_name = std::path::Path::new(note_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    if note_name.is_empty() {
        return 0;
    }

    let search_pattern = format!("[[{note_name}]]");
    let notebooks_dir = vault_path.join("Notebooks");

    if !notebooks_dir.exists() {
        return 0;
    }

    count_files_containing(&notebooks_dir, &search_pattern)
}

/// Recursively count markdown files containing a given string.
fn count_files_containing(dir: &std::path::Path, pattern: &str) -> usize {
    let mut count = 0;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += count_files_containing(&path, pattern);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md")
            && let Ok(content) = std::fs::read_to_string(&path)
            && content.contains(pattern)
        {
            count += 1;
        }
    }
    count
}

fn cutoff_iso(days: i64) -> String {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
    cutoff.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::Database;

    #[test]
    fn archives_a_page_with_no_edges_older_than_the_prune_threshold() {
        let db = Database::open_memory().unwrap();

        let dir = std::env::temp_dir().join(format!(
            "hq-forgetter-concept-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let vault = hq_vault::VaultClient::new(dir.clone()).unwrap();

        // Isolated page: no links in, no links out.
        crate::concept_pages::upsert_concept_page(&vault, "Orphan Concept", "concept", &[], "test")
            .unwrap();
        // Backdate its `updated` frontmatter past the prune threshold.
        let mut note = vault.read_note("_graph/orphan-concept.md").unwrap();
        note.frontmatter.insert(
            "updated".to_string(),
            serde_yaml::Value::String("2020-01-01T00:00:00Z".to_string()),
        );
        vault.write_note("_graph/orphan-concept.md", &note).unwrap();

        // Connected, but ALSO stale: must survive because the connectivity check
        // has to actually run (and find the edge) for this test to discriminate
        // canonical_key from canonical_slug lookups.
        crate::concept_pages::upsert_concept_page(
            &vault,
            "Linked A",
            "concept",
            &["Linked B".to_string()],
            "test",
        )
        .unwrap();
        crate::concept_pages::upsert_concept_page(&vault, "Linked B", "concept", &[], "test")
            .unwrap();
        // Backdate "Linked A" past the prune threshold too, same as the orphan above.
        let mut note = vault.read_note("_graph/linked-a.md").unwrap();
        note.frontmatter.insert(
            "updated".to_string(),
            serde_yaml::Value::String("2020-01-01T00:00:00Z".to_string()),
        );
        vault.write_note("_graph/linked-a.md", &note).unwrap();

        let forgetter = MemoryForgetter::new(db.clone(), dir);
        let archived = forgetter.archive_stale_concept_pages(&db).unwrap();

        assert_eq!(
            archived, 1,
            "only the isolated, stale page should be archived; the stale-but-connected page must survive via the canonical_key connectivity check"
        );
        assert!(
            !vault.note_exists("_graph/orphan-concept.md"),
            "live orphan page must be removed after archiving"
        );
        assert!(
            vault.note_exists("_graph/linked-a.md"),
            "stale but connected pages must survive because canonical_key correctly finds their entity_edges row"
        );
    }
}
