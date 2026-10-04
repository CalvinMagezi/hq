//! Pull-based, signature-verified self-update for HQ instances.
//!
//! `engine` holds the flow and talks to the outside world only through the
//! traits in `ports`, so it is fully testable with fakes; `real` holds the
//! production implementations and `cli` wires them together for `hq update`.
//! The release format is documented in docs/UPDATE_SYSTEM.md.

pub mod archive;
pub mod cli;
pub mod config;
pub mod dbops;
pub mod engine;
pub mod error;
pub mod lock;
pub mod manifest;
pub mod ports;
pub mod real;
pub mod state;
pub mod swap;
pub mod verify;

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod testkit;

pub use error::{Result, UpdateError};
