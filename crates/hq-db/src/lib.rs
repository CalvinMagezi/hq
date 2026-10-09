//! SQLite database layer — single consolidated database for all HQ data.

pub mod ask_requests;
pub mod background_turns;
pub mod chat;
pub mod copilot_usage_samples;
pub mod harness_drive_gate;
pub mod harness_sessions_registry;
pub mod migrations;
pub mod pool;
pub mod search;
pub mod self_update_runs;
pub mod session_tokens;
pub mod skill_invocations;
pub mod subagent_runs;
pub mod task_graph;
pub mod task_outcomes;
pub mod tasks;
pub mod tool_usage;
pub mod usage_ledger;
pub mod value_items;
pub mod vault_cache;

pub use pool::Database;

// Re-export the underlying rusqlite Connection type so downstream crates
// (e.g. hq-agent's card refresher) can write DB helpers without taking
// their own rusqlite dependency.
pub use rusqlite::Connection;
