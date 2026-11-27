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
use std::path::PathBuf;
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
    /// Protected: unconsolidated memories with access_count > 0 get a partial restore,
    /// since being served to a session signals ongoing relevance.
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
            // An older row may hold an integer timestamp. Reading it as text would fail the
            // whole forgetting cycle, so take whatever is stored and render it.
            let rows = stmt.query_map([], |row| {
                let created_at = match row.get::<_, rusqlite::types::Value>(1)? {
                    rusqlite::types::Value::Text(text) => text,
                    rusqlite::types::Value::Integer(n) => n.to_string(),
                    _ => String::new(),
                };
                Ok((row.get::<_, String>(0)?, created_at))
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
        // Read the vault once: every insight note below is looked up in the same index.
        let backlinks =
            hq_vault::relink::backlink_index(&hq_vault::relink::snapshot(&self.vault_path));

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

            // The notes that link to the insight, however the link is written.
            let link_count = count_linking_notes(&backlinks, &note_path);

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

/// How many notes link to `note_path`, by wikilink or markdown link, with a heading, folder or
/// alias or without. A link that could mean another note of the same name is not counted: the
/// count decides what the forgetter keeps, so it errs on the side of fewer.
fn count_linking_notes(
    index: &std::collections::HashMap<String, Vec<hq_vault::relink::Backlink>>,
    note_path: &str,
) -> usize {
    let mut paths: Vec<&str> = index
        .get(note_path)
        .into_iter()
        .flatten()
        .filter(|b| !b.ambiguous)
        .map(|b| b.path.as_str())
        .collect();
    paths.dedup();
    paths.len()
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
    fn a_note_counts_every_other_note_that_links_to_it_however_the_link_is_written() {
        let dir = std::env::temp_dir().join(format!(
            "hq-forgetter-backlinks-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |rel: &str, text: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write(
            "Notebooks/Memories/2026-11-20-12-00-00-insight.md",
            "insight",
        );
        // The four spellings the old `[[stem]]` substring match could not see, plus the plain one.
        write("Notebooks/A.md", "[[2026-11-20-12-00-00-insight#Part]]");
        write(
            "Notebooks/B.md",
            "[[Memories/2026-11-20-12-00-00-insight|the insight]]",
        );
        write(
            "Notebooks/C.md",
            "[t](Memories/2026-11-20-12-00-00-insight.md)",
        );
        write(
            "Notebooks/D.md",
            "[[2026-11-20-12-00-00-insight]] and again [[2026-11-20-12-00-00-insight]]",
        );
        // Not links: code, and another note with a similar name.
        write(
            "Notebooks/E.md",
            "`[[2026-11-20-12-00-00-insight]]` [[2026-11-20-12-00-00-insights]]",
        );

        let index = hq_vault::relink::backlink_index(&hq_vault::relink::snapshot(&dir));
        assert_eq!(
            count_linking_notes(&index, "Notebooks/Memories/2026-11-20-12-00-00-insight.md"),
            4,
            "four notes link to it; a note that links twice counts once"
        );
        assert_eq!(count_linking_notes(&index, "Notebooks/Nope.md"), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

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

    // ── Decay and pruning ────────────────────────────────────────────────

    use crate::db::{StoreMemoryParams, store_consolidation, store_memory};

    fn temp_vault() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn store(db: &Database, summary: &str, importance: f64) -> i64 {
        store_memory(
            db,
            &StoreMemoryParams {
                source: "chat-turn",
                harness: "web",
                raw_text: "",
                summary,
                entities: &[],
                topics: &[],
                importance,
                user_id: None,
            },
        )
        .unwrap()
    }

    fn age(db: &Database, id: i64, days: i64) {
        let at = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
        db.with_conn(|c| {
            c.execute(
                "UPDATE memories SET created_at = ?1 WHERE id = ?2",
                rusqlite::params![at, id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn importance_of(db: &Database, id: i64) -> Option<f64> {
        db.with_conn(|c| {
            Ok(
                c.query_row("SELECT importance FROM memories WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .ok(),
            )
        })
        .unwrap()
    }

    #[test]
    fn old_unconsolidated_memories_decay_and_fresh_ones_do_not() {
        let db = Database::open_memory().unwrap();
        let vault = temp_vault();
        let old = store(&db, "old", 0.5);
        let fresh = store(&db, "fresh", 0.5);
        age(&db, old, 10);
        age(&db, fresh, 2);

        let result = MemoryForgetter::new(db.clone(), vault.path().to_path_buf())
            .run_cycle()
            .unwrap();

        assert_eq!(result.decayed, 1);
        assert!((importance_of(&db, old).unwrap() - 0.485).abs() < 1e-9);
        assert!((importance_of(&db, fresh).unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_memory_that_was_served_regains_some_importance() {
        let db = Database::open_memory().unwrap();
        let vault = temp_vault();
        let used = store(&db, "used", 0.5);
        let unused = store(&db, "unused", 0.5);
        age(&db, used, 20);
        age(&db, unused, 20);
        crate::touch_memory(&db, used).unwrap();

        MemoryForgetter::new(db.clone(), vault.path().to_path_buf())
            .run_cycle()
            .unwrap();

        // Both lose 0.015, and only the one that was served gets 0.0075 back.
        assert!((importance_of(&db, unused).unwrap() - 0.485).abs() < 1e-9);
        assert!((importance_of(&db, used).unwrap() - 0.4925).abs() < 1e-9);
    }

    #[test]
    fn weak_old_memories_are_pruned_to_the_archive_and_the_rest_stay() {
        let db = Database::open_memory().unwrap();
        let vault = temp_vault();
        let weak_old = store(&db, "WEAK-OLD-FACT", 0.04);
        let weak_young = store(&db, "weak but young", 0.04);
        let strong_old = store(&db, "strong and old", 0.9);
        age(&db, weak_old, 70);
        age(&db, weak_young, 20);
        age(&db, strong_old, 70);

        let result = MemoryForgetter::new(db.clone(), vault.path().to_path_buf())
            .run_cycle()
            .unwrap();

        assert_eq!(result.pruned, 1);
        assert!(importance_of(&db, weak_old).is_none(), "pruned");
        assert!(importance_of(&db, weak_young).is_some(), "under 60 days");
        assert!(importance_of(&db, strong_old).is_some(), "important");

        let archive_dir = vault.path().join("_archive").join("memories");
        let archived: String = std::fs::read_dir(&archive_dir)
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .collect();
        assert!(archived.contains("WEAK-OLD-FACT"), "{archived}");
        assert!(!archived.contains("strong and old"));
    }

    /// Nothing writes the `[[...-insight]]` links the classifier counts, so a consolidated memory
    /// always lands in the fast-decay tier. Pinned here so replacing that rule is a visible edit.
    #[test]
    fn a_consolidated_memory_with_no_backlinks_decays_at_the_fast_rate() {
        let db = Database::open_memory().unwrap();
        let vault = temp_vault();
        let id = store(&db, "consolidated", 0.5);
        age(&db, id, 10);
        store_consolidation(&db, &[id], "an insight", &[]).unwrap();

        MemoryForgetter::new(db.clone(), vault.path().to_path_buf())
            .run_cycle()
            .unwrap();

        assert!((importance_of(&db, id).unwrap() - 0.45).abs() < 1e-9);
    }

    #[test]
    fn a_consolidation_with_an_integer_timestamp_does_not_fail_the_cycle() {
        let db = Database::open_memory().unwrap();
        let vault = temp_vault();
        let id = store(&db, "legacy", 0.5);
        age(&db, id, 10);
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO consolidations (memory_ids, insight, created_at) VALUES (?1, 'old', 1700000000)",
                [format!("[{id}]")],
            )?;
            Ok(())
        })
        .unwrap();

        let result = MemoryForgetter::new(db, vault.path().to_path_buf()).run_cycle();

        assert!(result.is_ok(), "{:?}", result.err());
    }
}
