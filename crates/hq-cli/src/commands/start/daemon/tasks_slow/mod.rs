//! Slow-cycle daemon tasks (6 hours — weekly).
//!
//! Each domain lives in its own submodule. This module re-exports every public
//! task function so callers can continue using `tasks_slow::run_*` paths.

pub mod cleanup;
pub mod memory;
pub mod vault_health;

// Flat re-exports so the existing `tasks_slow::run_*` call sites in mod.rs need no changes.
pub use cleanup::{
    run_db_vacuum, run_thread_log_rotation, run_vault_cap_enforcer, run_vault_cleanup,
};
pub use memory::run_memory_forgetting;
pub use vault_health::run_vault_health;
