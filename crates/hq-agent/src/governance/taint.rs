//! Session taint: once a tool returns content an outsider could have
//! written, later privileged calls are held to a stricter policy no matter
//! what the model says.

use std::sync::{Arc, Mutex};

/// Tools whose results carry text from outside the operator's control: the
/// web, email, converted documents, other agents' mailboxes, terminal output
/// of other agents, and vault notes (which hold clipped pages and mail).
const UNTRUSTED_SOURCE_TOOLS: &[&str] = &[
    "web_fetch",
    "web_search",
    "github_read",
    "github_clone",
    "google_workspace",
    "convert_to_markdown",
    "ocr_extract_text",
    "agent_read_inbox",
    "harness_session_logs",
    "host_read",
    "session_search",
    "vault_read",
    "vault_batch_read",
    "vault_read_section",
    "vault_search",
    "vault_find",
    "vault_find_similar",
    "vault_context",
    "context_packet",
    "subagent_run_status",
    "subagent_run_result",
    "hq",
];

/// Tool category whose every member is an untrusted source.
const UNTRUSTED_SOURCE_CATEGORY: &str = "remote_mcp";

/// Whether a tool's result should taint the session.
pub fn is_untrusted_source(tool_name: &str, category: &str) -> bool {
    category == UNTRUSTED_SOURCE_CATEGORY || UNTRUSTED_SOURCE_TOOLS.contains(&tool_name)
}

/// Session-scoped taint flag, shared by every governed tool in a session and
/// by the sub-agents it spawns, so delegation cannot launder it.
#[derive(Debug, Clone, Default)]
pub struct TaintTracker {
    source: Arc<Mutex<Option<String>>>,
}

impl TaintTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that `tool_name` returned untrusted content. The first source
    /// is kept for the denial message.
    pub fn mark(&self, tool_name: &str) {
        let mut guard = self.source.lock().expect("TaintTracker lock poisoned");
        if guard.is_none() {
            *guard = Some(tool_name.to_string());
        }
    }

    /// The tool that first tainted this session, if any.
    pub fn source(&self) -> Option<String> {
        self.source
            .lock()
            .expect("TaintTracker lock poisoned")
            .clone()
    }

    pub fn is_tainted(&self) -> bool {
        self.source().is_some()
    }
}
