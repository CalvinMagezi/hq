//! Tool governance — security profiles, path-based access control, permission
//! modes, and denial tracking.
//!
//! # Architecture (distilled from claude-code)
//!
//! Governance is layered:
//!
//! 1. **SecurityProfile** — compile-time floor. Enforced at `ToolGuardian`
//!    construction; cannot be overridden at runtime.
//! 2. **PermissionMode** — runtime interaction mode. Controls whether the
//!    executor prompts for approval, auto-approves read-only tools, or
//!    bypasses all checks.
//! 3. **DenialTracker** — tracks per-tool consecutive and total denials.
//!    After `MAX_CONSECUTIVE_DENIALS` consecutive denials or `MAX_TOTAL_DENIALS`
//!    total, surfaces a suggestion to create a permanent allow/deny rule.
//!    Distilled from claude-code's `denialTracking.ts`.
//! 4. **Path checks** — filesystem tools restricted to `allowed_paths`.
//! 5. **Injection policy** — credential files are never reachable, and once
//!    an untrusted-content tool has run (`TaintTracker`), project secrets and
//!    outbound network from bash are denied too.
//! 6. **Validation gate** — runs each tool's `validate()` before execution.

use hq_core::types::{PermissionMode, SecurityProfile};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::debug;

use crate::tools::{AgentTool, ToolRegistry};

#[cfg(test)]
mod bypass_corpus_tests;
mod egress;
mod governed_tool;
mod injection;
mod paths;
mod secrets;
mod taint;
mod trackers;

use governed_tool::GovernedTool;
pub use paths::system_temp_paths;
use paths::{expand_with_canonical_forms, sensitive_denied_paths};
pub use secrets::{SecretTier, always_denied_roots, classify_path};
pub use taint::{TaintTracker, is_untrusted_source};
pub use trackers::{DenialTracker, RepeatCallTracker};


// ─── Security floor ─────────────────────────────────────────────

/// Minimum security profile allowed. Profiles below this floor are
/// rejected at ToolGuardian construction time. This constant is
/// enforced in code and cannot be overridden by configuration.
const MIN_SECURITY_FLOOR: SecurityProfile = SecurityProfile::Standard;

/// Returns true if `profile` meets or exceeds the minimum security floor.
fn meets_security_floor(profile: &SecurityProfile) -> bool {
    let level = |p: &SecurityProfile| -> u8 {
        match p {
            SecurityProfile::Minimal => 0,
            SecurityProfile::Standard => 1,
            SecurityProfile::Guarded => 2,
            SecurityProfile::Admin => 3,
        }
    };
    level(profile) >= level(&MIN_SECURITY_FLOOR)
}
// ─── GovernedRegistry ───────────────────────────────────────────

/// A tool registry that has passed through governance checks.
///
/// This type can only be constructed via [`ToolGuardian::build_registry`],
/// ensuring that all tools are wrapped with path checking and call limits.
/// `AgentSession::new()` requires this type, making it impossible to
/// create an ungoverned session.
pub struct GovernedRegistry {
    inner: ToolRegistry,
}

impl std::ops::Deref for GovernedRegistry {
    type Target = ToolRegistry;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Attests whether the tools passed to `build_registry` belong to a session
/// a live user turn is directly driving. A typed newtype rather than a bare
/// `bool` so a call site has to name what it's asserting —
/// `LiveUserTurn::from_session_config(...)` reads it off the session's own
/// config instead of a caller conjuring a bare `true`/`false` inline, which
/// would make it too easy for a future call site to silently reopen the gate
/// this type exists to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveUserTurn(bool);

impl LiveUserTurn {
    /// Read liveness off a session's own config — the normal way to obtain one.
    pub fn from_session_config(config: &crate::session::SessionConfig) -> Self {
        Self(config.is_live_user_turn)
    }

    /// Explicit, named opt-out for the few call sites that build a registry
    /// without a `SessionConfig` in hand yet (or that are known-unattended).
    pub fn unattended() -> Self {
        Self(false)
    }

    fn is_live(self) -> bool {
        self.0
    }
}

/// Callback fired when a tool call is hard-denied and this exact
/// `(tool, reason)` pair hasn't been surfaced yet this session. `None` (the
/// default) means denials are only visible in the tool result text and
/// tracing — no external notification.
///
/// Wired at session-construction sites via `ToolGuardian::set_denial_notifier`,
/// never called directly from this module: `hq-agent` has no vault, mailbox,
/// or bus knowledge by design, so `cargo test -p hq-agent` can never page
/// anyone.
pub type DenialNotifier = Arc<dyn Fn(&str, &str) + Send + Sync>;

// ─── ToolGuardian ───────────────────────────────────────────────

/// Governs tool execution with path restrictions and call limits.
#[derive(Clone)]
pub struct ToolGuardian {
    allowed_paths: Vec<PathBuf>,
    denied_paths: Vec<PathBuf>,
    profile: SecurityProfile,
    permission_mode: PermissionMode,
    denial_tracker: DenialTracker,
    /// Session-scoped tracker advising the model when it repeats an
    /// identical tool call too many times in a row. Advisory only — never
    /// blocks, see `RepeatCallTracker`.
    repeat_call_tracker: RepeatCallTracker,
    denial_notifier: Option<DenialNotifier>,
    /// Denial reasons already surfaced via `denial_notifier` this session,
    /// keyed by `"{tool_name}:{reason}"`. Keeps a wedged agent retrying the
    /// same denied call from paging the operator on every retry.
    notified_denials: Arc<Mutex<HashSet<String>>>,
    /// In Plan mode, the single writable path (the plan file).
    plan_file_path: Option<PathBuf>,
    /// Tracks file paths that have been read during this session.
    /// Used to emit soft warnings when a write targets an unread file.
    files_read: Arc<Mutex<HashSet<PathBuf>>>,
    /// When false, `should_defer()` is forced off for every governed tool and
    /// all schemas ship every turn. Deferral was unreachable before governance
    /// started forwarding `should_defer`; this is the switch to put it back if
    /// a model handles the `tool_search` indirection badly.
    allow_defer: bool,
    /// Session taint, shared with sub-agents via `set_taint`.
    taint: TaintTracker,
}

impl std::fmt::Debug for ToolGuardian {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolGuardian")
            .field("allowed_paths", &self.allowed_paths)
            .field("denied_paths", &self.denied_paths)
            .field("profile", &self.profile)
            .field("permission_mode", &self.permission_mode)
            .field("has_denial_notifier", &self.denial_notifier.is_some())
            .field("tainted_by", &self.taint.source())
            .finish()
    }
}

impl ToolGuardian {
    /// Create a new guardian with the given allowed paths, security profile, and permission mode.
    ///
    /// # Panics
    ///
    /// Panics if `allowed_paths` is empty (would silently disable all path
    /// checks) or if `profile` is below the minimum security floor.
    pub fn new(
        allowed_paths: Vec<PathBuf>,
        profile: SecurityProfile,
        permission_mode: PermissionMode,
    ) -> Self {
        assert!(
            !allowed_paths.is_empty(),
            "ToolGuardian requires at least one allowed path. \
             An empty allowlist would silently disable all path restrictions."
        );
        assert!(
            meets_security_floor(&profile),
            "SecurityProfile::{:?} is below the minimum floor ({:?}). \
             Use Standard or higher.",
            profile,
            MIN_SECURITY_FLOOR
        );

        // This machine is single-tenant (RFC-001 §7): there is no second
        // user to sandbox hq away from, so the allowlist's job is to keep
        // raw file tools off credential material, not to confine hq to a
        // project directory. Every caller-supplied root plus $HOME goes in;
        // sensitive_denied_paths() below carves out what never should.
        let mut roots = allowed_paths;
        if let Some(home) = dirs::home_dir() {
            roots.push(home);
        }

        Self {
            allowed_paths: expand_with_canonical_forms(roots),
            denied_paths: expand_with_canonical_forms(sensitive_denied_paths()),
            profile,
            permission_mode,
            denial_tracker: DenialTracker::new(),
            repeat_call_tracker: RepeatCallTracker::new(),
            denial_notifier: None,
            notified_denials: Arc::new(Mutex::new(HashSet::new())),
            plan_file_path: None,
            files_read: Arc::new(Mutex::new(HashSet::new())),
            allow_defer: true,
            taint: TaintTracker::new(),
        }
    }

    /// Share a taint tracker, so a sub-agent's session and its parent see
    /// the same taint in both directions.
    pub fn set_taint(&mut self, taint: TaintTracker) {
        self.taint = taint;
    }

    /// The session's taint tracker.
    pub fn taint(&self) -> &TaintTracker {
        &self.taint
    }

    /// Attach a callback for surfacing hard denials outside the tool result
    /// text (e.g. a Telegram ping via the operator's mailbox). See
    /// `DenialNotifier` for the contract and why this crate never implements
    /// the sending itself.
    pub fn set_denial_notifier(&mut self, notifier: DenialNotifier) {
        self.denial_notifier = Some(notifier);
    }

    /// Create a guardian with `PermissionMode::Default` (convenience).
    pub fn with_default_mode(allowed_paths: Vec<PathBuf>, profile: SecurityProfile) -> Self {
        Self::new(allowed_paths, profile, PermissionMode::Default)
    }

    /// Create a guardian from a named `PermissionPreset` instead of a raw
    /// `(SecurityProfile, PermissionMode)` pair. Additive convenience —
    /// does not replace `new`/`with_default_mode`.
    #[cfg(test)]
    pub fn from_preset(
        preset: hq_core::types::PermissionPreset,
        allowed_paths: Vec<PathBuf>,
    ) -> Self {
        let (profile, mode) = preset.profile_and_mode();
        Self::new(allowed_paths, profile, mode)
    }

    /// Set the active plan file path. Required when `PermissionMode::Plan`
    /// is active so writes targeting the plan file are permitted.
    pub fn set_plan_file(&mut self, path: PathBuf) {
        self.plan_file_path = Some(path);
    }

    /// Wrap a tool with governance checks.
    pub fn govern(&self, tool: Box<dyn AgentTool>) -> Box<dyn AgentTool> {
        Box::new(GovernedTool {
            inner: tool,
            allowed_paths: self.allowed_paths.clone(),
            denied_paths: self.denied_paths.clone(),
            profile: self.profile.clone(),
            permission_mode: self.permission_mode.clone(),
            denial_tracker: self.denial_tracker.clone(),
            repeat_call_tracker: self.repeat_call_tracker.clone(),
            denial_notifier: self.denial_notifier.clone(),
            notified_denials: self.notified_denials.clone(),
            plan_file_path: self.plan_file_path.clone(),
            files_read: self.files_read.clone(),
            allow_defer: self.allow_defer,
            taint: self.taint.clone(),
        })
    }

    /// Build a governed registry from a list of tools.
    ///
    /// Every tool is wrapped with governance (path checking + call limits)
    /// before being inserted into the registry. The returned
    /// [`GovernedRegistry`] is the only way to create an `AgentSession`.
    ///
    /// `live` attests whether a live user turn is driving this session —
    /// see [`LiveUserTurn`]. A tool whose `requires_live_user_turn()` is
    /// `true` is excluded whenever `live` is not live. This is the
    /// authoritative, unbypassable gate: `GovernedRegistry` has no other
    /// constructor, so there is no way for such a tool to reach a live
    /// `AgentSession` from an unattended session, regardless of any
    /// allow-list applied elsewhere.
    pub fn build_registry(&self, tools: Vec<Box<dyn AgentTool>>, live: LiveUserTurn) -> GovernedRegistry {
        let mut registry = ToolRegistry::new();
        for tool in tools {
            if !live.is_live() && tool.requires_live_user_turn() {
                debug!(tool = %tool.name(), "excluded: requires a live user turn, session is unattended");
                continue;
            }
            registry.register(self.govern(tool));
        }
        GovernedRegistry { inner: registry }
    }

    /// Get the security profile.
    pub fn profile(&self) -> &SecurityProfile {
        &self.profile
    }

    /// Get the permission mode.
    pub fn permission_mode(&self) -> &PermissionMode {
        &self.permission_mode
    }

    /// Get a reference to the shared denial tracker.
    pub fn denial_tracker(&self) -> &DenialTracker {
        &self.denial_tracker
    }

    /// Get a reference to the shared repeat-call tracker.
    pub fn repeat_call_tracker(&self) -> &RepeatCallTracker {
        &self.repeat_call_tracker
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod injection_tests;
