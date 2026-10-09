//! What an orchestrator is told, and what gets recorded, when it reaches for
//! something outside its role.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::governance::DenialNotifier;

const ROUTING_HINT: &str = "Delegate it: start a coding session with harness_session_spawn \
    (task_id, goal, done criteria) and watch and steer it, or use spawn_subagents for read-only research.";

/// Marks where the bash tool starts printing stderr.
const STDERR_LABEL: &str = "STDERR:\n";

/// Output fragments a read-only sandbox produces when a command tries to write.
// ponytail: substring match on OS error text, misses a tool that words its own error; upgrade to a sandbox-reported flag if it matters.
const WRITE_BLOCKED_MARKERS: &[&str] = &["Read-only file system", "Operation not permitted"];

/// Tools that write to a path the caller names, and the argument that holds it.
const OUTPUT_PATH_ARGS: &[(&str, &str)] = &[
    ("convert_from_markdown", "output"),
    ("vault_export", "output"),
    ("vault_export_pdf", "output"),
];

pub(crate) struct RoleDenial {
    removed: &'static [&'static str],
    notifier: DenialNotifier,
    seen: Mutex<HashSet<String>>,
    /// Where tools that take an output path may write.
    output_roots: Vec<PathBuf>,
}

/// Absolute, `.`/`..`-free, with symlinks resolved on the part that exists.
/// `None` when a link on the way cannot be resolved (a dangling link could point anywhere).
fn resolve(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut clean = PathBuf::new();
    for c in absolute.components() {
        match c {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    let mut tail = Vec::new();
    let mut head = clean.clone();
    while head.symlink_metadata().is_err() {
        match head.file_name().map(|n| n.to_os_string()) {
            Some(name) => tail.push(name),
            None => break,
        }
        head.pop();
    }
    let mut resolved = head.canonicalize().ok()?;
    resolved.extend(tail.into_iter().rev());
    Some(resolved)
}

impl RoleDenial {
    pub(crate) fn new(
        removed: &'static [&'static str],
        notifier: DenialNotifier,
        output_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            removed,
            notifier,
            seen: Mutex::new(HashSet::new()),
            output_roots: output_roots.iter().filter_map(|r| resolve(r)).collect(),
        }
    }

    /// Refuses a write-to-path tool whose destination is outside the allowed roots.
    pub(crate) fn refuse_output(&self, tool: &str, args: &Value) -> Option<String> {
        let (_, key) = OUTPUT_PATH_ARGS.iter().find(|(t, _)| *t == tool)?;
        let dest = resolve(Path::new(args.get(*key)?.as_str()?));
        if dest
            .as_ref()
            .is_some_and(|d| self.output_roots.iter().any(|root| d.starts_with(root)))
        {
            return None;
        }
        self.notify(tool, "output outside the orchestrator's write roots");
        Some(format!(
            "`{tool}` may only write under the vault's Notebooks folder or the temp directory in the orchestrator role. \
             Choose an output path there, or {ROUTING_HINT}"
        ))
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
        // Only stderr counts: a command can print these words from a file it reads.
        let (_, stderr) = output.split_once(STDERR_LABEL)?;
        if !WRITE_BLOCKED_MARKERS.iter().any(|m| stderr.contains(m)) {
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
        (
            RoleDenial::new(&["edit_file"], notifier, vec![PathBuf::from("/tmp")]),
            count,
        )
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
        assert!(
            denial
                .for_blocked_write("STDERR:\ntouch: x: Read-only file system\n(exit code 1)")
                .is_some()
        );
        assert!(
            denial
                .for_blocked_write("exit=1\n\nSTDERR:\ntouch: x: Read-only file system\n")
                .is_some(),
            "still caught when the command's own exit status hides the failure"
        );
        assert!(
            denial
                .for_blocked_write("Read-only file system in stdout\nSTDERR:\nwarning\n")
                .is_none()
        );
        assert!(
            denial
                .for_blocked_write("grep hit: Read-only file system in a source file")
                .is_none()
        );
        assert!(denial.for_blocked_write("total 0").is_none());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn output_paths_outside_the_write_roots_are_refused_including_dotdot_and_symlinks() {
        let root = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let notifier: DenialNotifier = Arc::new(|_, _| {});
        let denial = RoleDenial::new(&[], notifier, vec![root.path().to_path_buf()]);
        let args = |p: &Path| serde_json::json!({"output": p.display().to_string()});
        assert!(
            denial
                .refuse_output(
                    "convert_from_markdown",
                    &args(&root.path().join("a/b.docx"))
                )
                .is_none()
        );
        assert!(
            denial
                .refuse_output(
                    "convert_from_markdown",
                    &args(&outside.path().join("x.html"))
                )
                .is_some()
        );
        let sneaky = root
            .path()
            .join("../")
            .join(outside.path().file_name().unwrap())
            .join("x.html");
        assert!(
            denial
                .refuse_output("convert_from_markdown", &args(&sneaky))
                .is_some()
        );
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        assert!(
            denial
                .refuse_output(
                    "convert_from_markdown",
                    &args(&root.path().join("link/x.html"))
                )
                .is_some()
        );
        std::os::unix::fs::symlink(
            outside.path().join("missing/target"),
            root.path().join("dangling"),
        )
        .unwrap();
        assert!(
            denial
                .refuse_output(
                    "convert_from_markdown",
                    &args(&root.path().join("dangling"))
                )
                .is_some(),
            "a dangling link can point anywhere"
        );
        assert!(
            denial
                .refuse_output("other_tool", &args(&outside.path().join("x")))
                .is_none()
        );
    }
}
