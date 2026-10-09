//! Agent session — drives the prompt-tool-call loop.
//!
//! Manages a conversation with an LLM, dispatching tool calls and handling
//! compaction when the context window fills up.

pub mod context;
mod credits;
pub mod r#loop;
pub mod routing;
pub(crate) mod role_denial;
mod skills;
mod stream;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use hq_core::types::{ChatMessage, EnvelopeKind, EventSource, SessionEventEnvelope};
use hq_llm::provider::LlmProvider;

use crate::backend::{ApiBackend, SessionBackend};
use crate::governance::GovernedRegistry;

pub use context::estimate_token_count;

/// Session execution mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SessionMode {
    /// Standard interactive session.
    #[default]
    Normal,
    /// Plan mode: explore → plan → await approval. No writes allowed.
    Plan,
    /// Execute mode: follow a plan step-by-step with verification.
    Execute {
        /// Path to the plan file being executed.
        plan_path: String,
    },
}

/// Configuration for an agent session.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub model: String,
    pub context_window: usize,
    pub compaction_threshold: f64,
    pub max_retries: u32,
    pub retry_base_delay: Duration,
    pub preemptive_threshold: f64,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub fallback_model: Option<String>,
    pub self_healing: SelfHealingConfig,
    pub max_budget_usd: Option<f64>,
    /// Wall-clock limit in seconds. None = unlimited. Default = 5 hours.
    pub max_duration_secs: Option<u64>,
    pub mode: SessionMode,
    pub agent_name: String,
    /// Whether to run the post-session background review hook.
    pub background_review: bool,
    /// Whether a live user turn (chat, a just-received relay/Telegram/Discord
    /// message, CLI) is directly driving this session, as opposed to an
    /// autonomous/unattended trigger (daemon periodic task, mission engine
    /// step executor, or any future background job). Defaults to `false` —
    /// fail-safe: a session that doesn't explicitly claim liveness is
    /// treated as unattended. `ToolGuardian::build_registry` reads this
    /// (via `LiveUserTurn::from_session_config`) to hard-exclude any tool
    /// whose `requires_live_user_turn()` is `true`; `AgentSession::execute_tool`
    /// re-checks it as a backstop.
    pub is_live_user_turn: bool,
    /// Emit `StepCredits` after each step served by Copilot (web chat only, since it costs one HTTP call per step).
    pub copilot_step_credits: bool,
}

/// Configuration for self-healing execution (AutoResearchClaw-inspired).
///
/// When enabled, failed or low-quality job outputs trigger a diagnosis step
/// followed by a repair attempt, up to `max_attempts` times.
#[derive(Debug, Clone)]
pub struct SelfHealingConfig {
    pub enabled: bool,
    pub max_attempts: u32,
    pub error_markers: Vec<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            model: "relay".to_string(),
            context_window: 200_000,
            compaction_threshold: 0.75,
            preemptive_threshold: 0.50,
            max_retries: 6,
            retry_base_delay: Duration::from_secs(2),
            temperature: None,
            max_tokens: None,
            fallback_model: None,
            self_healing: SelfHealingConfig::default(),
            max_budget_usd: None,
            max_duration_secs: Some(18_000),
            mode: SessionMode::default(),
            agent_name: "hq".to_string(),
            background_review: true,
            is_live_user_turn: false,
            copilot_step_credits: false,
        }
    }
}

impl Default for SelfHealingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 3,
            error_markers: vec![
                "Error:".to_string(),
                "FAILED".to_string(),
                "panic".to_string(),
                "stack overflow".to_string(),
                "out of memory".to_string(),
            ],
        }
    }
}

/// A pre-computed summary ready for instant compaction.
pub(super) struct PrecomputedSummary {
    pub text: String,
    /// Number of messages from the front that this summary covers.
    pub covers_up_to: usize,
}

/// Helper for accumulating streaming tool call deltas.
#[derive(Default)]
pub(super) struct ToolCallBuilder {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Accumulated stats for a session.
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    /// Tokens served from the provider's prompt cache (cheaper than fresh input).
    pub total_cache_read_tokens: u64,
    /// Tokens written into the prompt cache this session.
    pub total_cache_write_tokens: u64,
    /// Fraction of input tokens served from cache (0.0 to 1.0).
    pub cache_hit_ratio: f64,
    pub total_cost: f64,
    pub tool_call_count: u32,
    pub message_count: u32,
}

/// Called once per completed prompt with the user message and the final
/// reply, as `(role, content)` pairs. Used for memory extraction.
pub type PostTurnCallback = Arc<dyn Fn(Vec<(String, String)>) + Send + Sync>;

/// A subscriber over the correlated [`SessionEventEnvelope`] stream.
pub type EnvelopeSubscriber = Box<dyn Fn(SessionEventEnvelope) + Send + Sync>;

/// The agent session — drives the prompt-tool-call loop.
pub struct AgentSession {
    pub(super) messages: Vec<ChatMessage>,
    pub(super) message_tokens: Vec<usize>,
    pub(super) tools: Arc<Mutex<GovernedRegistry>>,
    /// Utility LLM provider used for internal (non-turn) calls: compaction
    /// summaries and preemptive summarization. Turns are driven by [`backend`].
    pub(super) provider: Arc<dyn LlmProvider>,
    /// The explicit root execution backend for turns. Defaults to an
    /// [`ApiBackend`] adapter over [`provider`]; [`SessionBuilder`] installs a
    /// [`ProviderChain`](crate::backend::ProviderChain) when a chain is configured.
    /// Selection is explicit and fixed for the session — never adaptive per turn.
    pub(super) backend: Arc<dyn SessionBackend>,
    pub(super) config: SessionConfig,
    pub(super) system_prompt: Option<String>,
    pub(super) system_prompt_tokens: usize,
    /// Tool schemas sent with the last request; they fill the window too.
    pub(super) tool_schema_tokens: usize,
    pub(super) subscribers: Vec<Box<dyn Fn(hq_core::types::SessionEvent) + Send + Sync>>,
    /// Envelope subscribers receive every event wrapped with run correlation.
    /// The legacy `subscribers` list above is kept working as an adapter.
    pub(super) envelope_subscribers: Vec<EnvelopeSubscriber>,
    /// Monotonic per-session event sequence for envelope ordering.
    pub(super) event_seq: AtomicU64,
    /// Identifier of the currently-executing run (one per `prompt`/`prompt_stream`).
    pub(super) run_id: String,
    /// When set, [`begin_run`](Self::begin_run) pins this exact run id instead of
    /// minting a fresh UUID. The agent service uses this so a child session's
    /// emitted envelopes carry the same `run_id` it announced in its
    /// `ChildStarted` marker, keeping the correlated stream joinable.
    pub(super) pinned_run_id: Option<String>,
    /// Parent run id, set when this session is a child of another run. Reserved
    /// for the agent service; unused by the standalone session engine today.
    pub(super) parent_run_id: Option<String>,
    pub(super) total_input_tokens: u64,
    pub(super) total_output_tokens: u64,
    pub(super) total_cache_read_tokens: u64,
    pub(super) total_cache_write_tokens: u64,
    pub(super) total_cost: f64,
    pub(super) tool_call_count: u32,
    pub(super) tool_count: usize,
    pub(super) precomputed: Arc<Mutex<Option<PrecomputedSummary>>>,
    pub(super) preemptive_handle: Option<tokio::task::JoinHandle<()>>,
    pub(super) post_turn_callback: Option<PostTurnCallback>,
    pub last_resolved_model: Option<String>,
    pub session_id: String,
    pub(super) telemetry_db: Option<Arc<hq_db::Database>>,
    /// Set for an orchestrator: explains and records calls outside its role.
    pub(super) role_denial: Option<Arc<role_denial::RoleDenial>>,
    /// Skill hint index, used to auto-load matching skills on the first user
    /// turn. Enrichment cannot happen at build time because it matches against
    /// the user's instruction, which does not exist yet.
    pub(super) skill_index: Option<Arc<hq_tools::skills::SkillHintIndex>>,
    /// Latches after the first turn enriches. Re-running per turn would
    /// rewrite the system prompt every turn and destroy the cached prefix.
    pub(super) skills_enriched: bool,
    /// Token ceiling for auto-loaded skill content.
    pub(super) max_skill_tokens: usize,
    /// `tool_call_count` at the last skill review or `skill_manage` call.
    pub(super) skill_review_mark: u32,
    /// Skills loaded or already suggested this session, so each is nudged at most once.
    pub(super) skills_in_play: std::collections::HashSet<String>,
    pub(super) vault_path: Option<std::path::PathBuf>,
    /// Wall-clock start time; set when the session is created.
    pub(crate) start_time: Instant,
    /// Set to true by relay !cancel command; loop checks this each turn.
    pub cancel: Arc<AtomicBool>,
    /// Text dropped in by a relay surface while this session has a turn in
    /// flight (a message that arrived mid-turn); drained at the same
    /// checkpoints as `cancel` and injected via `steer()`.
    pub pending_steer: Arc<std::sync::Mutex<Option<String>>>,
    /// Paths of files modified this session; used in cancel summary.
    pub(crate) files_touched: Vec<String>,
    step_credits: Option<credits::CreditTracker>,
}

impl AgentSession {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        tools: GovernedRegistry,
        config: SessionConfig,
    ) -> Self {
        let tool_count = tools.len();
        let config_step_credits = config
            .copilot_step_credits
            .then(credits::CreditTracker::live);
        // Default root backend: adapt the provider through the same
        // `SessionBackend` contract the configured chain uses, so every turn
        // flows through one engine. Explicit and fixed for the session.
        let backend: Arc<dyn SessionBackend> = Arc::new(ApiBackend::new(
            provider.name().to_string(),
            provider.clone(),
        ));
        Self {
            messages: Vec::new(),
            message_tokens: Vec::new(),
            tools: Arc::new(Mutex::new(tools)),
            provider,
            backend,
            config,
            system_prompt: None,
            system_prompt_tokens: 0,
            tool_schema_tokens: 0,
            subscribers: Vec::new(),
            envelope_subscribers: Vec::new(),
            event_seq: AtomicU64::new(0),
            run_id: uuid::Uuid::new_v4().to_string(),
            pinned_run_id: None,
            parent_run_id: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_write_tokens: 0,
            total_cost: 0.0,
            tool_call_count: 0,
            tool_count,
            precomputed: Arc::new(Mutex::new(None)),
            preemptive_handle: None,
            post_turn_callback: None,
            last_resolved_model: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            telemetry_db: None,
            role_denial: None,
            skill_index: None,
            skills_enriched: false,
            max_skill_tokens: 3000,
            skill_review_mark: 0,
            skills_in_play: std::collections::HashSet::new(),
            vault_path: None,
            start_time: Instant::now(),
            cancel: Arc::new(AtomicBool::new(false)),
            pending_steer: Arc::new(std::sync::Mutex::new(None)),
            files_touched: Vec::new(),
            step_credits: config_step_credits,
        }
    }

    /// Install an explicit root execution backend (e.g. a configured
    /// [`ProviderChain`](crate::backend::ProviderChain)) for driving turns.
    ///
    /// Replaces the default [`ApiBackend`] adapter created in [`new`](Self::new).
    /// The utility [`provider`](Self::provider) handle is left untouched — it
    /// still serves compaction/summarization calls.
    pub fn set_backend(&mut self, backend: Arc<dyn SessionBackend>) {
        self.backend = backend;
    }

    /// The root execution backend's name (for diagnostics).
    pub fn backend_name(&self) -> &str {
        self.backend.name()
    }

    /// Install the skill index so the first user turn can auto-load matching
    /// skills.
    pub fn set_skill_index(
        &mut self,
        index: Arc<hq_tools::skills::SkillHintIndex>,
        max_skill_tokens: usize,
    ) {
        self.skill_index = Some(index);
        self.max_skill_tokens = max_skill_tokens;
    }

    pub(crate) fn set_role_denial(&mut self, denial: Arc<role_denial::RoleDenial>) {
        self.role_denial = Some(denial);
    }

    pub fn set_telemetry_db(&mut self, db: Arc<hq_db::Database>) {
        self.telemetry_db = Some(db);
    }

    pub fn set_vault_path(&mut self, path: std::path::PathBuf) {
        self.vault_path = Some(path);
    }

    /// Build a `SessionContext` snapshot for scoping provider calls.
    pub(super) fn session_context(&self) -> hq_llm::SessionContext {
        hq_llm::SessionContext {
            session_id: self.session_id.clone(),
            turn_idx: self.tool_call_count as i64,
        }
    }

    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    pub fn tool_count(&self) -> usize {
        self.tool_count
    }

    /// Names of every registered tool, sorted.
    ///
    /// `tool_count` counts deferred tools too, so it cannot tell you whether a
    /// specific capability survived the profile and policy filters.
    #[cfg(test)]
    pub async fn tool_names(&self) -> Vec<String> {
        let registry = self.tools.lock().await;
        let mut names: Vec<String> = registry.names().map(str::to_string).collect();
        names.sort_unstable();
        names
    }

    /// Deferred tools as `(name, hint)`, i.e. those whose schemas are loaded
    /// on demand via `tool_search` instead of shipping every turn.
    #[cfg(test)]
    pub async fn deferred_tool_catalog(&self) -> Vec<(String, String)> {
        self.tools.lock().await.deferred_catalog()
    }

    /// Definitions actually sent to the model each turn (deferred excluded).
    #[cfg(test)]
    pub async fn active_tool_definitions(&self) -> Vec<hq_core::types::ToolDefinition> {
        self.tools.lock().await.active_definitions()
    }

    pub fn model(&self) -> &str {
        &self.config.model
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.config.model = model.into();
    }

    pub fn total_input_tokens(&self) -> u64 {
        self.total_input_tokens
    }

    pub fn total_output_tokens(&self) -> u64 {
        self.total_output_tokens
    }

    pub fn total_cost(&self) -> f64 {
        self.total_cost
    }

    pub fn context_window(&self) -> usize {
        self.config.context_window
    }

    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    pub fn set_post_turn_callback(&mut self, callback: PostTurnCallback) {
        self.post_turn_callback = Some(callback);
    }

    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt_tokens = estimate_token_count(&prompt);
        self.system_prompt = Some(prompt);
    }

    /// Push a message and cache its token estimate in lockstep.
    pub fn push_message(&mut self, msg: ChatMessage) {
        let tokens = message_token_estimate(&msg);
        self.messages.push(msg);
        self.message_tokens.push(tokens);
    }

    /// Insert a message at an arbitrary index and cache its token estimate.
    pub(super) fn insert_message(&mut self, index: usize, msg: ChatMessage) {
        let tokens = message_token_estimate(&msg);
        self.messages.insert(index, msg);
        self.message_tokens.insert(index, tokens);
    }

    pub fn on_event(
        &mut self,
        callback: impl Fn(hq_core::types::SessionEvent) + Send + Sync + 'static,
    ) {
        self.subscribers.push(Box::new(callback));
    }

    /// Subscribe to the correlated [`SessionEventEnvelope`] stream.
    ///
    /// Every event the session emits reaches both legacy `on_event` subscribers
    /// (unwrapped) and envelope subscribers (wrapped with run id, sequence, and
    /// source). Run/child lifecycle envelopes (`RunStarted`/`RunFinished`) are
    /// delivered here only.
    pub fn on_envelope(&mut self, callback: impl Fn(SessionEventEnvelope) + Send + Sync + 'static) {
        self.envelope_subscribers.push(Box::new(callback));
    }

    /// Set this session's parent run id (marks it a child run). Reserved for the
    /// agent service; the standalone engine does not consult it.
    pub fn set_parent_run_id(&mut self, parent_run_id: Option<String>) {
        self.parent_run_id = parent_run_id;
    }

    /// Pin the run id [`begin_run`](Self::begin_run) will use for the next run,
    /// instead of minting a fresh UUID. The agent service sets this so a child
    /// session's forwarded envelopes share the `run_id` it announced in its
    /// `ChildStarted` marker.
    pub fn set_run_id(&mut self, run_id: impl Into<String>) {
        let id = run_id.into();
        self.pinned_run_id = Some(id.clone());
        self.run_id = id;
    }

    /// The current run's id.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Inject a steering message (appears as a system-like user message).
    pub fn steer(&mut self, message: &str) {
        self.push_message(ChatMessage {
            image_parts: Vec::new(),
            role: hq_core::types::MessageRole::User,
            content: format!("[STEERING] {}", message),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        });
    }

    /// Next envelope sequence number (monotonic per session).
    fn next_seq(&self) -> u64 {
        self.event_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    pub fn emit(&self, event: hq_core::types::SessionEvent) {
        // Session-generated lifecycle/accounting events originate from the engine.
        self.emit_from(EventSource::Session, event);
    }

    /// Emit a [`SessionEvent`] tagged with an explicit [`EventSource`].
    ///
    /// Backend-origin events (streamed model text/reasoning, backend progress) use
    /// [`EventSource::Backend`] with the *selected* backend's identity so a chain
    /// fallback is observable on the envelope stream; session-generated
    /// lifecycle/accounting events keep [`EventSource::Session`] via [`emit`](Self::emit).
    /// Legacy `on_event` subscribers see the unwrapped event either way.
    pub(super) fn emit_from(&self, source: EventSource, event: hq_core::types::SessionEvent) {
        for subscriber in &self.subscribers {
            subscriber(event.clone());
        }
        if !self.envelope_subscribers.is_empty() {
            let envelope = SessionEventEnvelope::session(
                self.run_id.clone(),
                self.parent_run_id.clone(),
                self.next_seq(),
                source,
                event,
            );
            for subscriber in &self.envelope_subscribers {
                subscriber(envelope.clone());
            }
        }
    }

    /// Emit a lifecycle envelope (run/child markers) to envelope subscribers.
    /// Legacy `on_event` subscribers do not see these (they carry no
    /// [`SessionEvent`]).
    pub(super) fn emit_lifecycle(&self, source: EventSource, kind: EnvelopeKind) {
        if self.envelope_subscribers.is_empty() {
            return;
        }
        let envelope = SessionEventEnvelope::lifecycle(
            self.run_id.clone(),
            self.parent_run_id.clone(),
            self.next_seq(),
            source,
            kind,
        );
        for subscriber in &self.envelope_subscribers {
            subscriber(envelope.clone());
        }
    }

    /// Begin a new correlated run: assign a fresh run id and announce it.
    pub(super) fn begin_run(&mut self) {
        // A pinned run id (set by the agent service for child sessions) keeps the
        // child's emitted envelopes joinable with the `ChildStarted` marker; only
        // mint a fresh UUID when no id was pinned.
        self.run_id = self
            .pinned_run_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.emit_lifecycle(
            EventSource::Backend(self.backend.name().to_string()),
            EnvelopeKind::RunStarted {
                model: self.config.model.clone(),
            },
        );
    }

    /// Announce the terminal outcome of the current run.
    pub(super) fn end_run(&self, result: &anyhow::Result<hq_core::types::SessionResult>) {
        let outcome = match result {
            Ok(r) => r.outcome_label().to_string(),
            Err(_) => "error".to_string(),
        };
        self.emit_lifecycle(EventSource::Session, EnvelopeKind::RunFinished { outcome });
    }

    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    pub fn stats(&self) -> SessionStats {
        SessionStats {
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            total_tokens: self.total_input_tokens + self.total_output_tokens,
            total_cache_read_tokens: self.total_cache_read_tokens,
            total_cache_write_tokens: self.total_cache_write_tokens,
            cache_hit_ratio: if self.total_input_tokens > 0 {
                self.total_cache_read_tokens as f64 / self.total_input_tokens as f64
            } else {
                0.0
            },
            total_cost: self.total_cost,
            tool_call_count: self.tool_call_count,
            message_count: self.messages.len() as u32,
        }
    }

    /// Returns a clone of the cancel flag handle so the relay can set it.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Returns a clone of the steer inbox handle so the relay can drop a
    /// mid-turn redirect message into it.
    pub fn steer_handle(&self) -> Arc<std::sync::Mutex<Option<String>>> {
        Arc::clone(&self.pending_steer)
    }

    /// Builds a human-readable summary of what the session accomplished before cancellation.
    pub fn build_cancel_summary(&self) -> String {
        let elapsed = self.start_time.elapsed();
        let secs = elapsed.as_secs();
        let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
        let time_str = if h > 0 {
            format!("{h}h {m}m {s}s")
        } else {
            format!("{m}m {s}s")
        };

        let files = if self.files_touched.is_empty() {
            "none".to_string()
        } else {
            self.files_touched.join(", ")
        };

        format!(
            "Cancelled after {time_str} / {} tool calls.\nFiles modified: {files}",
            self.tool_call_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_config_default_has_no_turn_field_and_a_five_hour_wall_clock() {
        // FR-056: `SessionConfig` has no `max_turns` (or equivalent) member —
        // there is no application-level turn-count ceiling to set. The
        // wall-clock and budget caps are distinct, legitimate controls and
        // stay.
        let cfg = SessionConfig::default();
        assert_eq!(cfg.max_duration_secs, Some(18_000));
        assert_eq!(cfg.max_budget_usd, None);
    }
}

/// Content plus tool-call names and arguments, which the model reads back each turn.
pub(super) fn message_token_estimate(msg: &ChatMessage) -> usize {
    let calls: usize = msg
        .tool_calls
        .iter()
        .map(|c| estimate_token_count(&c.name) + estimate_token_count(&c.arguments.to_string()))
        .sum();
    estimate_token_count(&msg.content) + calls
}

#[cfg(test)]
mod token_estimate_tests {
    use super::*;
    use hq_core::types::{MessageRole, ToolCall};

    #[test]
    fn tool_call_arguments_count_toward_the_window() {
        let call = ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({ "command": "x".repeat(300) }),
        };
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            tool_calls: vec![call],
            tool_call_id: None,
            reasoning_content: None,
            image_parts: Vec::new(),
        };
        assert!(message_token_estimate(&msg) > estimate_token_count(&msg.content) + 90);
    }
}
