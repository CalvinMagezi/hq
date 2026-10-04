//! LSP client for Agent HQ — language server protocol integration.
//!
//! Provides semantic code intelligence (go-to-definition, find-references, hover,
//! document symbols, workspace symbols) via language server subprocess management.
//!
//! Supports: rust-analyzer, typescript-language-server, pyright.

mod client;
mod detection;
mod manager;
mod protocol;

pub use manager::LspManager;
