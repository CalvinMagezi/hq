//! Creating, reading, changing and deleting events on the owner's phone: the checks HQ runs first, the pending-action store,
//! and the tool core. The web server carries the wire (`device_action` frames and the routes
//! under `/api/device-actions`); the agent crate wraps `DeviceCalendar` as a tool.

pub mod check;
pub mod create;
pub mod delete;
pub mod list;
pub mod outcome;
pub mod store;
pub mod target;
pub mod update;

pub use create::{
    DeviceCalendar, PhoneClock, TOOL_NAME, finalize, finalize_for, result_is_acceptable,
    result_is_acceptable_for,
};
pub use delete::DELETE_TOOL_NAME;
pub use list::LIST_TOOL_NAME;
pub use store::{DeviceActionStore, Reject};
pub use target::describe_modification;
pub use update::UPDATE_TOOL_NAME;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_modify;
