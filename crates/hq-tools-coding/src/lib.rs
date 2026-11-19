//! The coding tools of `hq-tools` and the file-edit engine under them: `coding` (read, write,
//! edit, search, git and shell tools for a workspace) and `file_edit`.
//!
//! Split out so a change in the rest of the tools does not recompile them, and the other way
//! round. `hq-tools` re-exports both modules under the same names, so `hq_tools::coding::...`
//! keeps working everywhere.

// The moved files name this as `crate::registry::...`; importing it here keeps every path in
// them valid without edits.
use hq_tool_api::registry;

pub mod coding;
pub mod file_edit;
