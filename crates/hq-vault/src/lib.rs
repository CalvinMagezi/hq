//! Vault filesystem operations — read/write markdown notes with frontmatter.

pub mod client;
pub mod frontmatter;
pub mod notes;
pub mod remediation;
pub mod reorg;
pub mod system;

pub use client::VaultClient;
pub use notes::{NoteHeading, NoteSection};
