//! The part of `hq-tools` every tool depends on: the `HqTool` trait and `ToolRegistry`
//! (`registry`), shared helpers (`util`) and the family gates (`family_guest`, `family_confirm`).
//!
//! It depends only on `hq-core`. That is what lets other crates, and the tool crates split out of
//! `hq-tools`, depend on the trait without pulling in the 100,000 lines of tools themselves.
//! `hq-tools` re-exports every module, so `hq_tools::registry::HqTool` and `crate::registry::...`
//! inside `hq-tools` keep working.

pub mod family_confirm;
pub mod family_guest;
pub mod office;
pub mod registry;
pub mod util;
