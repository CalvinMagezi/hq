//! `office_file`'s name and the actions that move an original. The tool itself is in the
//! `hq-tools-office` crate; these three items live in `hq-tool-api` and are re-exported here so
//! `hq_tools::office::OFFICE_TOOL` and the others keep resolving.

pub use hq_tool_api::office::{OFFICE_TOOL, SAFE_ACTIONS, moves_original};
