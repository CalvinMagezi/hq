//! Periodic daemon tasks (seconds — minutes cadence).
//!
//! Each domain lives in its own submodule. This module re-exports every public
//! task function so callers can continue using `tasks_periodic::run_*` paths.

pub mod copilot_usage;
pub mod disk_watchdog;
pub mod email_ingest;
pub mod embeddings;
pub mod health;
pub mod session_events;
pub mod session_supervisor;
pub mod subagent_supervisor;
pub mod task_stale_digest;
pub mod turn_reconcile;
pub mod harness_usage;
pub mod usage_ledger;

pub use copilot_usage::run_copilot_usage;
pub use disk_watchdog::run_disk_watchdog;
pub use email_ingest::run_email_poll;
pub use embeddings::{run_embeddings, run_inbox_triage};
pub use health::{run_heartbeat, run_memory_consolidation};
pub use subagent_supervisor::run_subagent_supervisor;
pub use task_stale_digest::run_task_stale_digest;
pub use turn_reconcile::run_turn_reconcile;
pub use harness_usage::run_harness_usage;
pub use usage_ledger::run_usage_ledger;
