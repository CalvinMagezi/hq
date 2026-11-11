//! Session taint: once a tool returns content an outsider could have
//! written, later privileged calls are held to a stricter policy no matter
//! what the model says.

use regex::Regex;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

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
    "vault_import",
    "ocr_extract_text",
    "agent_read_inbox",
    "harness_session_logs",
    "harness_session_pending",
    "harness_session_events",
    "task_decisions",
    "host_read",
    "session_search",
    "history_restore",
    "tool_output_read",
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
    // Titles and calendar names come from invitations and shared calendars.
    hq_tools::calendar::LIST_TOOL_NAME,
    "hq",
];

/// Tool category whose every member is an untrusted source.
const UNTRUSTED_SOURCE_CATEGORY: &str = "remote_mcp";

/// Whether a tool's result should taint the session.
pub fn is_untrusted_source(tool_name: &str, category: &str) -> bool {
    category == UNTRUSTED_SOURCE_CATEGORY || UNTRUSTED_SOURCE_TOOLS.contains(&tool_name)
}

/// A host name in the owner's words: in a URL, or bare like `docs.example.com`.
static OWNER_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b((?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z]{2,63})\b")
        .expect("static host pattern must compile")
});

/// Session-scoped taint flag, shared by every governed tool in a session and
/// by the sub-agents it spawns, so delegation cannot launder it. It also
/// carries the two facts the tainted-session rules need beside it: which hosts
/// the owner named in their own messages, and whether anyone is there to ask.
#[derive(Debug, Clone, Default)]
pub struct TaintTracker {
    source: Arc<Mutex<Option<String>>>,
    owner_hosts: Arc<Mutex<HashSet<String>>>,
    unattended: Arc<AtomicBool>,
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

    /// Remember the hosts named in text the owner wrote themselves (their chat
    /// messages, a watch they created). Never text a tool returned or HQ wrote.
    pub fn note_owner_words(&self, text: &str) {
        let mut hosts = self.owner_hosts.lock().expect("TaintTracker lock poisoned");
        for m in OWNER_HOST.captures_iter(text).filter_map(|c| c.get(1)) {
            hosts.insert(m.as_str().to_ascii_lowercase());
        }
    }

    /// Whether `host` is one the owner named, or a subdomain of one.
    pub fn owner_named_host(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let hosts = self.owner_hosts.lock().expect("TaintTracker lock poisoned");
        hosts
            .iter()
            .any(|named| host == *named || host.ends_with(&format!(".{named}")))
    }

    /// Mark the session as one nobody is there to approve: a watch firing, a
    /// turn the web driver or a follow-up started. Shared with sub-agents.
    pub fn set_unattended(&self) {
        self.unattended.store(true, Ordering::Relaxed);
    }

    pub fn is_unattended(&self) -> bool {
        self.unattended.load(Ordering::Relaxed)
    }
}
