//! The web tools of `hq-tools` and the image generator that uses them: `web` (search, fetch and
//! the engine pool), `remote_mcp` (the bridge to remote MCP servers) and `imagegen`.
//!
//! Split out so a change in the rest of the tools does not recompile ~11,700 lines of networking
//! code, and the other way round. `hq-tools` re-exports all three modules under the same names, so
//! `hq_tools::web::...` keeps working everywhere.

// The moved files name these as `crate::registry::...`; importing them here keeps every path in
// them valid without edits.
use hq_tool_api::{family_confirm, family_guest, registry};

pub mod imagegen;
pub mod remote_mcp;
pub mod web;
