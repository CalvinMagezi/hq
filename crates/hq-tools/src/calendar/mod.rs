//! Creating, reading, changing and deleting events on the owner's phone: the checks HQ runs
//! first and the tool core for each. The pending-action store, the link to the phone and the
//! risk table are `device_actions`; this module supplies the calendar's four specs.

pub mod check;
pub mod create;
pub mod delete;
pub mod list;
pub mod outcome;
pub mod target;
pub mod update;

pub use create::{TOOL_NAME, finalize, result_is_acceptable};
pub use delete::DELETE_TOOL_NAME;
pub use list::LIST_TOOL_NAME;
pub use update::UPDATE_TOOL_NAME;

use crate::device_actions::DeviceToolSpec;

pub const ACTION_NAME: &str = "calendar.create";
pub const LIST_ACTION_NAME: &str = "calendar.list";
pub const UPDATE_ACTION_NAME: &str = "calendar.update";
pub const DELETE_ACTION_NAME: &str = "calendar.delete";

/// The calendar's tools, in the order a phone chat is offered them.
pub const SPECS: &[DeviceToolSpec] = &[create::SPEC, update::SPEC, delete::SPEC, list::SPEC];

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_modify;
