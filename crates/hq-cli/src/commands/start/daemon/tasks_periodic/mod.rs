//! Periodic daemon tasks (seconds — minutes cadence).
//!
//! Each domain lives in its own submodule. This module re-exports every public
//! task function so callers can continue using `tasks_periodic::run_*` paths.

pub mod adoption_digest;
pub mod backup;
pub mod collection_tip;
pub mod copilot_usage;
pub mod disk_watchdog;
pub mod due_reminders;
pub mod edge_worker_dispatch;
pub mod email_ingest;
pub mod embeddings;
pub mod harness_usage;
pub mod health;
pub mod inbox_tidy;
pub mod link_capture;
pub mod scheduled_offers;
pub mod session_events;
pub mod session_supervisor;
pub mod skill_outcomes;
pub mod stack_dispatch;
pub mod subagent_supervisor;
pub mod task_stale_digest;
pub mod tool_failure_digest;
pub mod turn_reconcile;
pub mod usage_ledger;
pub mod verify_audit;
pub mod verify_briefing;
pub mod verify_dispatch;

pub use adoption_digest::run_adoption_digest;
pub use backup::run_backup;
pub use collection_tip::run_collection_tip;
pub use copilot_usage::run_copilot_usage;
pub use disk_watchdog::run_disk_watchdog;
pub use due_reminders::run_due_reminders;
pub use edge_worker_dispatch::run_edge_worker_dispatch;
pub use email_ingest::run_email_poll;
pub use embeddings::{run_embeddings, run_fts_sync, run_inbox_triage};
pub use harness_usage::run_harness_usage;
pub use health::{run_heartbeat, run_memory_consolidation};
pub use inbox_tidy::run_inbox_tidy;
pub use link_capture::run_link_capture;
pub use scheduled_offers::run_scheduled_offers;
pub use skill_outcomes::run_skill_outcomes;
pub use stack_dispatch::run_stack_dispatch;
pub use subagent_supervisor::run_subagent_supervisor;
pub use task_stale_digest::run_task_stale_digest;
pub use tool_failure_digest::run_tool_failure_digest;
pub use turn_reconcile::run_turn_reconcile;
pub use usage_ledger::run_usage_ledger;
pub use verify_audit::run_verify_audit;
pub use verify_briefing::run_verify_briefing;
pub use verify_dispatch::run_verify_dispatch;
