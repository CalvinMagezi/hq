//! hq-memory — Always-on persistent memory for Agent-HQ.
//!
//! Ported from the TypeScript `@repo/vault-memory` package.
//!
//! Components:
//! - **Ingester** — converts raw text into structured memory entries via LLM extraction
//! - **Consolidator** — clusters unconsolidated memories, synthesizes insights via LLM
//! - **Querier** — retrieves relevant memories for context injection (pure SQLite)
//! - **Forgetter** — implements Synaptic Homeostasis (tiered decay + pruning)
//! - **ollama** — local embeddings when no OpenRouter key is set
//!

pub mod concept_pages;
pub mod context_packet;
pub mod consolidator;
pub mod db;
pub mod entity_graph;
pub mod forgetter;
pub mod graph_index;
pub mod graph_paths;
pub mod ingester;
mod json_recovery;
pub mod llm_bridge;
pub mod ollama;
pub mod openrouter_embed;
pub mod querier;
pub mod turn_gate;
pub mod types;

pub use consolidator::MemoryConsolidator;
pub use db::{
    StoreMemoryParams,
    decay_old_memories,
    get_consolidation_history,
    get_memory_stats,
    get_recent_memories,
    get_unconsolidated_memories,
    store_consolidation,
    store_consolidation_chained,
    store_memory,
    touch_memory,
};
pub use forgetter::{ForgetterResult, MemoryForgetter};
pub use ingester::MemoryIngester;
pub use llm_bridge::MemoryLlm;
pub use ollama::generate_embedding;
pub use openrouter_embed::{
    generate_embedding as generate_embedding_openrouter, openrouter_embedding_model,
};
pub use querier::{MemoryContext, MemoryQuerier};
pub use types::*;

