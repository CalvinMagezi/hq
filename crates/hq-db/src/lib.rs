//! SQLite database layer — single consolidated database for all HQ data.

pub mod adoption;
pub mod ask_requests;
pub mod background_turns;
pub mod bridge;
pub mod bridge_setup;
pub mod chat;
pub mod chat_search;
pub mod client_ids;
pub mod coder_jobs;
pub mod copilot_usage_samples;
pub mod dialog_alerts;
pub mod fetch_sites;
pub mod fleet_digest;
pub mod folder_grants;
pub mod harness_drive_gate;
pub mod harness_sessions_registry;
pub mod host_outages;
pub mod inbox_prefs;
pub mod inbox_tidy;
pub mod link_captures;
pub mod mcp_connector;
pub mod migrations;
pub mod moved;
pub mod note_marks;
pub mod offers;
pub mod owner_approvals;
pub mod pool;
pub mod provenance;
pub mod push_subscriptions;
pub mod search;
pub mod self_update_runs;
pub mod session_checkouts;
pub mod session_events;
pub mod session_policies;
pub mod session_tokens;
pub mod skill_evidence;
pub mod skill_invocations;
pub mod skill_outcomes;
pub mod subagent_runs;
pub mod task_graph;
pub mod task_outcomes;
pub mod tasks;
pub mod thread_folds;
pub mod thread_taint;
pub mod tool_failures;
pub mod tool_usage;
pub mod usage_ledger;
pub mod value_items;
pub mod vault_cache;
pub mod web_auth;
pub mod wire_time;

pub use pool::Database;

// Re-export the underlying rusqlite Connection type so downstream crates
// (e.g. hq-agent's card refresher) can write DB helpers without taking
// their own rusqlite dependency.
pub use rusqlite::Connection;
