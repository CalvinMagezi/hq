//! Actions HQ asks the owner's phone to carry out: the pending-action store, the link a chat
//! turn holds to its phone, and the table of device tools with their risk classes. The web
//! server carries the wire (`device_action` frames and the routes under `/api/device-actions`);
//! the agent crate offers each spec as a tool; a domain such as `calendar` supplies the specs.
//! How to add a domain: `docs/DEVICE_ACTIONS.md`.

#[cfg(any(test, feature = "test-support"))]
pub mod fake;
pub mod link;
pub mod spec;
pub mod store;

pub use link::{DEVICE_UNREACHABLE, DeviceLink, Emit, PhoneClock, code, unreachable};
pub use spec::{
    DeviceRun, DeviceToolSpec, RiskClass, RunFuture, always_ask_reason, finalize_for,
    is_untrusted_source, offered, outcome_is_known, register, result_is_acceptable_for, spec_for,
    spec_for_action, tainted_reason,
};
pub use store::{DeviceActionStore, Reject};

#[cfg(test)]
mod tests;
