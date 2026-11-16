//! Durable storage for the HQ Suite.
//!
//! Four small pieces that the shrink pipeline stands on:
//!
//! - [`fsx`]: files written whole or not at all, moved without clobbering, hashed in a stream.
//! - [`cas`]: pictures and other media kept once under their SHA-256, where notes link to them
//!   with ordinary relative links.
//! - [`trash`]: originals set aside in the vault's own `_trash/<day>/` layout, so its 30-day
//!   retention applies and a person can find them with a file manager.
//! - [`journal`]: one file per shrink job saying how far it got, so a crash at any moment
//!   leaves the original or the finished record, never neither.
//!
//! Nothing here finds the vault, reads the clock or touches the network. Callers hand in a
//! [`SuitePaths`] and, where a date is needed, the day as text.

pub mod cas;
mod error;
pub mod fsx;
pub mod journal;
mod paths;
pub mod trash;

pub use cas::{Cas, CasRef, Stored};
pub use error::StoreError;
pub use journal::{Disposition, Job, Journal, Step};
pub use paths::SuitePaths;
