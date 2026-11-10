//! How a tool learns who is calling it.
//!
//! A model chooses the arguments of a tool call, so nothing it sends can say who the caller is.
//! The runtime says it instead, in reserved arguments only it may set. The MCP gateway removes
//! each marker it owns from what a caller sent and sets it again from what it verified (the key
//! the call arrived on, a connector grant, a session's token), and drops everything under
//! [`NATIVE_ARG_PREFIX`]. A native agent session drops every key under [`RUNTIME_ARG_PREFIX`] the
//! model sent and then adds the `_hq_native_*` markers for itself. A tool that reads one of these
//! is reading the runtime, never the caller.
//!
//! The names and their readers live in the base because everything that writes on someone's
//! behalf depends on them: tasks, vault review, provenance, `hq_ask`, the gateway and the
//! coding-agent sessions. A module may read them. None may define its own way to name a caller.

use anyhow::{Result, bail};
use serde_json::Value;

pub use crate::approval_key::RUNTIME_ARG_PREFIX;
use crate::text::clean_label;

/// Set by the MCP gateway (never by a caller) on calls that arrive on the
/// handoff-scoped key, so `agent_host.handoff_cwd_allow` can bind them.
pub const HANDOFF_SCOPE_ARG: &str = "_hq_handoff_scope";
/// Set by the gateway, never by a caller, on a call that arrived on the tasks scope.
pub const TASKS_SCOPE_ARG: &str = "_hq_tasks_scope";
/// Who a tasks-scope write is attributed to, whatever name the caller supplies.
pub const TASKS_SCOPE_ACTOR: &str = "mcp:tasks";
/// Set by the gateway, never by a caller, on a call from a claude.ai connector grant. Its
/// value is the grant's label.
pub const CONNECTOR_ARG: &str = "_hq_connector";
/// Prefix of the name a connector grant's writes are attributed to.
pub const CONNECTOR_ACTOR: &str = "mcp:connector";

/// Set by the gateway when the caller says it runs inside a session HQ spawned
/// (header `x-hq-session-id`, filled from the `HQ_SESSION_ID` the pane was
/// launched with). A caller that sets it itself only restricts itself.
pub const SPAWNED_SESSION_ARG: &str = "_hq_spawned_session";

/// The environment variable HQ sets in every pane it launches, naming the session.
pub const SESSION_ENV: &str = "HQ_SESSION_ID";

/// Argument the gateway sets, and only the gateway, to the session a call came
/// from once that session proved itself with its token. Tools that act on behalf
/// of an agent (messaging, claiming) read the caller from here, never from a
/// field the caller supplies.
pub const CALLER_SESSION_ARG: &str = "_hq_caller_session";

pub const SPAWNED_REFUSAL: &str = "HQ-spawned sessions cannot resume, hand off or type into other sessions, or run full-mode asks; start a teammate with agent_delegate (or harness_session_spawn with from_session) and talk to it with agent_message_send";

/// Set by a native agent session, never by the model: the session the call belongs to.
pub const NATIVE_SESSION_ARG: &str = "_hq_native_session";
/// Set by a native agent session: the agent's name.
pub const NATIVE_AGENT_ARG: &str = "_hq_native_agent";
/// Set by a native agent session: ids of the task leases it was handed, newest last. Whether any
/// is still live, and on which task, is read from the database when a write is recorded.
pub const NATIVE_LEASES_ARG: &str = "_hq_native_leases";
/// Everything a native session sets starts with this; the gateway drops all of it from callers.
pub const NATIVE_ARG_PREFIX: &str = "_hq_native_";

/// The connector grant's label, when the call came in on one.
pub fn connector_label(args: &Value) -> Option<&str> {
    args.get(CONNECTOR_ARG).and_then(Value::as_str)
}

/// The name a connector grant's writes are attributed to: `mcp:connector/<label>`.
pub fn connector_actor(label: &str) -> String {
    format!("{CONNECTOR_ACTOR}/{}", clean_label(label))
}

/// Whether a scoped call may treat `name` as its own: the tasks scope's names, or this
/// connector grant's name. A tasks-scope grant writes and leases under the latter.
pub fn is_own_scope_actor(args: &Value, name: &str) -> bool {
    connector_label(args).is_some_and(|label| name == connector_actor(label))
        || is_tasks_scope_actor(name)
}

/// Whether `name` is of the connector's shape, which only a connector grant may write under.
pub fn is_connector_actor(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with(CONNECTOR_ACTOR)
}

/// Whether `name` is one a tasks-scope caller writes under: the bare scope actor, or the
/// `mcp:tasks/<label>` a work lease taken on that scope is named.
pub fn is_tasks_scope_actor(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == TASKS_SCOPE_ACTOR
        || name
            .strip_prefix(TASKS_SCOPE_ACTOR)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether a tool call arrived on the tasks scope. That scope may file and edit tasks but
/// must not reach anything else, so task tools that would otherwise write to an agent's
/// mailbox, attribute a write to a name the caller picked, or set routing tags skip it.
pub fn is_tasks_scope(args: &Value) -> bool {
    args.get(TASKS_SCOPE_ARG).and_then(Value::as_bool) == Some(true)
}

/// Whether a task write must mail no one: not an agent mailbox behind a tag or assignee, and
/// not the owner's chat. The tasks scope and every claude.ai connector grant write this way, so
/// text from a Claude conversation (which may have read a hostile page) reaches an agent only
/// when the agent looks at the task, never as a message pushed at it. The owner needs no
/// approval for these writes because of it.
pub fn writes_no_mail(args: &Value) -> bool {
    is_tasks_scope(args) || connector_label(args).is_some()
}

/// Whether a tool call arrived on the handoff-scoped key.
pub fn is_handoff_scope(args: &Value) -> bool {
    args.get(HANDOFF_SCOPE_ARG).and_then(Value::as_bool) == Some(true)
}

/// The session the gateway attested for this call, if any.
pub fn caller_session(args: &Value) -> Option<&str> {
    args.get(CALLER_SESSION_ARG)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// The id of the HQ-spawned session this call came from, when the gateway marked it.
pub fn spawned_session(args: &Value) -> Option<&str> {
    args.get(SPAWNED_SESSION_ARG)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

pub fn refuse_if_spawned(args: &Value) -> Result<()> {
    match spawned_session(args) {
        Some(_) => bail!(SPAWNED_REFUSAL),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Every argument the runtime sets, by who sets it.
    const GATEWAY_ARGS: &[&str] = &[
        HANDOFF_SCOPE_ARG,
        TASKS_SCOPE_ARG,
        CONNECTOR_ARG,
        SPAWNED_SESSION_ARG,
        CALLER_SESSION_ARG,
    ];
    const NATIVE_ARGS: &[&str] = &[NATIVE_SESSION_ARG, NATIVE_AGENT_ARG, NATIVE_LEASES_ARG];

    /// A native session clears a model's forgeries with one prefix test, and approvals are
    /// matched on arguments with that prefix left out. Both hold only while every marker is
    /// under the prefix, so a marker added outside it fails here and not in production.
    #[test]
    fn every_marker_is_under_the_runtime_prefix() {
        for arg in GATEWAY_ARGS.iter().chain(NATIVE_ARGS) {
            assert!(arg.starts_with(RUNTIME_ARG_PREFIX), "{arg}");
        }
    }

    /// The gateway drops a caller's native markers by prefix, and nothing it sets itself may be
    /// caught by that.
    #[test]
    fn native_markers_share_a_prefix_no_gateway_marker_has() {
        for arg in NATIVE_ARGS {
            assert!(arg.starts_with(NATIVE_ARG_PREFIX), "{arg}");
        }
        for arg in GATEWAY_ARGS {
            assert!(!arg.starts_with(NATIVE_ARG_PREFIX), "{arg}");
        }
    }

    #[test]
    fn a_scope_marker_counts_only_when_it_is_true() {
        assert!(is_tasks_scope(&json!({ TASKS_SCOPE_ARG: true })));
        assert!(!is_tasks_scope(&json!({ TASKS_SCOPE_ARG: false })));
        assert!(!is_tasks_scope(&json!({ TASKS_SCOPE_ARG: "true" })));
        assert!(!is_tasks_scope(&json!({})));
        assert!(is_handoff_scope(&json!({ HANDOFF_SCOPE_ARG: true })));
        assert!(!is_handoff_scope(&json!({ HANDOFF_SCOPE_ARG: 1 })));
    }

    #[test]
    fn an_empty_session_marker_names_no_one() {
        assert_eq!(caller_session(&json!({ CALLER_SESSION_ARG: "" })), None);
        assert_eq!(
            caller_session(&json!({ CALLER_SESSION_ARG: "hs-1" })),
            Some("hs-1")
        );
        assert_eq!(spawned_session(&json!({ SPAWNED_SESSION_ARG: "" })), None);
        assert!(refuse_if_spawned(&json!({})).is_ok());
        let refused = refuse_if_spawned(&json!({ SPAWNED_SESSION_ARG: "hs-2" })).unwrap_err();
        assert_eq!(refused.to_string(), SPAWNED_REFUSAL);
    }

    #[test]
    fn a_connector_writes_under_its_cleaned_label_and_no_other_name() {
        let args = json!({ CONNECTOR_ARG: "Claude\u{200B} on the web" });
        assert_eq!(connector_label(&args), Some("Claude\u{200B} on the web"));
        let actor = connector_actor("Claude\u{200B} on the web");
        assert_eq!(actor, "mcp:connector/Claude on the web");
        assert!(is_connector_actor(&actor));
        assert!(is_connector_actor("MCP:Connector/anything"));
        assert!(is_own_scope_actor(&args, &actor));
        assert!(!is_own_scope_actor(&args, "mcp:connector/someone else"));
        assert!(is_own_scope_actor(&args, "mcp:tasks"));
        assert!(writes_no_mail(&args));
        assert!(!writes_no_mail(&json!({})));
    }

    #[test]
    fn the_tasks_scope_owns_its_bare_name_and_its_leases() {
        assert!(is_tasks_scope_actor("mcp:tasks"));
        assert!(is_tasks_scope_actor("MCP:tasks/laptop"));
        assert!(!is_tasks_scope_actor("mcp:tasksmith"));
        assert!(!is_tasks_scope_actor("relay"));
    }
}
