//! Vault filesystem operations — read/write markdown notes with frontmatter.

pub mod check;
pub mod client;
pub mod frontmatter;
pub mod links;
pub mod note_lock;
pub mod notes;
pub mod rel_path;
pub mod relink;
pub mod remediation;
pub mod reorg;
pub mod system;
pub mod walk;

pub use client::VaultClient;
pub use notes::{NoteHeading, NoteSection};
