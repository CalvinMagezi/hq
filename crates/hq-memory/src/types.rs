//! Memory system types — ported from vault-memory/src/db.ts

use serde::{Deserialize, Serialize};

/// A single memory record stored in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    /// Origin: 'discord', 'job-abc', 'delegation-xyz', 'vault-note', 'daemon'
    pub source: String,
    /// Harness: 'hq', 'relay', 'agent' (formerly supported external harnesses now retired)
    pub harness: String,
    /// Original text (truncated to 4000 chars at ingestion)
    pub raw_text: String,
    /// LLM-extracted 1-2 sentence summary
    pub summary: String,
    /// Key people, projects, tools
    pub entities: Vec<String>,
    /// 2-4 topic tags
    pub topics: Vec<String>,
    /// 0.0 - 1.0 importance score
    pub importance: f64,
    /// Whether this memory has been consolidated
    pub consolidated: bool,
    /// Number of times this memory was replayed (reverse/forward)
    pub replay_count: i64,
    /// ISO 8601 timestamp
    pub created_at: String,
    /// Last time this memory was served to an agent
    pub last_accessed_at: Option<String>,
    /// Number of times this memory was accessed for context
    pub access_count: i64,
    /// Cached differential summary for pattern separation
    pub delta_summary: Option<String>,
}

/// An edge between entities (co-occurrence or direct relationship).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityEdge {
    pub source_id: i64,
    pub target_id: i64,
    pub relationship: String, // co_occurs, relates_to
    pub weight: f64,
    pub source_memory_id: Option<i64>, // optionally link to memory that created this edge
    pub updated_at: String,
}

/// Result of a spreading activation query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivatedEntity {
    pub id: i64,
    pub canonical: String,
    pub display_name: String,
    pub entity_type: String,
    pub activation: f64,
}

/// A consolidation record — the insight from clustering related memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Consolidation {
    pub id: i64,
    /// IDs of the memories that produced this insight
    pub source_ids: Vec<i64>,
    /// The synthesized insight text
    pub insight: String,
    /// Discovered connections between memories
    pub connections: Vec<Connection>,
    /// Parent consolidation ID for insight chaining (evolving understanding)
    pub parent_id: Option<i64>,
    /// ISO 8601 timestamp
    pub created_at: String,
}

/// A connection between two memories found during consolidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub from_id: i64,
    pub to_id: i64,
    pub relationship: String,
}


/// Aggregate memory stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total: i64,
    pub unconsolidated: i64,
    pub consolidations: i64,
}

/// LLM-extracted memory from raw text (ingester output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedMemory {
    pub summary: String,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
    pub importance: f64,
}

/// Result of an LLM consolidation call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationResult {
    pub connections: Vec<Connection>,
    pub insight: String,
}
