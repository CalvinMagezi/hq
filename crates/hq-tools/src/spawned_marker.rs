//! How a call says it comes from a session HQ spawned.
//!
//! Free of the harness session code so the host tools can check the marker without reaching up
//! into it. `harness_session` re-exports everything here, so existing paths still work.

use anyhow::{Result, bail};
use serde_json::Value;

/// Set by the gateway when the caller says it runs inside a session HQ spawned
/// (header `x-hq-session-id`, filled from the `HQ_SESSION_ID` the pane was
/// launched with). A caller that sets it itself only restricts itself.
pub const SPAWNED_SESSION_ARG: &str = "_hq_spawned_session";

pub const SPAWNED_REFUSAL: &str = "HQ-spawned sessions cannot resume, hand off or type into other sessions, or run full-mode asks; start a teammate with agent_delegate (or harness_session_spawn with from_session) and talk to it with agent_message_send";

/// The id of the HQ-spawned session this call came from, when the gateway marked it.
pub fn spawned_session(args: &Value) -> Option<&str> {
    args.get(SPAWNED_SESSION_ARG)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// Whether a marked (HQ-spawned) caller may type into `target`: only its own session.
pub fn spawned_may_target(args: &Value, target: &str) -> Result<()> {
    match spawned_session(args) {
        Some(own) if own != target => bail!(SPAWNED_REFUSAL),
        _ => Ok(()),
    }
}
