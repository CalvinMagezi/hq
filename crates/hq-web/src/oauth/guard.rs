//! What stands between a connector grant and HQ's tools (`docs/plans/claude-connector.md`,
//! "Limiting the blast radius"): a per-grant rate limit, the owner's signed approval for
//! high-risk calls, the gateway marker that names the connection, and a log of every call.

use crate::WsState;
use crate::mcp_http::{CallCaller, call_tool, rpc_ok};
use axum::Json;
use hq_core::types::{ValueItem, ValueKind, ValueState};
use hq_db::mcp_connector::{self as store, GrantAccess};
use hq_db::value_items;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// Calls one grant may make in a minute.
pub(crate) const MAX_CALLS_PER_MINUTE: i64 = 120;
const RATE_WINDOW_SECS: i64 = 60;
/// How long a call that needs approval waits before answering "pending". claude.ai's own
/// tool timeout is not published, so this stays well under a minute.
pub(crate) const APPROVAL_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(40)
};
const APPROVAL_POLL: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(1)
};
/// Arguments longer than this are not shown in a chat message, so such a call is refused
/// rather than approved from a cut view.
pub(crate) const MAX_APPROVAL_ARGS_CHARS: usize = 3000;
const SOURCE: &str = "mcp-connector";
/// Approval requests one grant may have waiting at once, so a hijacked conversation cannot
/// flood the owner's chat.
pub(crate) const MAX_WAITING_APPROVALS: i64 = 5;
/// How long an approval request, and a decline, lasts.
const APPROVAL_TTL_HOURS: i64 = 24;
/// Gateway markers start with this; a caller's copy never counts toward the arguments' hash.
const MARKER_PREFIX: &str = "_hq_";

/// One `tools/call` from a connector grant.
pub(crate) async fn connector_call(
    state: &WsState,
    id: Value,
    body: &Value,
    allowed: Option<&[&str]>,
    grant: &GrantAccess,
) -> Json<Value> {
    let call = Call::of(body);
    let now = super::now();
    let busy = state
        .db
        .with_conn(|c| store::calls_since(c, &grant.grant_id, now - RATE_WINDOW_SECS));
    // An unreadable count fails closed, like the limit itself.
    if busy.is_err() || busy.is_ok_and(|n| n >= MAX_CALLS_PER_MINUTE) {
        log(state, grant, &call, "rate_limited");
        return tool_text(
            id,
            true,
            "Too many calls from this connection in the last minute; wait and try again.",
        );
    }
    if let Some(stop) = confirmation(state, grant, &call, allowed).await {
        log(state, grant, &call, stop.outcome);
        return tool_text(id, stop.is_error, &stop.text);
    }
    let caller = CallCaller {
        connector: Some(&grant.label),
        ..CallCaller::default()
    };
    let reply = call_tool(state, id, body, allowed, caller).await;
    let failed = reply.0.get("error").is_some() || reply.0["result"]["isError"] == true;
    log(state, grant, &call, if failed { "error" } else { "ok" });
    reply
}

/// The tool a `tools/call` runs and the arguments it runs with: for `hq_call`, the inner tool.
struct Call {
    tool: String,
    args: Value,
    digest: String,
    /// Only `hq_call` runs a registry tool; any other name is a gateway tool or refused.
    via_hq_call: bool,
}

impl Call {
    fn of(body: &Value) -> Self {
        let params = &body["params"];
        let outer = params["name"].as_str().unwrap_or_default();
        let via_hq_call = outer == "hq_call";
        let (tool, args) = if via_hq_call {
            let inner = &params["arguments"];
            let args = match &inner["args"] {
                Value::Null => json!({}),
                a => a.clone(),
            };
            (inner["tool"].as_str().unwrap_or_default().to_string(), args)
        } else {
            (outer.to_string(), params["arguments"].clone())
        };
        let args = without_markers(&args);
        let digest = sha256_hex(&canonical(&args));
        Self {
            tool,
            args,
            digest,
            via_hq_call,
        }
    }
}

fn without_markers(args: &Value) -> Value {
    match args {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !k.starts_with(MARKER_PREFIX))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// JSON with every object's keys sorted, so the same arguments always hash the same.
fn canonical(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", json!(k), canonical(&map[k])))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Why a call did not run.
struct Stop {
    outcome: &'static str,
    is_error: bool,
    text: String,
}

/// `None` when the call may run now; otherwise why it may not. An approval is bound to this
/// grant, this tool and these exact arguments, and is spent by the one call it lets through.
async fn confirmation(
    state: &WsState,
    grant: &GrantAccess,
    call: &Call,
    allowed: Option<&[&str]>,
) -> Option<Stop> {
    let registry = state.registry.as_ref()?;
    // A tool outside the grant's scope is refused by the gateway; the owner is never asked.
    // The tasks scope drops tags and assignees itself, so its task writes route nowhere.
    let tasks_scope = allowed == Some(hq_mcp::gateway::TASKS_ALLOWLIST)
        && hq_mcp::gateway::TASK_ROUTING_TOOLS.contains(&call.tool.as_str());
    let gated = !tasks_scope
        && call.via_hq_call
        && allowed.is_none_or(|a| a.contains(&call.tool.as_str()))
        && hq_mcp::gateway::needs_owner_confirmation(registry, &call.tool, &call.args);
    if !gated {
        return None;
    }
    let prefix = format!("connector:{}:", grant.grant_id);
    let key = format!("{prefix}{}:{}", call.tool, call.digest);
    match gate(&state.db, &key) {
        None => return Some(lookup_failed()),
        Some(Gate::Run) => return None,
        Some(Gate::Declined) => return Some(declined()),
        Some(Gate::Ask) => {
            if let Err(stop) = ask_owner(state, grant, call, &key, &prefix) {
                return Some(stop);
            }
        }
        Some(Gate::Wait) => {}
    }
    wait_for_owner(&state.db, &key, &call.tool).await
}

/// Where the approval for one exact call stands, spending it when it is there to spend.
/// `None` when the store cannot be read.
fn gate(db: &hq_db::Database, key: &str) -> Option<Gate> {
    let latest = value_items::latest_engagement_by_dedup(db, key).ok()?;
    if value_items::consume_approval(db, key).ok()? {
        return Some(Gate::Run);
    }
    Some(Gate::from_latest(latest, chrono::Utc::now()))
}

async fn wait_for_owner(db: &hq_db::Database, key: &str, tool: &str) -> Option<Stop> {
    let deadline = tokio::time::Instant::now() + APPROVAL_WAIT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(APPROVAL_POLL).await;
        match gate(db, key) {
            Some(Gate::Run) => return None,
            Some(Gate::Declined) => return Some(declined()),
            None => return Some(lookup_failed()),
            _ => {}
        }
    }
    Some(Stop {
        outcome: "pending",
        is_error: false,
        text: format!(
            "Waiting for the owner to approve this {tool} call in HQ (sent to their chat and the web inbox). \
             Once they say it is approved, call hq_call again with exactly the same tool and arguments; \
             any change needs a new approval."
        ),
    })
}

/// Where an approval for one exact call stands.
#[derive(Debug, PartialEq)]
enum Gate {
    Run,
    /// No live request: send one.
    Ask,
    Wait,
    Declined,
}

impl Gate {
    /// An expired request, answered or not, is asked again: the owner may have missed it, and a
    /// decline lasts as long as the request does.
    fn from_latest(
        latest: Option<value_items::LatestEngagement>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let Some(latest) = latest else {
            return Self::Ask;
        };
        if latest.expires_at.is_some_and(|at| at <= now) {
            return Self::Ask;
        }
        let spent = latest.engagement.as_deref() == Some(value_items::ENGAGEMENT_USED);
        match latest.state {
            ValueState::Engaged if spent => Self::Ask,
            ValueState::Expired => Self::Ask,
            ValueState::Dismissed => Self::Declined,
            _ => Self::Wait,
        }
    }
}

fn lookup_failed() -> Stop {
    Stop {
        outcome: "error",
        is_error: true,
        text: "HQ could not check the owner's approvals just now; nothing ran. Try again shortly."
            .into(),
    }
}

fn declined() -> Stop {
    Stop {
        outcome: "declined",
        is_error: true,
        text: "The owner declined this exact call. Do not retry it with the same arguments; \
               ask them what they want instead."
            .into(),
    }
}

/// Files the approval request and sends it to the owner's chat at once, past the batching.
fn ask_owner(
    state: &WsState,
    grant: &GrantAccess,
    call: &Call,
    key: &str,
    prefix: &str,
) -> Result<(), Stop> {
    let waiting =
        value_items::count_waiting_with_prefix(&state.db, prefix).map_err(|_| lookup_failed())?;
    if waiting >= MAX_WAITING_APPROVALS {
        return Err(Stop {
            outcome: "refused",
            is_error: true,
            text: format!(
                "{MAX_WAITING_APPROVALS} approval requests from this connection are already waiting \
                 for the owner. Wait for them to answer before asking for more."
            ),
        });
    }
    let shown = serde_json::to_string_pretty(&call.args).unwrap_or_default();
    if shown.chars().count() > MAX_APPROVAL_ARGS_CHARS {
        return Err(Stop {
            outcome: "refused",
            is_error: true,
            text: format!(
                "This {} call is too large for the owner to review in a message, so it cannot be \
                 approved from here. Ask the owner to do it themselves, or send a smaller call.",
                call.tool
            ),
        });
    }
    let mut item = ValueItem::new(
        SOURCE,
        ValueKind::ActionNeeded,
        format!("Claude wants to run {}", call.tool),
        format!(
            "The claude.ai connection \"{}\" asks to run {} with exactly these arguments:\n\n{shown}\n\n\
             Approve to let it run once. Dismiss to refuse. If you did not ask Claude for this, \
             dismiss it and disconnect the connection in Settings.",
            grant.label, call.tool
        ),
    )
    .with_dedup_key(key.to_string());
    item.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(APPROVAL_TTL_HOURS));
    let sent = value_items::insert(&state.db, &item)
        .and_then(|()| value_items::deliver(&state.vault_path, &state.db, &item, true));
    if !matches!(sent, Ok(true)) {
        tracing::warn!(tool = %call.tool, "connector: the approval request did not reach the relay; it is still in the web inbox");
    }
    Ok(())
}

/// Longest tool name the log keeps: real names are far shorter, and a caller picks this one.
const MAX_LOGGED_TOOL_CHARS: usize = 64;

fn log(state: &WsState, grant: &GrantAccess, call: &Call, outcome: &str) {
    let tool: String = if call.tool.is_empty() {
        "unknown".into()
    } else {
        call.tool.chars().take(MAX_LOGGED_TOOL_CHARS).collect()
    };
    let logged = state.db.with_conn(|c| {
        store::record_call(
            c,
            &grant.grant_id,
            &tool,
            outcome,
            &call.digest,
            super::now(),
        )
    });
    if let Err(e) = logged {
        tracing::warn!("connector: logging a call failed: {e}");
    }
}

fn tool_text(id: Value, is_error: bool, text: &str) -> Json<Value> {
    rpc_ok(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
    )
}

/// Tells the owner, past the relay's batching, about a connection event they should see.
pub(crate) fn notify_owner(state: &WsState, title: &str, body: &str) {
    let item = ValueItem::new(SOURCE, ValueKind::Fyi, title, body);
    let sent = value_items::insert(&state.db, &item)
        .and_then(|()| value_items::deliver(&state.vault_path, &state.db, &item, true));
    if !matches!(sent, Ok(true)) {
        tracing::warn!("connector: a notice did not reach the relay mailbox: {title}");
    }
}

#[cfg(test)]
mod tests;
