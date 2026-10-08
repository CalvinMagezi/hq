//! What an orchestrator is told, and what gets recorded, when it reaches for
//! something outside its role.

use std::collections::HashSet;
use std::sync::Mutex;

use crate::governance::DenialNotifier;

const ROUTING_HINT: &str = "Delegate it: start a coding session with harness_session_spawn \
    (task_id, goal, done criteria) and watch and steer it, or use spawn_subagents for read-only research.";

/// Output fragments a read-only sandbox produces when a command tries to write.
// ponytail: substring match on OS error text, misses a tool that words its own error; upgrade to a sandbox-reported flag if it matters.
const WRITE_BLOCKED_MARKERS: &[&str] = &["Read-only file system", "Operation not permitted"];

pub(crate) struct RoleDenial {
    removed: &'static [&'static str],
    notifier: DenialNotifier,
    seen: Mutex<HashSet<String>>,
}

impl RoleDenial {
    pub(crate) fn new(removed: &'static [&'static str], notifier: DenialNotifier) -> Self {
        Self {
            removed,
            notifier,
            seen: Mutex::new(HashSet::new()),
        }
    }

    /// The reply for a call to a tool this role does not have, or `None` if the name is unknown for another reason.
    pub(crate) fn for_removed_tool(&self, tool: &str) -> Option<String> {
        if !self.removed.contains(&tool) {
            return None;
        }
        self.notify(tool, "not available in the orchestrator role");
        Some(format!(
            "`{tool}` is not available in the orchestrator role. {ROUTING_HINT}"
        ))
    }

    /// Extra text to append to a bash result that was stopped by the read-only sandbox.
    pub(crate) fn for_blocked_write(&self, output: &str) -> Option<String> {
        if !WRITE_BLOCKED_MARKERS.iter().any(|m| output.contains(m)) {
            return None;
        }
        self.notify("bash", "write blocked by the read-only sandbox");
        Some(format!(
            "The orchestrator's shell is read-only. {ROUTING_HINT}"
        ))
    }

    /// Reports each distinct (tool, reason) once, so a retry loop pages once.
    fn notify(&self, tool: &str, reason: &str) {
        let first = self
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(format!("{tool}:{reason}"));
        if first {
            (self.notifier)(tool, reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn denial() -> (RoleDenial, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let notifier: DenialNotifier = Arc::new(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        (RoleDenial::new(&["edit_file"], notifier), count)
    }

    #[test]
    fn a_removed_tool_gets_the_hint_and_one_audit_entry_however_often_it_retries() {
        let (denial, count) = denial();
        for _ in 0..10 {
            let reply = denial.for_removed_tool("edit_file").unwrap();
            assert!(reply.contains("harness_session_spawn"));
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(denial.for_removed_tool("no_such_tool").is_none());
    }

    #[test]
    fn a_blocked_shell_write_gets_the_hint() {
        let (denial, count) = denial();
        assert!(denial.for_blocked_write("touch: x: Read-only file system").is_some());
        assert!(denial.for_blocked_write("total 0").is_none());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
