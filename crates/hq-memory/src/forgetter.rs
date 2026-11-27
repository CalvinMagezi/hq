//! MemoryForgetter: what stops being brought up on its own, and when.
//!
//! HQ used to lower a memory's importance every day and delete it for good once it fell low
//! enough, about two months after it was saved. That quietly lost most of what HQ learned.
//! Now nothing is deleted by HQ. A note leaves automatic recall in two cases, and the owner can
//! bring either back ("Use it again"):
//!
//! 1. It has gone unused for a year and is not pinned.
//! 2. There are more than [`MAX_ACTIVE`] in use, and it is the longest unused of the unpinned ones.
//!
//! An aged-out note keeps its `archived` standing: it is still listed and searchable on the
//! owner's page, it is just never brought up in a chat unasked. A pinned note never ages out.
//!
//! The same daily pass clears the full text of old chat turns that earlier versions stored and
//! never read, and archives concept pages that have gone stale and lead nowhere.

use anyhow::Result;
use hq_db::Database;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::info;

use crate::db::get_memory_stats;
use crate::types::MemoryStats;

/// How long an unused, unpinned note stays in use, in days.
pub const KEEP_DAYS: i64 = 365;
/// The most notes kept in use. Past this the longest unused are put away, so a very long history
/// cannot slow every chat down.
pub const MAX_ACTIVE: i64 = 20_000;

/// Result of a forgetting cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgetterResult {
    /// Notes put away because a year passed without them being used.
    pub aged_out: i64,
    /// Notes put away because there were more than [`MAX_ACTIVE`] in use.
    pub capped: i64,
    /// Old turn texts that were cleared.
    pub scrubbed: i64,
    /// Stats after the cycle
    pub stats_after: MemoryStats,
}

/// Memory forgetter: puts unused notes away, deletes none.
pub struct MemoryForgetter {
    db: Database,
    vault_path: PathBuf,
}

impl MemoryForgetter {
    pub fn new(db: Database, vault_path: PathBuf) -> Self {
        Self { db, vault_path }
    }

    /// Run one pass. Safe to run any number of times.
    pub fn run_cycle(&self) -> Result<ForgetterResult> {
        let aged_out = age_out(&self.db, KEEP_DAYS)?;
        let capped = cap_active(&self.db, MAX_ACTIVE)?;
        let scrubbed = scrub_turn_text(&self.db)?;
        let _ = self.archive_stale_concept_pages(&self.db);
        let stats_after = get_memory_stats(&self.db)?;
        if aged_out > 0 || capped > 0 || scrubbed > 0 {
            info!(aged_out, capped, scrubbed, "memory forgetter pass complete");
        }
        Ok(ForgetterResult {
            aged_out,
            capped,
            scrubbed,
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
}

/// Puts away the notes that are in use, not pinned, and have gone `days` without being used. A
/// note counts as used when HQ last showed it to the model; one never shown is as old as it is.
fn age_out(db: &Database, days: i64) -> Result<i64> {
    let cutoff = cutoff_iso(days);
    db.with_conn(|conn| {
        Ok(conn.execute(
            "UPDATE memories SET status = 'archived'
              WHERE status = 'active' AND pinned = 0
                AND COALESCE(last_accessed_at, created_at) < ?1",
            [cutoff],
        )? as i64)
    })
}

/// Puts away the longest unused notes beyond `max` that are in use, never a pinned one.
fn cap_active(db: &Database, max: i64) -> Result<i64> {
    db.with_conn(|conn| {
        Ok(conn.execute(
            "UPDATE memories SET status = 'archived' WHERE id IN (
                SELECT id FROM memories WHERE status = 'active' AND pinned = 0
                 ORDER BY COALESCE(last_accessed_at, created_at) ASC
                 LIMIT MAX(0, (SELECT COUNT(*) FROM memories WHERE status = 'active') - ?1))",
            [max],
        )? as i64)
    })
}

/// Clears the stored text of whole chat turns. Earlier versions kept it and nothing ever read it.
fn scrub_turn_text(db: &Database) -> Result<i64> {
    db.with_conn(|conn| {
        Ok(conn.execute("UPDATE memories SET raw_text = '' WHERE raw_text <> ''", [])? as i64)
    })
}

fn cutoff_iso(days: i64) -> String {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
    cutoff.to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{StoreMemoryParams, store_memory};
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

    // ── Putting notes away ───────────────────────────────────────────────

    fn temp_vault() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn store(db: &Database, summary: &str) -> i64 {
        store_memory(
            db,
            &StoreMemoryParams {
                source: "chat-turn",
                harness: "web",
                raw_text: "",
                summary,
                entities: &[],
                topics: &[],
                importance: 0.5,
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

    fn status_of(db: &Database, id: i64) -> Option<String> {
        db.with_conn(|c| {
            Ok(
                c.query_row("SELECT status FROM memories WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .ok(),
            )
        })
        .unwrap()
    }

    fn run(db: &Database) -> ForgetterResult {
        let vault = temp_vault();
        MemoryForgetter::new(db.clone(), vault.path().to_path_buf())
            .run_cycle()
            .unwrap()
    }

    #[test]
    fn a_note_unused_for_a_year_is_put_away_and_a_younger_one_is_not() {
        let db = Database::open_memory().unwrap();
        let old = store(&db, "old note");
        let young = store(&db, "young note");
        age(&db, old, 400);
        age(&db, young, 300);

        let result = run(&db);

        assert_eq!(result.aged_out, 1);
        assert_eq!(status_of(&db, old).as_deref(), Some("archived"));
        assert_eq!(status_of(&db, young).as_deref(), Some("active"));
    }

    #[test]
    fn nothing_is_ever_deleted_however_old_or_unimportant() {
        let db = Database::open_memory().unwrap();
        let id = store(&db, "ancient and unimportant");
        age(&db, id, 5000);
        db.with_conn(|c| {
            c.execute("UPDATE memories SET importance = 0.0 WHERE id = ?1", [id])?;
            Ok(())
        })
        .unwrap();

        run(&db);
        run(&db);

        assert_eq!(
            status_of(&db, id).as_deref(),
            Some("archived"),
            "kept, and only put away"
        );
    }

    #[test]
    fn a_pinned_note_never_ages_out() {
        let db = Database::open_memory().unwrap();
        let pinned = store(&db, "pinned note");
        let plain = store(&db, "plain note");
        for id in [pinned, plain] {
            age(&db, id, 2000);
        }
        db.with_conn(|c| {
            c.execute("UPDATE memories SET pinned = 1 WHERE id = ?1", [pinned])?;
            Ok(())
        })
        .unwrap();

        run(&db);

        assert_eq!(status_of(&db, pinned).as_deref(), Some("active"));
        assert_eq!(status_of(&db, plain).as_deref(), Some("archived"));
    }

    #[test]
    fn a_note_shown_lately_stays_in_use_however_old_it_is() {
        let db = Database::open_memory().unwrap();
        let used = store(&db, "old but used");
        let unused = store(&db, "old and unused");
        for id in [used, unused] {
            age(&db, id, 800);
        }
        crate::touch_memory(&db, used).unwrap();

        run(&db);

        assert_eq!(status_of(&db, used).as_deref(), Some("active"));
        assert_eq!(status_of(&db, unused).as_deref(), Some("archived"));
    }

    #[test]
    fn only_notes_in_use_are_touched() {
        let db = Database::open_memory().unwrap();
        let held = store(&db, "held back");
        let outdated = store(&db, "out of date");
        for id in [held, outdated] {
            age(&db, id, 800);
        }
        db.with_conn(|c| {
            c.execute(
                "UPDATE memories SET status = 'review' WHERE id = ?1",
                [held],
            )?;
            c.execute(
                "UPDATE memories SET status = 'superseded' WHERE id = ?1",
                [outdated],
            )?;
            Ok(())
        })
        .unwrap();

        let result = run(&db);

        assert_eq!(
            result.aged_out, 0,
            "the owner's own choices are left as they made them"
        );
        assert_eq!(status_of(&db, held).as_deref(), Some("review"));
        assert_eq!(status_of(&db, outdated).as_deref(), Some("superseded"));
    }

    #[test]
    fn past_the_cap_the_longest_unused_are_put_away_and_pinned_ones_stay() {
        let db = Database::open_memory().unwrap();
        let ids: Vec<i64> = (0..6).map(|i| store(&db, &format!("note {i}"))).collect();
        for (i, id) in ids.iter().enumerate() {
            age(&db, *id, 10 + 10 * (5 - i as i64)); // note 0 is the oldest
        }
        db.with_conn(|c| {
            c.execute("UPDATE memories SET pinned = 1 WHERE id = ?1", [ids[0]])?;
            Ok(())
        })
        .unwrap();

        // Keep four in use: two must go, the oldest that are not pinned.
        assert_eq!(cap_active(&db, 4).unwrap(), 2);

        assert_eq!(status_of(&db, ids[0]).as_deref(), Some("active"), "pinned");
        assert_eq!(status_of(&db, ids[1]).as_deref(), Some("archived"));
        assert_eq!(status_of(&db, ids[2]).as_deref(), Some("archived"));
        assert_eq!(status_of(&db, ids[3]).as_deref(), Some("active"));
        assert_eq!(
            cap_active(&db, 4).unwrap(),
            0,
            "a second pass changes nothing"
        );
        assert_eq!(
            cap_active(&db, 100).unwrap(),
            0,
            "under the cap nothing goes"
        );
    }

    #[test]
    fn the_stored_text_of_old_turns_is_cleared_once() {
        let db = Database::open_memory().unwrap();
        let id = store(&db, "a note");
        db.with_conn(|c| {
            c.execute(
                "UPDATE memories SET raw_text = 'user: a whole old exchange' WHERE id = ?1",
                [id],
            )?;
            Ok(())
        })
        .unwrap();

        assert_eq!(run(&db).scrubbed, 1);
        assert_eq!(run(&db).scrubbed, 0);
        let kept: String = db
            .with_conn(|c| {
                Ok(
                    c.query_row("SELECT summary FROM memories WHERE id = ?1", [id], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(kept, "a note", "only the turn text goes, not the note");
    }

    #[test]
    fn a_legacy_numeric_timestamp_does_not_fail_the_pass() {
        let db = Database::open_memory().unwrap();
        let id = store(&db, "legacy");
        db.with_conn(|c| {
            c.execute(
                "UPDATE memories SET created_at = 1700000000 WHERE id = ?1",
                [id],
            )?;
            Ok(())
        })
        .unwrap();

        assert!(
            MemoryForgetter::new(db, temp_vault().path().to_path_buf())
                .run_cycle()
                .is_ok()
        );
    }
}
