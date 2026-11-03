//! Owner approval for one tool call in `PermissionMode::Default`: the call waits until the
//! owner approves exactly this tool with exactly these arguments in their inbox.

use std::sync::Arc;
use std::time::Duration;

use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use hq_db::value_items::{self, ApprovalGate, RequestState};
use serde_json::Value;

/// Tools that change things the owner cannot simply undo from their notes and tasks: files and
/// commands on the machine, git and GitHub, HQ's config and updates, other machines and agents,
/// and the owner's other accounts. Narrower than the connector's list (`CONNECTOR_CONFIRM_TOOLS`
/// in hq-mcp), which also guards reads from a remote caller; here the caller is the owner's own
/// chat, so reading is never asked about.
const OWNER_CONFIRM_TOOLS: &[&str] = &[
    "bash",
    "write_file",
    "edit_file",
    "rollback_file",
    "file_edit_batch",
    "git_commit",
    "git_pr",
    "github_write",
    "task_delete",
    "self_update_install",
    "self_update_rollback",
    "host_add",
    "host_send",
    "agent_send_message",
    "google_workspace",
    "skill_manage",
    "slash_command_manage",
];

/// `config_manage` changes HQ only when it sets something.
const CONFIG_TOOL: &str = "config_manage";
const CONFIG_READ_ACTION: &str = "get";

/// How long a call waits in the turn for the owner before it returns as pending.
const APPROVAL_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(60)
};
const APPROVAL_POLL: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(1)
};
/// How long a request, and the owner's answer to it, lasts.
const APPROVAL_TTL_HOURS: i64 = 24;
/// Requests one chat may have waiting at once, so a looping model cannot flood the inbox.
const MAX_WAITING_APPROVALS: i64 = 5;
/// Arguments longer than this cannot be reviewed in an inbox message, so they are refused.
const MAX_APPROVAL_ARGS_CHARS: usize = 3000;
const SOURCE: &str = "owner-approval";
const KEY_PREFIX: &str = "owner-approval";

/// Whether a call to `tool` (in `category`) with `args` waits for the owner.
pub fn needs_owner(tool: &str, category: &str, args: &Value) -> bool {
    if tool == CONFIG_TOOL {
        return args.get("action").and_then(Value::as_str) != Some(CONFIG_READ_ACTION);
    }
    OWNER_CONFIRM_TOOLS.contains(&tool) || category == hq_tools::remote_mcp::REMOTE_MCP_CATEGORY
}

/// Where a session asks the owner. Set only on sessions the owner drives from a chat; every
/// other session (sub-agents of unattended work, daemon tasks, watches) has none and never asks.
pub struct OwnerApproval {
    db: Arc<Database>,
    /// Which chat the approvals belong to, set by the runtime and never by the model.
    scope: String,
}

impl OwnerApproval {
    pub fn new(db: Arc<Database>, scope: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            db,
            scope: scope.into(),
        })
    }

    fn prefix(&self) -> String {
        format!("{KEY_PREFIX}:{}:", self.scope)
    }

    fn key(&self, tool: &str, args: &Value) -> String {
        let digest = hq_core::approval_key::args_digest(args);
        format!("{}{tool}:{digest}", self.prefix())
    }

    /// `None` when the call may run now; otherwise the text the model gets instead.
    pub async fn check(&self, tool: &str, args: &Value) -> Option<String> {
        let key = self.key(tool, args);
        match self.gate(tool, &key) {
            Gate::Run => return None,
            Gate::Stop(text) => return Some(text),
            Gate::Ask => {
                if let Err(refused) = self.ask(tool, args, &key) {
                    return Some(refused);
                }
            }
            Gate::Wait => {}
        }
        self.wait(tool, &key).await
    }

    /// Where this exact call's approval stands, spending it when it is there. A store that
    /// cannot be read stops the call.
    fn gate(&self, tool: &str, key: &str) -> Gate {
        match value_items::approval_gate(&self.db, key) {
            Ok(ApprovalGate::Run) => Gate::Run,
            Ok(ApprovalGate::Pending(RequestState::Declined)) => Gate::Stop(declined(tool)),
            Ok(ApprovalGate::Pending(RequestState::Ask)) => Gate::Ask,
            Ok(ApprovalGate::Pending(RequestState::Wait)) => Gate::Wait,
            Err(e) => {
                tracing::warn!(%e, "owner approval: reading the inbox failed");
                Gate::Stop(lookup_failed())
            }
        }
    }

    fn ask(&self, tool: &str, args: &Value, key: &str) -> Result<(), String> {
        let waiting = value_items::count_waiting_with_prefix(&self.db, &self.prefix())
            .map_err(|_| lookup_failed())?;
        if waiting >= MAX_WAITING_APPROVALS {
            return Err(format!(
                "{MAX_WAITING_APPROVALS} approval requests from this chat are already waiting for \
                 the owner. Wait for them to answer before asking for more."
            ));
        }
        let shown = serde_json::to_string_pretty(&without_runtime_keys(args)).unwrap_or_default();
        if shown.chars().count() > MAX_APPROVAL_ARGS_CHARS {
            return Err(format!(
                "This {tool} call is too large for the owner to review in a message, so it cannot \
                 be approved. Split it into smaller calls."
            ));
        }
        let mut item = ValueItem::new(
            SOURCE,
            ValueKind::ActionNeeded,
            format!("HQ wants to run {tool}"),
            format!(
                "Your chat with HQ asks to run {tool} with exactly these arguments:\n\n{shown}\n\n\
                 Approve to let it run once. Dismiss to refuse."
            ),
        )
        .with_dedup_key(key.to_string());
        item.expires_at = Some(chrono::Utc::now() + chrono::Duration::hours(APPROVAL_TTL_HOURS));
        value_items::insert(&self.db, &item).map_err(|_| lookup_failed())
    }

    async fn wait(&self, tool: &str, key: &str) -> Option<String> {
        let deadline = tokio::time::Instant::now() + APPROVAL_WAIT;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(APPROVAL_POLL).await;
            match self.gate(tool, key) {
                Gate::Run => return None,
                Gate::Stop(text) => return Some(text),
                Gate::Ask | Gate::Wait => {}
            }
        }
        Some(format!(
            "Waiting for the owner to approve this {tool} call (sent to their HQ inbox). Tell them \
             what you want to run and why. Once they say it is approved, make exactly the same \
             call again; any change to the arguments needs a new approval."
        ))
    }
}

enum Gate {
    Run,
    Stop(String),
    Ask,
    Wait,
}

fn without_runtime_keys(args: &Value) -> Value {
    match args {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !k.starts_with(hq_core::approval_key::RUNTIME_ARG_PREFIX))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn declined(tool: &str) -> String {
    format!(
        "The owner declined this exact {tool} call. Do not retry it with the same arguments; ask \
         them what they want instead."
    )
}

fn lookup_failed() -> String {
    "HQ could not check the owner's approvals just now; nothing ran. Try again shortly.".into()
}

#[cfg(test)]
#[path = "owner_approval_tests.rs"]
mod tests;
