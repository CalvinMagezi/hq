//! Creating an event on the owner's phone: the checks HQ runs first, the pending-action store,
//! and the tool core. The web server carries the wire (`device_action` frames and the routes
//! under `/api/device-actions`); the agent crate wraps `DeviceCalendar` as a tool.

pub mod check;
pub mod create;
pub mod outcome;
pub mod store;

pub use create::{DeviceCalendar, PhoneClock, TOOL_NAME, finalize, result_is_acceptable};
pub use store::{DeviceActionStore, Reject};

#[cfg(test)]
mod tests;
