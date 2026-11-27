//! MemoryQuerier — retrieves recent memories for context injection.
//!
//! Pure SQLite, no LLM. Memories that share 3+ topic tags with one already
//! picked are skipped so the injected set stays diverse.

use anyhow::Result;
use hq_db::Database;
use serde::{Deserialize, Serialize};

use crate::db::{get_memory_stats, get_recent_memories, touch_memory};
use crate::types::{Memory, MemoryStats};

/// Formatted memory context ready for system prompt injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryContext {
    /// Formatted string ready for injection into a system prompt
    pub formatted: String,
    /// Raw memories retrieved
    pub memories: Vec<Memory>,
    /// Stats for debugging
    pub stats: MemoryStats,
}

/// Topic tags two memories must share to count as overlapping.
const OVERLAP_TOPICS: usize = 3;

pub struct MemoryQuerier {
    db: Database,
}

impl MemoryQuerier {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Get recent high-importance memories formatted for context injection.
    /// Returns an empty context if no memories exist yet.
    pub fn get_recent_context(
        &mut self,
        limit: Option<i64>,
        topic_filter: Option<&[String]>,
    ) -> Result<MemoryContext> {
        let limit = limit.unwrap_or(8);

        let mut memories = get_recent_memories(&self.db, limit * 2)?;

        if let Some(filter) = topic_filter
            && !filter.is_empty()
        {
            let filter_set: std::collections::HashSet<String> =
                filter.iter().map(|t| t.to_lowercase()).collect();
            memories.retain(|m| {
                m.topics
                    .iter()
                    .any(|t| filter_set.contains(&t.to_lowercase()))
            });
        }

        let memories = deduplicate_by_topics(memories, limit as usize);

        // Touch accessed memories so the forgetter knows they're still relevant
        for m in &memories {
            let _ = touch_memory(&self.db, m.id);
        }

        let stats = get_memory_stats(&self.db)?;

        let formatted = format_for_context(&memories);

        Ok(MemoryContext {
            memories,
            stats,
            formatted,
        })
    }
}

/// Keep up to `limit` memories, skipping any that shares `OVERLAP_TOPICS`
/// tags with one already kept. High-salience memories always survive.
fn deduplicate_by_topics(memories: Vec<Memory>, limit: usize) -> Vec<Memory> {
    let mut kept: Vec<Memory> = Vec::new();
    for candidate in memories {
        if kept.len() >= limit {
            break;
        }
        let high_salience = candidate.topics.iter().any(|t| t == "high-salience");
        let overlaps = kept.iter().any(|k| {
            candidate
                .topics
                .iter()
                .filter(|t| k.topics.contains(t))
                .count()
                >= OVERLAP_TOPICS
        });
        if high_salience || !overlaps {
            kept.push(candidate);
        }
    }
    kept
}

/// Format memories as a compact block for system prompt injection. Insights
/// are not repeated here: MEMORY.md's "Agent Insights" section carries them.
/// Adds staleness caveats for old memories (>30 days: "[may be outdated]").
pub(crate) fn format_for_context(memories: &[Memory]) -> String {
    if memories.is_empty() {
        return String::new();
    }
    let mut parts = vec!["**Recent Memory:**".to_string()];
    for m in memories {
        let age = relative_age(&m.created_at);
        let caveat = staleness_caveat(&m.created_at);
        if caveat.is_empty() {
            parts.push(format!("- [{}/{age}] {}", m.source, m.summary));
        } else {
            parts.push(format!("- [{}/{age}] {caveat} {}", m.source, m.summary));
        }
    }
    parts.join("\n")
}

/// Returns a staleness caveat string for old content.
/// Empty string for fresh content, "[may be outdated]" for >30 days.
fn staleness_caveat(iso_date: &str) -> &'static str {
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_date) else {
        return "";
    };
    let days = chrono::Utc::now().signed_duration_since(dt).num_days();
    if days > 30 { "[may be outdated]" } else { "" }
}

/// Compute a human-readable relative age string.
pub(crate) fn relative_age(iso_date: &str) -> String {
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(iso_date) else {
        return "?".into();
    };
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(dt);
    let mins = duration.num_minutes();

    if mins < 60 {
        format!("{mins}m ago")
    } else {
        let hrs = mins / 60;
        if hrs < 24 {
            format!("{hrs}h ago")
        } else {
            format!("{}d ago", hrs / 24)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{StoreMemoryParams, store_memory};

    fn store(db: &Database, summary: &str, importance: f64, topics: &[&str]) -> i64 {
        let topics: Vec<String> = topics.iter().map(ToString::to_string).collect();
        store_memory(
            db,
            &StoreMemoryParams {
                source: "chat-turn",
                harness: "web",
                raw_text: "",
                summary,
                entities: &[],
                topics: &topics,
                importance,
                user_id: None,
            },
        )
        .unwrap()
    }

    fn ids(ctx: &MemoryContext) -> Vec<i64> {
        ctx.memories.iter().map(|m| m.id).collect()
    }

    #[test]
    fn an_empty_store_gives_an_empty_block() {
        let db = Database::open_memory().unwrap();
        let ctx = MemoryQuerier::new(db)
            .get_recent_context(None, None)
            .unwrap();
        assert!(ctx.memories.is_empty());
        assert!(ctx.formatted.is_empty());
        assert_eq!(ctx.stats.total, 0);
    }

    #[test]
    fn the_default_is_eight_memories() {
        let db = Database::open_memory().unwrap();
        for i in 0..12 {
            store(&db, &format!("m{i}"), 0.5, &[]);
        }
        let ctx = MemoryQuerier::new(db)
            .get_recent_context(None, None)
            .unwrap();
        assert_eq!(ctx.memories.len(), 8);
    }

    #[test]
    fn a_memory_sharing_three_topics_with_a_kept_one_is_skipped() {
        let db = Database::open_memory().unwrap();
        let first = store(&db, "first", 0.9, &["a", "b", "c", "d"]);
        let echo = store(&db, "echo", 0.8, &["a", "b", "c"]);
        let two_shared = store(&db, "two shared", 0.7, &["a", "b", "z"]);

        let ctx = MemoryQuerier::new(db)
            .get_recent_context(Some(8), None)
            .unwrap();

        assert!(ids(&ctx).contains(&first));
        assert!(
            !ids(&ctx).contains(&echo),
            "three shared topics is a repeat"
        );
        assert!(
            ids(&ctx).contains(&two_shared),
            "two shared is still distinct"
        );
    }

    #[test]
    fn a_high_salience_memory_survives_the_overlap_filter() {
        let db = Database::open_memory().unwrap();
        let first = store(&db, "first", 0.9, &["a", "b", "c"]);
        let salient = store(&db, "salient", 0.8, &["a", "b", "c", "high-salience"]);

        let ctx = MemoryQuerier::new(db)
            .get_recent_context(Some(8), None)
            .unwrap();

        assert_eq!(ids(&ctx), vec![first, salient]);
    }

    #[test]
    fn a_topic_filter_keeps_only_matching_memories_ignoring_case() {
        let db = Database::open_memory().unwrap();
        let money = store(&db, "budget", 0.9, &["Finance"]);
        store(&db, "lunch", 0.8, &["food"]);

        let ctx = MemoryQuerier::new(db)
            .get_recent_context(Some(8), Some(&["finance".to_string()]))
            .unwrap();

        assert_eq!(ids(&ctx), vec![money]);
    }

    #[test]
    fn an_old_memory_is_marked_as_possibly_outdated() {
        let db = Database::open_memory().unwrap();
        let old = store(&db, "an old fact", 0.5, &[]);
        let at = (chrono::Utc::now() - chrono::Duration::days(45)).to_rfc3339();
        db.with_conn(|c| {
            c.execute(
                "UPDATE memories SET created_at = ?1 WHERE id = ?2",
                rusqlite::params![at, old],
            )?;
            Ok(())
        })
        .unwrap();
        store(&db, "a new fact", 0.4, &[]);

        let ctx = MemoryQuerier::new(db)
            .get_recent_context(Some(8), None)
            .unwrap();

        let line = |needle: &str| {
            ctx.formatted
                .lines()
                .find(|l| l.contains(needle))
                .unwrap_or_default()
                .to_string()
        };
        assert!(line("an old fact").contains("[may be outdated]"));
        assert!(!line("a new fact").contains("[may be outdated]"));
        assert!(ctx.formatted.starts_with("**Recent Memory:**"));
    }

    /// Pins today's behaviour so the change to count only memories that were actually shown
    /// (the recall rewrite) is a visible edit to this test, not a silent one.
    #[test]
    fn building_the_block_counts_an_access_for_every_memory_in_it() {
        let db = Database::open_memory().unwrap();
        let id = store(&db, "shown", 0.5, &[]);
        let mut querier = MemoryQuerier::new(db.clone());

        querier.get_recent_context(Some(8), None).unwrap();

        let count: i64 = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT access_count FROM memories WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}
