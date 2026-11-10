//! The tool contract lives in `hq-contract`. It is re-exported here so tools in this crate, and
//! callers that reach it through `hq_tools`, keep their paths.

pub use hq_contract::tool::{
    HqTool, ToolPolicy, ToolRegistry, ToolSummary, derive_search_hint, truncate_note,
    validate_and_execute,
};
