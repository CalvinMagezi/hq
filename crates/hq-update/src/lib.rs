//! Pull-based, signature-verified self-update for HQ instances.
//!
//! `engine` holds the flow and talks to the outside world only through the
//! traits in `ports`, so it is fully testable with fakes; `real` holds the
//! production implementations and `cli` wires them together for `hq update`.
//! The release format is documented in docs/UPDATE_SYSTEM.md.

// The updater swaps binaries in place with Unix file modes, flock and ownership checks, so it
// is built on Unix only. HQ Lite on Windows is updated by running the installer again.
#[cfg(unix)]
pub mod archive;
#[cfg(unix)]
pub mod cli;
#[cfg(unix)]
pub mod config;
#[cfg(unix)]
pub mod dbops;
#[cfg(unix)]
pub mod engine;
#[cfg(unix)]
pub mod error;
#[cfg(unix)]
pub mod lock;
#[cfg(unix)]
pub mod manifest;
#[cfg(unix)]
pub mod ports;
#[cfg(unix)]
pub mod real;
#[cfg(unix)]
pub mod state;
#[cfg(unix)]
pub mod swap;
#[cfg(unix)]
pub mod verify;

#[cfg(all(unix, test))]
mod engine_tests;
#[cfg(all(unix, test))]
mod testkit;

#[cfg(unix)]
pub use error::{Result, UpdateError};
