//! Session builder — centralizes `AgentSession` construction.
//!
//! Extracts the provider + tools + governance + context wiring that was previously
//! scattered across CLI commands and relay bots. All entry points
//! (CLI, relay, web API) should use `SessionBuilder` to create sessions.
//!
//! Context assembly uses the full 5-layer `ContextEngine` with token-budgeted
//! knapsack allocation, dynamic `MemoryQuerier` retrieval, and cross-session
//! thread continuity from `.vault/_threads/`.

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

use hq_core::config::HqConfig;
use hq_core::types::{PermissionMode, PermissionPreset, SecurityProfile, SessionEvent};
use hq_llm::provider::LlmProvider;

use crate::session::{AgentSession, SessionConfig, SessionMode};
use crate::tools::AgentTool;

pub(crate) mod prompt;
mod provider;
mod tools;

use prompt::*;

/// Controls which tool tier is injected into a session.
///
/// `Weak` — only shortcut tools (Weak policy). For relay bots, local models,
///           free-budget sub-agents. Fewer tools = fewer reasoning steps.
/// `Standard` — all tools including shortcuts (Standard + Weak policies).
/// `Full` — every tool including destructive/complex ops (Full policy).
#[derive(Debug, Clone, Copy, Default)]
pub enum SessionProfile {
    Weak,
    #[default]
    Standard,
    Full,
}

impl SessionProfile {
    pub(crate) fn max_policy(&self) -> hq_tools::registry::ToolPolicy {
        match self {
            SessionProfile::Weak => hq_tools::registry::ToolPolicy::Weak,
            SessionProfile::Standard => hq_tools::registry::ToolPolicy::Standard,
            SessionProfile::Full => hq_tools::registry::ToolPolicy::Full,
        }
    }
}

/// What a session is for. The Implementor does the work itself; the
/// Orchestrator plans, delegates, monitors and reports, and is held to that by
/// the tool catalog and sandbox rather than by its prompt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionRole {
    #[default]
    Implementor,
    Orchestrator,
}

/// Tools an orchestrator session never gets. They change files, git history or
/// HQ's own config; that work goes to a coding session or child agent instead.
/// Chosen by effect: every tool that writes outside the vault and task stores.
pub const ORCHESTRATOR_REMOVED_TOOLS: &[&str] = &[
    "edit_file",
    "write_file",
    "file_edit_batch",
    "rollback_file",
    "git_commit",
    "git_pr",
    "config_manage",
];

/// Tools that survive the `SessionProfile` filter regardless of their
/// `tool_policy()`.
///
/// The Weak tier is a token optimization, not a lobotomy. A relay turn
/// classified as vault-shaped (the trigger is the bare word "note",
/// "notebook", "remember", or "vault") was losing shell and file access
/// entirely, so "remember to check the deploy" produced a session that could
/// not read a file. Deliberately enforced here at the profile filter rather
/// than in `tool_policy::filter`: profile tiering is a coarse token
/// heuristic, but a named agent's explicit `deny_tools` is an operator
/// decision and must still be able to remove `bash`.
const CAPABILITY_FLOOR: &[&str] = &["bash", "read_file", "write_file", "edit_file", "web_search"];

// --- HqTool -> AgentTool adapter ------------------------------------------

/// Wraps an `hq_tools::HqTool` to implement `AgentTool`, bridging the
/// MCP tool interface with the agent session's tool interface.
pub(crate) struct HqToolAdapter {
    pub(crate) inner: Box<dyn hq_tools::HqTool>,
}

#[async_trait::async_trait]
impl AgentTool for HqToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn parameters(&self) -> serde_json::Value {
        self.inner.parameters()
    }
    fn category(&self) -> &str {
        self.inner.category()
    }
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }
    fn is_destructive(&self) -> bool {
        // hq_tools::HqTool now has its own is_destructive() (default
        // !is_read_only(), the same proxy this adapter used to apply here
        // directly) — forward the tool's real answer instead of
        // re-deriving one, so a tool that overrides it (e.g. the crypto
        // tools' CryptoToolAdapter) is respected instead of guessed at.
        self.inner.is_destructive()
    }
    fn requires_live_user_turn(&self) -> bool {
        self.inner.requires_live_user_turn()
    }
    async fn execute(
        &self,
        _id: &str,
        args: serde_json::Value,
    ) -> Result<hq_core::types::ToolResult> {
        let value = hq_tools::registry::validate_and_execute(self.inner.as_ref(), args).await?;
        let text = match &value {
            serde_json::Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        };
        Ok(hq_core::types::ToolResult {
            content: vec![hq_core::types::ToolResultContent {
                r#type: "text".into(),
                text,
            }],
            details: None,
            context_modifier: None,
        })
    }

    fn tool_policy(&self) -> hq_tools::registry::ToolPolicy {
        self.inner.tool_policy()
    }

    /// `HqTool::effective_hint` returns a `Cow`, which cannot satisfy this
    /// `-> &str` signature. Return the hand-written hint when there is one and
    /// let the caller derive from the description otherwise —
    /// `derive_search_hint` is idempotent, so applying it at format time to an
    /// already-short hint is a no-op.
    fn search_hint(&self) -> &str {
        self.inner
            .search_hint()
            .unwrap_or_else(|| self.inner.description())
    }

    fn timeout_ms(&self) -> Option<u64> {
        self.inner.timeout_ms()
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        self.inner.behavioral_prompt()
    }
}

/// Build a `DenialNotifier` that surfaces a hard tool denial to the operator
/// by dropping a message in the `relay` mailbox — the same delivery path
/// `agent_send_message(recipient="relay")` uses, which the mailbox poller
/// forwards to whichever of Telegram/Discord the operator is active on
/// within ~45s. This is the one place that turns a governance denial into
/// operator-visible output; `governance.rs` itself only knows about the
/// callback type, so unit tests there can never trigger a real mailbox
/// write. Fire-and-forget: a write failure is logged and swallowed, never
/// allowed to affect the tool call it's reporting on.
pub fn mailbox_denial_notifier(vault_path: PathBuf) -> crate::governance::DenialNotifier {
    Arc::new(move |tool_name: &str, reason: &str| {
        let vault_path = vault_path.clone();
        let mut msg = hq_core::mailbox::new_message(
            "hq",
            "relay",
            hq_core::types::MailboxMessageType::Nudge,
            Some("HQ tool access denied"),
            &format!("Tool '{tool_name}' was denied: {reason}"),
            None,
        );
        // Same-tick delivery: an operator should know about a governance
        // denial immediately, not on the next digest cycle.
        msg.meta.insert(
            hq_core::mailbox::META_INTERRUPT.to_string(),
            "true".to_string(),
        );
        tokio::spawn(async move {
            if let Err(e) = hq_core::mailbox::send_message(&vault_path, &msg) {
                tracing::warn!(error = %e, "failed to surface tool denial to relay mailbox");
            }
        });
    })
}

// ─── SessionBuilder ────────────────────────────────────────────

/// Builder for `AgentSession` that centralizes all wiring.
///
/// # Usage
///
/// ```ignore
/// let (tx, _rx) = broadcast::channel(256);
/// let mut session = SessionBuilder::from_config(&config)
///     .event_channel(tx)
///     .system_prompt("You are a helpful agent.".into())
///     .build()
///     .await?;
/// let result = session.prompt("Hello").await?;
/// ```
pub struct SessionBuilder {
    config: HqConfig,
    system_prompt: Option<String>,
    /// Harness-specific instructions appended after the soul (not a replacement).
    /// Callers that built `soul + RELAY_INSTRUCTIONS` strings should switch to this.
    harness_instructions: Option<String>,
    event_tx: Option<broadcast::Sender<SessionEvent>>,
    extra_allowed_paths: Vec<PathBuf>,
    working_dir: Option<PathBuf>,
    provider_override: Option<Arc<dyn LlmProvider>>,
    session_config_override: Option<SessionConfig>,
    extra_tools: Vec<Box<dyn AgentTool>>,
    security_profile: SecurityProfile,
    /// Runtime permission mode. Independent of `security_profile` (which is
    /// the compile-time path/call-count floor); see `PermissionMode`.
    /// Defaults to `PermissionMode::Default`, matching the previous
    /// hardcoded `ToolGuardian::with_default_mode` behavior — set via
    /// `.permission_mode()` or the `.permission_preset()` convenience.
    permission_mode: PermissionMode,
    /// If set, limits sub-agent spawning depth.
    subagent_depth: u32,
    /// Deprecated: skip context assembly. All callers should use unified context.
    /// Kept for backward compatibility; will be removed in a future release.
    skip_context: bool,
    /// If true, load recent thread from _threads/ for cross-session continuity
    /// (default: true).
    enable_thread_continuity: bool,
    /// Which interface this session serves; drives cross-interface thread
    /// merging and the "speaking via X" identity line.
    identity: Option<hq_core::identity::RequestIdentity>,
    /// Skip this interface's own thread file in the merge. Set when the
    /// caller injects its own conversation history (the relay does) so the
    /// same turns are not duplicated.
    exclude_own_interface_thread: bool,
    /// Tool visibility profile for this session.
    /// Applied to the tool list in build() — wired in Task 9 (easy-action-tools plan).
    session_profile: SessionProfile,
    /// Tools whose name starts with one of these are left out of the session.
    tool_deny_prefixes: Vec<String>,
    /// Whether this session works or orchestrates; see [`SessionRole`].
    role: SessionRole,
    /// Originating chat turn id (e.g. a `background_turns` row) copied into the
    /// `ChildExecContext` so non-blocking child completions can be routed back.
    child_parent_turn_id: Option<String>,
    /// Completion callback for non-blocking child plans. Surfaced by callers
    /// (relay native paths) that want async child results pushed to the chat.
    child_completion_sink: Option<crate::agents::CompletionSink>,
    /// Progress callback for the `report_progress` tool. Surfaced by callers
    /// (relay native paths) that want volunteered mid-turn notes pushed to the
    /// chat. `None` = the tool reports "not available" and never panics.
    child_progress_sink: Option<crate::native_hq::ProgressSink>,
}

impl SessionBuilder {
    /// Create a builder from an `HqConfig`.
    pub fn from_config(config: &HqConfig) -> Self {
        Self {
            config: config.clone(),
            system_prompt: None,
            harness_instructions: None,
            event_tx: None,
            extra_allowed_paths: Vec::new(),
            working_dir: None,
            provider_override: None,
            session_config_override: None,
            extra_tools: Vec::new(),
            security_profile: SecurityProfile::Guarded,
            permission_mode: PermissionMode::Default,
            subagent_depth: 0,
            enable_thread_continuity: true,
            skip_context: false,
            identity: None,
            exclude_own_interface_thread: false,
            session_profile: SessionProfile::Standard,
            tool_deny_prefixes: Vec::new(),
            role: SessionRole::default(),
            child_parent_turn_id: None,
            child_completion_sink: None,
            child_progress_sink: None,
        }
    }

    /// Set the resolved request identity for this session.
    ///
    /// Enables cross-interface conversation continuity: the merged thread
    /// loader prefixes messages from other interfaces, and the soul gains a
    /// line stating which interface the current conversation is on.
    pub fn with_identity(mut self, identity: hq_core::identity::RequestIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Skip this interface's own thread file when merging cross-interface
    /// context. Use when the caller injects its own history into the session.
    pub fn exclude_own_interface_thread(mut self) -> Self {
        self.exclude_own_interface_thread = true;
        self
    }

    /// Wire async child-completion delivery for non-blocking `spawn_subagents`
    /// plans: the originating chat turn id plus the sink that receives one
    /// [`ChildCompletionEvent`](crate::agents::ChildCompletionEvent) per child.
    pub fn child_completion(
        mut self,
        parent_turn_id: Option<String>,
        sink: crate::agents::CompletionSink,
    ) -> Self {
        self.child_parent_turn_id = parent_turn_id;
        self.child_completion_sink = Some(sink);
        self
    }

    /// Record the turn id children spawned in this session report as their
    /// parent, with or without a completion sink. A follow-up turn relies on
    /// it so the children it spawns inherit the follow-up depth.
    pub fn child_turn(mut self, parent_turn_id: Option<String>) -> Self {
        if parent_turn_id.is_some() {
            self.child_parent_turn_id = parent_turn_id;
        }
        self
    }

    /// Wire volunteered mid-turn progress delivery: the sink the
    /// `report_progress` tool fires into. The turn id rides on
    /// `child_parent_turn_id` (set via [`SessionBuilder::child_completion`] or
    /// repeated here when no completion sink is needed).
    pub fn child_progress(
        mut self,
        parent_turn_id: Option<String>,
        sink: crate::native_hq::ProgressSink,
    ) -> Self {
        if parent_turn_id.is_some() {
            self.child_parent_turn_id = parent_turn_id;
        }
        self.child_progress_sink = Some(sink);
        self
    }

    /// Set the system prompt (overrides context-assembled prompt).
    pub fn system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = Some(prompt);
        self
    }

    /// Subscribe to session events via a broadcast channel.
    pub fn event_channel(mut self, tx: broadcast::Sender<SessionEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Add extra paths the agent is allowed to access.
    pub fn allow_path(mut self, path: PathBuf) -> Self {
        self.extra_allowed_paths.push(path);
        self
    }

    /// Set the working directory for file operations.
    pub fn working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Override the LLM provider (instead of constructing from config).
    pub fn provider(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.provider_override = Some(provider);
        self
    }

    /// Override the session config.
    pub fn session_config(mut self, config: SessionConfig) -> Self {
        self.session_config_override = Some(config);
        self
    }

    /// Register additional tools beyond the defaults.
    #[cfg(test)]
    pub fn add_tool(mut self, tool: Box<dyn AgentTool>) -> Self {
        self.extra_tools.push(tool);
        self
    }

    /// Set the security profile for governance.
    pub fn security_profile(mut self, profile: SecurityProfile) -> Self {
        self.security_profile = profile;
        self
    }

    /// Set the runtime permission mode for governance (independent of
    /// `security_profile`). See `PermissionMode`.
    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = mode;
        self
    }

    /// Set both `security_profile` and `permission_mode` from a single named
    /// [`PermissionPreset`] — the CLI/chat-command-facing convenience so a
    /// caller picks one name instead of two raw flags.
    pub fn permission_preset(mut self, preset: PermissionPreset) -> Self {
        let (profile, mode) = preset.profile_and_mode();
        self.security_profile = profile;
        self.permission_mode = mode;
        self
    }

    /// Set the sub-agent depth (for nested agent spawning).
    pub fn subagent_depth(mut self, depth: u32) -> Self {
        self.subagent_depth = depth;
        self
    }

    /// Skip context assembly (use for lightweight relay chat sessions).
    pub fn skip_context(mut self) -> Self {
        self.skip_context = true;
        self
    }

    /// Set harness-specific instructions that are appended after the soul.
    ///
    /// Unlike `system_prompt()` which replaces the entire system prompt,
    /// this appends context-specific instructions (relay rules, coding
    /// guidelines, etc.) after the soul. The soul is always loaded from
    /// `_system/SOUL.md` by the context engine.
    ///
    /// Callers that previously built their own `soul + RELAY_INSTRUCTIONS`
    /// string should switch to this method with only the instructions part.
    pub fn harness_instructions(mut self, instructions: String) -> Self {
        self.harness_instructions = Some(instructions);
        self
    }

    /// Disable cross-session thread continuity.
    pub fn no_thread_continuity(mut self) -> Self {
        self.enable_thread_continuity = false;
        self
    }

    /// Run this session as an implementor (the default) or an orchestrator.
    pub fn role(mut self, role: SessionRole) -> Self {
        self.role = role;
        self
    }

    pub(crate) fn session_role(&self) -> SessionRole {
        self.role
    }

    /// Set the session profile controlling which tool tier is visible.
    ///
    /// Use `Weak` for relay sessions and sub-agents running on local/free models.
    /// Leave out every tool whose name starts with one of `prefixes`, whatever
    /// the profile or policy would otherwise allow.
    pub fn deny_tool_prefixes(mut self, prefixes: Vec<String>) -> Self {
        self.tool_deny_prefixes = prefixes;
        self
    }

    pub fn session_profile(mut self, profile: SessionProfile) -> Self {
        self.session_profile = profile;
        self
    }

    /// Set the session execution mode (Normal, Plan, or Execute).
    pub fn mode(mut self, mode: SessionMode) -> Self {
        self.session_config_override
            .get_or_insert_with(SessionConfig::default)
            .mode = mode;
        self
    }

    /// Build the `AgentSession`.
    pub async fn build(mut self) -> Result<AgentSession> {
        // Opened first so the outcome sink is attached before any LLM call.
        // A missing DB is non-fatal: the session runs without telemetry.
        let vault_path = self.config.vault_path.clone();
        let shared_db = hq_db::Database::open(&vault_path.join("_data/vault.db"))
            .ok()
            .map(Arc::new);
        let backends = provider::resolve_backends(
            &self.config,
            self.provider_override.take(),
            shared_db.as_ref(),
        )?;
        let mut session_config =
            provider::resolve_session_config(&self.config, self.session_config_override.take())
                .await;
        // Sub-agents report to a parent whose own review covers the work.
        if self.subagent_depth > 0 {
            session_config.background_review = false;
        }
        let allowed_paths = self.allowed_paths(&vault_path);
        let mut tools = self.native_tools(&vault_path, shared_db.clone());
        // One taint flag for this session and every sub-agent it spawns.
        let taint = crate::governance::TaintTracker::new();
        tools.extend(self.subagent_tools(
            &backends.provider,
            &vault_path,
            &allowed_paths,
            &session_config,
            shared_db.clone(),
            &taint,
        ));
        // Minted here rather than in `AgentSession::new` so `load_skill` logs
        // under the id the post-session skill review later reads back.
        let session_id = uuid::Uuid::new_v4().to_string();
        tools.extend(self.skill_tools(&vault_path, shared_db.clone(), &session_id));
        tools.extend(std::mem::take(&mut self.extra_tools));
        let (governed_registry, preset) =
            self.govern_tools(tools, &vault_path, &allowed_paths, &session_config, taint);
        let tool_count = governed_registry.len();

        // Derived here, before the registry moves into `AgentSession::new`.
        // The post-governance `AgentTool` view is the only one that reflects
        // the profile filter, the preset filter, and governance denies — i.e.
        // the tools the model can actually call. Deriving from the `HqTool`
        // registry instead would advertise tools this session does not have.
        let tool_notes = {
            let block = governed_registry.behavioral_block(MAX_BEHAVIORAL_NOTE_CHARS);
            (!block.is_empty()).then_some(block)
        };
        let weak_catalog = matches!(self.session_profile, SessionProfile::Weak)
            .then(|| governed_registry.catalog_block_by_category())
            .filter(|b| !b.is_empty());

        info!(
            tools = tool_count,
            model = %session_config.model,
            "SessionBuilder: built session"
        );

        let mut session = AgentSession::new(
            backends.provider.clone(),
            governed_registry,
            session_config.clone(),
        );
        session.session_id = session_id;
        if self.role == SessionRole::Orchestrator {
            session.set_role_denial(Arc::new(crate::session::role_denial::RoleDenial::new(
                ORCHESTRATOR_REMOVED_TOOLS,
                mailbox_denial_notifier(vault_path.clone()),
                vec![vault_path.join("Notebooks"), std::env::temp_dir()],
            )));
        }

        // Explicit root backend: when the versioned `backends` chain is
        // configured (see `provider::resolve_backends`) *and no `.provider(...)` override was
        // given*, drive turns through the ProviderChain (primary + ordered
        // fallbacks). An explicit provider override takes precedence: turns run
        // through the single `ApiBackend` adapter `AgentSession::new` created over
        // that provider (which also serves utility calls), matching the
        // documented contract that `.provider(...)` overrides turn execution.
        // Otherwise the legacy provider is adapted into a deterministic
        // single-backend chain by `AgentSession::new`, so behavior is unchanged
        // for existing configs. Selection is explicit and fixed for the session.
        if !backends.has_override
            && let Some(chain) = backends.chain
        {
            session.set_backend(chain);
        }

        if let Some(ref db) = shared_db {
            session.set_telemetry_db(db.clone());
        }
        session.set_vault_path(vault_path.clone());

        // Wire the broadcast channel.
        if let Some(tx) = self.event_tx.take() {
            session.on_event(move |event| {
                // Best-effort: ignore send errors (no receivers)
                let _ = tx.send(event);
            });
        }

        self.apply_system_prompt(
            &mut session,
            prompt::PromptInputs {
                vault_path: &vault_path,
                shared_db,
                preset,
                tool_count,
                tool_notes: tool_notes.as_deref(),
                weak_catalog: weak_catalog.as_deref(),
                context_window: session_config.context_window,
                identity: hq_core::config::runtime_identity_block(&self.config),
            },
        )
        .await;

        Ok(session)
    }
}

#[cfg(test)]
impl SessionBuilder {
    /// Test-only accessors: no production code needs to read these back off
    /// the builder (they only matter once threaded into `ToolGuardian` at
    /// `build()` time), so keep the getters test-scoped rather than adding
    /// public API surface.
    fn test_security_profile(&self) -> &SecurityProfile {
        &self.security_profile
    }

    fn test_permission_mode(&self) -> &PermissionMode {
        &self.permission_mode
    }
}

#[cfg(test)]
mod compat_tests;
#[cfg(test)]
mod tests;
