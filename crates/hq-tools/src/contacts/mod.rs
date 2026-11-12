//! Looking up people in the owner's phone contacts: the checks HQ runs first, the tool core and
//! the filter on the phone's answer. The pending-action store, the link to the phone and the risk
//! table are `device_actions`; this module supplies the one spec. Contact text is written by
//! other people, so the tool is a `Read` and taints the thread.

pub mod outcome;
pub mod search;

pub use search::SEARCH_TOOL_NAME;

use crate::device_actions::DeviceToolSpec;

pub const SEARCH_ACTION_NAME: &str = "contacts.search";

/// The contacts tools, in the order a phone chat is offered them.
pub const SPECS: &[DeviceToolSpec] = &[search::SPEC];

#[cfg(test)]
mod tests;
