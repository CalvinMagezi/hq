use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::{
    PermissionMode, SecurityProfile, ToolResult, ToolResultContent, ValidationResult,
};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use super::injection::{injection_denial, without_default_drive};
use super::paths::{PATH_TOOLS, path_args_for_tool};
use super::taint::{TaintTracker, is_untrusted_source};
use super::{DenialNotifier, DenialTracker, RepeatCallTracker};
use crate::coding::text_result;
use crate::tools::AgentTool;

/// A tool wrapped with governance: path checking, call limits, and permission modes.
pub(super) struct GovernedTool {
    pub(super) inner: Box<dyn AgentTool>,
    pub(super) allowed_paths: Vec<PathBuf>,
    pub(super) denied_paths: Vec<PathBuf>,
    pub(super) profile: SecurityProfile,
    pub(super) permission_mode: PermissionMode,
    pub(super) denial_tracker: DenialTracker,
    pub(super) repeat_call_tracker: RepeatCallTracker,
    pub(super) denial_notifier: Option<DenialNotifier>,
    pub(super) notified_denials: Arc<Mutex<HashSet<String>>>,
    /// When in Plan mode, the only writable path (the active plan file).
    /// All other writes are denied. Set via `ToolGuardian::set_plan_file`.
    pub(super) plan_file_path: Option<PathBuf>,
    /// Shared set of file paths read during this session. Used to warn
    /// when a write targets a file that hasn't been read first.
    pub(super) files_read: Arc<Mutex<HashSet<PathBuf>>>,
    /// Mirrors `ToolGuardian::allow_defer`; gates `should_defer()`.
    pub(super) allow_defer: bool,
    /// Session taint shared with every other tool and with sub-agents.
    pub(super) taint: TaintTracker,
}

impl GovernedTool {
    /// Check if a path is within the allowed paths.
    pub(super) fn is_path_allowed(&self, path: &str) -> bool {
        // No silent bypass: if allowed_paths is empty, deny everything.
        // (ToolGuardian::new() already asserts this can't happen, but
        // defense-in-depth means we check here too.)
        if self.allowed_paths.is_empty() {
            return false;
        }

        let target = Path::new(path);

        // Attempt to canonicalize; fall back to the raw path
        let resolved = target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf());

        if self
            .denied_paths
            .iter()
            .any(|denied| resolved.starts_with(denied))
        {
            return false;
        }

        self.allowed_paths
            .iter()
            .any(|allowed| resolved.starts_with(allowed))
    }

    /// Surface a hard denial through `denial_notifier`, if attached, at most
    /// once per exact `(tool, reason)` pair this session and never once the
    /// session's total denial count has already saturated (`is_saturated`) —
    /// a wedged agent retrying the same or varying denied calls gets the
    /// operator one useful ping, not a pager storm.
    fn notify_denial(&self, reason: &str) {
        let Some(notifier) = &self.denial_notifier else {
            return;
        };
        if self.denial_tracker.is_saturated() {
            return;
        }
        let tool_name = self.inner.name();
        let key = format!("{tool_name}:{reason}");
        {
            let mut seen = self
                .notified_denials
                .lock()
                .expect("notified_denials lock poisoned");
            if !seen.insert(key) {
                return;
            }
        }
        notifier(tool_name, reason);
    }

    /// Check whether a write tool targets the active plan file.
    /// Returns true if the tool is writing to the plan file path (Plan mode exception).
    fn is_plan_file_write(&self, args: &Value) -> bool {
        let plan_path = match &self.plan_file_path {
            Some(p) => p,
            None => return false,
        };
        let tool_name = self.inner.name();
        let path_keys = path_args_for_tool(tool_name);
        for key in path_keys {
            if let Some(path_val) = args.get(*key).and_then(|v| v.as_str()) {
                let target = Path::new(path_val);
                let resolved = target
                    .canonicalize()
                    .unwrap_or_else(|_| target.to_path_buf());
                if resolved == *plan_path || target == plan_path.as_path() {
                    return true;
                }
            }
        }
        false
    }

    /// Track reads and check for writes to unread files. Call after successful execution.
    /// Returns a warning string to prepend if writing to an unread file, or None.
    fn track_read_before_write(&self, args: &Value) -> Option<String> {
        let tool_name = self.inner.name();
        let file_path = args.get("file_path").and_then(|v| v.as_str());
        let file_path = {
            let p = file_path?;
            PathBuf::from(p)
        };

        match tool_name {
            "read_file" | "read" => {
                let mut guard = self.files_read.lock().expect("files_read lock poisoned");
                guard.insert(file_path);
                None
            }
            "write_file" | "write" | "edit_file" | "edit" => {
                let guard = self.files_read.lock().expect("files_read lock poisoned");
                if !guard.contains(&file_path) {
                    debug!(
                        tool = tool_name,
                        path = %file_path.display(),
                        "write to file that was not read first in this session"
                    );
                    Some(
                        "\u{26a0} Warning: Writing to a file that wasn't read first \
                         in this session. Read the file first to avoid overwriting \
                         unintended content.\n\n"
                            .to_string(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Run the inner tool, enforcing its declared `timeout_ms()` if any.
    ///
    /// A tool with no declared timeout runs exactly as before (no behavior
    /// change). A tool that declares one races `self.inner.execute(...)`
    /// against a timer; on elapse this returns a structured `TOOL_TIMEOUT`
    /// result instead of the inner future's eventual output — the wrapper
    /// stops *waiting*, it does not (and cannot) kill cooperative work the
    /// tool itself does not cancel.
    async fn execute_with_timeout(&self, id: &str, args: Value) -> Result<ToolResult> {
        let Some(ms) = self.inner.timeout_ms() else {
            return self.inner.execute(id, args).await;
        };
        let tool_name = self.inner.name();
        match tokio::time::timeout(
            std::time::Duration::from_millis(ms),
            self.inner.execute(id, args),
        )
        .await
        {
            Ok(result) => result,
            Err(_elapsed) => {
                warn!(tool = tool_name, timeout_ms = ms, "tool call timed out");
                Ok(text_result(format!(
                    "Error: tool call timed out after {ms}ms"
                )))
            }
        }
    }

    /// Validate path arguments in the tool call.
    fn check_paths(&self, args: &Value) -> Option<String> {
        let tool_name = self.inner.name();
        let path_keys = path_args_for_tool(tool_name);

        for key in path_keys {
            if let Some(path_val) = args.get(*key).and_then(|v| v.as_str())
                && !self.is_path_allowed(path_val)
            {
                return Some(format!(
                    "Access denied: path '{}' is outside allowed directories. \
                         Allowed: {:?}",
                    path_val, self.allowed_paths
                ));
            }
        }

        None
    }
    /// Count a denial, logging once the session crosses the denial threshold.
    fn record_denial(&self, threshold_note: &str) {
        let tool_name = self.inner.name();
        if self.denial_tracker.record_denial(tool_name) {
            info!(
                tool = tool_name,
                session_total = self.denial_tracker.session_total(),
                "{threshold_note}"
            );
        }
    }

    /// Execute the inner tool, record success, and prepend the
    /// read-before-write warning when one applies.
    async fn run_tracked(&self, id: &str, args: Value) -> Result<ToolResult> {
        let warning = self.track_read_before_write(&args);
        let result = self.execute_with_timeout(id, args).await;
        if result.is_ok() {
            self.denial_tracker.record_success(self.inner.name());
        }
        match (warning, result) {
            (Some(warn_text), Ok(mut tr)) => {
                if let Some(first) = tr.content.first_mut() {
                    first.text = format!("{warn_text}{}", first.text);
                } else {
                    tr.content.push(ToolResultContent {
                        r#type: "text".to_string(),
                        text: warn_text,
                    });
                }
                Ok(tr)
            }
            (_, result) => result,
        }
    }

    /// BypassPermissions skips the permission-mode checks but still enforces
    /// path restrictions. A path denial here notifies without counting.
    async fn execute_bypassed(&self, id: &str, args: Value) -> Result<ToolResult> {
        let tool_name = self.inner.name();
        if PATH_TOOLS.contains(&tool_name)
            && let Some(denial) = self.check_paths(&args)
        {
            debug!(tool = tool_name, denial = %denial, "BypassPermissions — path still denied");
            self.notify_denial(&denial);
            return Ok(text_result(denial));
        }
        if let Some(denial) = injection_denial(tool_name, &args, &self.taint) {
            warn!(tool = tool_name, denial = %denial, "BypassPermissions: injection policy still denies");
            self.notify_denial(&denial);
            return Ok(text_result(denial));
        }
        debug!(
            tool = tool_name,
            "BypassPermissions — skipping permission checks"
        );
        self.run_tracked(id, args).await
    }

    /// The DontAsk and Plan mode denials, both distilled from claude-code:
    /// DontAsk allows only read-only tools, Plan also allows writes to the
    /// active plan file.
    fn permission_mode_denial(&self, args: &Value) -> Option<ToolResult> {
        let tool_name = self.inner.name();
        if self.inner.is_read_only() {
            return None;
        }
        if matches!(self.permission_mode, PermissionMode::DontAsk) {
            self.record_denial(
                "denial threshold exceeded — consider adding a permanent allow rule",
            );
            debug!(
                tool = tool_name,
                "DontAsk mode — denying non-read-only tool"
            );
            self.notify_denial("DontAsk mode blocked a non-read-only tool");
            return Some(text_result(format!(
                "Tool '{}' requires write access but the current permission mode \
                 (DontAsk) only allows read-only tools.",
                tool_name
            )));
        }
        if matches!(self.permission_mode, PermissionMode::Plan) && !self.is_plan_file_write(args) {
            self.record_denial("denial threshold exceeded in plan mode");
            debug!(
                tool = tool_name,
                "Plan mode — denying non-read-only, non-plan-file tool"
            );
            self.notify_denial("Plan mode blocked a non-read-only, non-plan-file tool");
            return Some(text_result(format!(
                "Plan mode active: tool '{}' requires write access but only \
                 read-only tools and plan file edits are allowed. \
                 Call exit_plan_mode when your plan is ready for approval.",
                tool_name
            )));
        }
        None
    }

    /// The full governance waterfall (call limits, permission-mode
    /// checks, path restrictions, validation) around a single tool call.
    /// Split out of `execute` so the trait method can wrap this with
    /// cross-cutting concerns — currently the repeat-call advisory —
    /// without duplicating the waterfall itself.
    async fn execute_governed(&self, id: &str, args: Value) -> Result<ToolResult> {
        let tool_name = self.inner.name();

        // Log destructive tool usage in Guarded mode (preparation for future interactive approval)
        if self.inner.is_destructive() && matches!(self.profile, SecurityProfile::Guarded) {
            debug!(
                tool = tool_name,
                "destructive tool invoked under Guarded security profile"
            );
        }

        if matches!(self.permission_mode, PermissionMode::BypassPermissions) {
            return self.execute_bypassed(id, args).await;
        }

        if let Some(denied) = self.permission_mode_denial(&args) {
            return Ok(denied);
        }

        // Check path restrictions for filesystem tools.
        // AcceptEdits and Plan modes still enforce path restrictions.
        if PATH_TOOLS.contains(&tool_name)
            && let Some(denial) = self.check_paths(&args)
        {
            self.record_denial(
                "denial threshold exceeded — consider adding a permanent allow rule",
            );
            debug!(
                tool = tool_name,
                denial = %denial,
                "path access denied by governance"
            );
            self.notify_denial(&denial);
            return Ok(text_result(denial));
        }

        // Secret files and tainted-session egress, independent of what the
        // model was told. See docs/security/PROMPT_INJECTION.md.
        if let Some(denial) = injection_denial(tool_name, &args, &self.taint) {
            self.record_denial("denial threshold exceeded under the injection policy");
            warn!(tool = tool_name, denial = %denial, "denied by injection policy");
            self.notify_denial(&denial);
            return Ok(text_result(denial));
        }

        // Run tool-level input validation before any I/O.
        // Distilled from claude-code's Tool.validateInput() pipeline.
        // AcceptEdits auto-approves edits but still validates inputs structurally.
        if let ValidationResult::Err {
            message,
            error_code,
            ..
        } = self.inner.validate(&args).await
        {
            debug!(tool = tool_name, error_code, "tool validation failed");
            return Ok(text_result(format!(
                "Validation error (code {error_code}): {message}"
            )));
        }

        self.run_tracked(id, args).await
    }
}

#[async_trait]
impl AgentTool for GovernedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters(&self) -> Value {
        self.inner.parameters()
    }

    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }

    fn is_destructive(&self) -> bool {
        self.inner.is_destructive()
    }

    fn requires_live_user_turn(&self) -> bool {
        self.inner.requires_live_user_turn()
    }

    // Governance wraps every tool in the session, so anything this impl fails
    // to forward collapses to the `AgentTool` trait default for the whole
    // registry. That is why `should_defer` was permanently false (deferred
    // schemas and the `tool_search` meta-tool were unreachable) and why
    // `category` read "general" everywhere downstream.
    fn category(&self) -> &str {
        self.inner.category()
    }

    fn search_hint(&self) -> &str {
        self.inner.search_hint()
    }

    fn should_defer(&self) -> bool {
        self.allow_defer && self.inner.should_defer()
    }

    fn tool_policy(&self) -> hq_tools::registry::ToolPolicy {
        self.inner.tool_policy()
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        self.inner.behavioral_prompt()
    }

    async fn execute(&self, id: &str, args: Value) -> Result<ToolResult> {
        let tool_name = self.inner.name();
        let repeat_advisory = self.repeat_call_tracker.record_call(tool_name, &args);
        let args = without_default_drive(tool_name, args, &self.taint);
        let mut result = self.execute_governed(id, args).await;
        if is_untrusted_source(tool_name, self.inner.category()) {
            self.taint.mark(tool_name);
        }
        if let Ok(tr) = &mut result
            && let Some(advisory) = repeat_advisory
        {
            tr.context_modifier = Some(match tr.context_modifier.take() {
                Some(existing) => format!("{existing}\n\n{advisory}"),
                None => advisory,
            });
        }
        result
    }
}
