//! The contract between HQ's base and what is built on it.
//!
//! A tool is written against [`HqTool`] and handed to a [`ToolRegistry`]. The agent loop, the MCP
//! gateway and the crates that hold tools all need those two names, and before this crate
//! existed they could only get them from `hq-tools`, which also holds every tool and links what
//! the tools need. This crate depends on `hq-core` and nothing else in the workspace, so each
//! of them can share the contract without depending on the others.
//!
//! Keep it small. A change here is a change to what every tool and every caller agrees on.

pub mod tool;

pub use tool::{
    HqTool, ToolPolicy, ToolRegistry, ToolSummary, derive_search_hint, truncate_note,
    validate_and_execute,
};
