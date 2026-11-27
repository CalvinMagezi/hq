//! SQLite database layer — single consolidated database for all HQ data.

pub mod adoption;
pub mod approval_followup;
pub mod ask_requests;
pub mod background_turns;
pub mod bridge;
pub mod bridge_setup;
pub mod chat;
pub mod chat_search;
pub mod client_ids;
pub mod coder_jobs;
pub mod context_audit;
pub mod context_grants;
pub mod copilot_usage_samples;
pub mod dialog_alerts;
pub mod fetch_sites;
pub mod fleet_digest;
pub mod focus_lanes;
pub mod folder_grants;
pub mod harness_drive_gate;
pub mod harness_sessions_registry;
pub mod helper_status;
pub mod host_outages;
pub mod inbox_prefs;
pub mod inbox_tidy;
pub mod lane_quota_alerts;
pub mod lane_quota_samples;
pub mod link_captures;
pub mod mcp_connector;
pub mod migrations;
pub mod moved;
pub mod note_keep;
pub mod note_marks;
pub mod nudge_ledger;
pub mod offers;
pub mod owner_approvals;
pub mod owner_decisions;
pub mod owner_folder_grants;
pub mod owner_prefs;
pub mod pool;
pub mod provenance;
pub mod push_subscriptions;
pub mod reminders;
pub mod sandbox_grants;
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
pub mod thread_seen_urls;
pub mod thread_taint;
pub mod tool_failures;
pub mod tool_routing;
pub mod tool_usage;
pub mod usage_ledger;
pub mod value_items;
pub mod vault_cache;
pub mod vault_health;
pub mod web_auth;
pub mod wire_time;

pub use pool::Database;

// Re-export the underlying rusqlite Connection type so downstream crates
// (e.g. hq-agent's card refresher) can write DB helpers without taking
// their own rusqlite dependency.
pub use rusqlite::Connection;
