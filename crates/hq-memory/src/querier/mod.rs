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
fn relative_age(iso_date: &str) -> String {
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
