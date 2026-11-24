//! Importing the owner's mobile-money messages from their phone into the private finance ledger.
//! The pending-action store, the link to the phone and the risk table are `device_actions`; this
//! module supplies the one spec.
//!
//! The messages never come back through the tool. The phone reads them, hands them straight to the
//! owner-only route `POST /api/finance/import/{action_id}` (`hq-web`), which writes the ledger and
//! settles the action with counts, and the model is only ever told those counts. The tool is a
//! `Modify` so the owner is asked every time, and the result carries no text from any message.

pub mod import;
pub mod outcome;

pub use import::IMPORT_TOOL_NAME;

use crate::device_actions::DeviceToolSpec;

pub const IMPORT_ACTION_NAME: &str = "finance.import";

/// The finance tools, in the order a phone chat is offered them.
pub const SPECS: &[DeviceToolSpec] = &[import::SPEC];

#[cfg(test)]
mod tests;
