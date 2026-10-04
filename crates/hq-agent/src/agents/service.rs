//! [`AgentService`] — the single child-execution runtime.
//!
//! Consolidates what used to be four separate surfaces (`SpawnSubagentTool`,
//! `CoordinatorSession`, the fleet dispatch table, and the harness router) into
//! one governed API with four execution modes. A parent hands it a
//! [`ChildPlan`]; it resolves governance + backend selection once per child,
//! runs the children (single / parallel / race / graph), streams correlated
//! envelopes back to the parent, and returns structured [`ChildOutcome`]s for
//! the parent to synthesize. It never synthesizes a final answer itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hq_core::types::{
    EnvelopeKind, EventSource, PermissionMode, SecurityProfile, SessionEventEnvelope,
};
use hq_db::Database;
use hq_llm::LlmProvider;

use crate::backend::{BackendRegistry, SessionBackend};
use crate::governance::ToolGuardian;
use crate::session::{AgentSession, SessionConfig};

use super::child_context;
use super::ledger::{Ledger, REPORTING_PROTOCOL};
use super::model_select::{ModelPlan, resolve_child_model};
use super::types::{
    ChildExecContext, ChildMode, ChildOutcome, ChildPlan, ChildRequest, ChildStatus, EnvelopeSink,
    resolve_timeout,
};

/// The in-process executor label (mirrors the historical "hq" harness name).
pub const INPROCESS_BACKEND: &str = "hq";

/// The unified child-execution runtime.
///
/// Cheap to clone (every field is `Arc`/`Copy`/small), so it can be moved into
/// spawned per-child tasks and re-derived at depth+1 for recursion.
#[derive(Clone)]
pub struct AgentService {
    /// Utility/turn provider for in-process children (the HQ LLM router).
    provider: Arc<dyn LlmProvider>,
    /// Registry of configured external backends. `None` = in-process only.
    backend_registry: Option<Arc<BackendRegistry>>,
    /// Vault root (soul summary, telemetry db location).
    vault_path: PathBuf,
    /// The service's own path allowlist. Child overrides must stay within it.
    allowed_paths: Vec<PathBuf>,
    /// Base session config cloned for each child.
    session_config: SessionConfig,
    /// Security profile applied to every child's governed registry.
    security_profile: SecurityProfile,
    /// Permission mode applied to every child, so a parent that may only read
    /// (DontAsk) cannot hand its work to a child that may write.
    permission_mode: PermissionMode,
    /// This service's recursion depth. Children run at `depth + 1`.
    depth: u32,
    /// Maximum recursion depth. Spawning is allowed only while `depth < max_depth`.
    max_depth: u32,
    /// Default per-child wall-clock timeout.
    default_timeout: Duration,
    /// Optional telemetry/synergy database.
    db: Option<Arc<Database>>,
    /// `governance.bash` for children's bash tool, so a child keeps the
    /// operator's env passthrough (e.g. `GH_TOKEN`) and sandbox mode.
    bash_settings: crate::bash_sandbox::BashSettings,
    /// The parent session's taint, shared so delegation cannot launder it.
    taint: crate::governance::TaintTracker,
}

/// How a child's backend resolved after governance + policy.
enum ResolvedExecutor {
    /// Run in-process via a fresh [`AgentSession`] with the full HQ tool set.
    InProcess,
    /// Drive turns through an external [`SessionBackend`] (no local tools).
    External(Arc<dyn SessionBackend>),
}

impl std::fmt::Debug for ResolvedExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InProcess => write!(f, "InProcess"),
            Self::External(b) => write!(f, "External({})", b.name()),
        }
    }
}

/// Attach the effective model and scrub anything credential-shaped from the
/// text a provider error or child output may have carried back.
fn finish_outcome(mut outcome: ChildOutcome, model: &ModelPlan) -> ChildOutcome {
    outcome.effective_model = Some(model.effective.clone());
    outcome.output = hq_core::redact::redact_secrets(&outcome.output);
    outcome.error = outcome.error.map(|e| hq_core::redact::redact_secrets(&e));
    outcome
}

/// The outcome of governance + backend selection for one child.
#[derive(Debug)]
struct Resolution {
    executor: ResolvedExecutor,
    resolved_backend: String,
    fallback_used: bool,
    allowed_paths: Vec<PathBuf>,
    /// The child's model selection, filled by [`AgentService::authorize_and_resolve`].
    model: ModelPlan,
}

/// Resolved per-execute correlation context (never-empty parent id + sink).
#[derive(Clone)]
pub(super) struct RunCtx {
    parent_run_id: String,
    sink: Option<EnvelopeSink>,
    parent_messages: Option<Vec<hq_core::types::ChatMessage>>,
    seq: Arc<AtomicU64>,
    ledger: Arc<Ledger>,
}

impl AgentService {
    /// Construct a service. `depth`/`max_depth` come from the collaboration
    /// config exactly as [`SpawnSubagentTool`](crate::subagent::SpawnSubagentTool)
    /// uses them.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        vault_path: PathBuf,
        allowed_paths: Vec<PathBuf>,
        session_config: SessionConfig,
        security_profile: SecurityProfile,
        depth: u32,
        max_depth: u32,
        default_timeout: Duration,
    ) -> Self {
        Self {
            provider,
            backend_registry: None,
            vault_path,
            allowed_paths,
            session_config,
            security_profile,
            permission_mode: PermissionMode::Default,
            depth,
            max_depth,
            default_timeout,
            db: None,
            bash_settings: crate::bash_sandbox::BashSettings::default(),
            taint: crate::governance::TaintTracker::new(),
        }
    }

    /// Give children's bash tool the operator's `governance.bash` settings.
    pub fn with_bash_settings(mut self, settings: crate::bash_sandbox::BashSettings) -> Self {
        self.bash_settings = settings;
        self
    }

    /// Run every child under the parent session's permission mode.
    pub fn with_permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = mode;
        self
    }

    /// Share the parent session's taint tracker with every child.
    pub fn with_taint(mut self, taint: crate::governance::TaintTracker) -> Self {
        self.taint = taint;
        self
    }

    /// Attach a registry of configured external backends.
    pub fn with_backend_registry(mut self, registry: Option<Arc<BackendRegistry>>) -> Self {
        self.backend_registry = registry;
        self
    }

    /// Attach a telemetry/synergy database handle.
    pub fn with_db(mut self, db: Option<Arc<Database>>) -> Self {
        self.db = db;
        self
    }

    /// This service's depth (for diagnostics/tests).
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Whether this service may still spawn children (recursion guard).
    pub fn can_spawn(&self) -> bool {
        self.depth < self.max_depth
    }

    /// The guardian for one child: the service's profile and permission mode over the child's paths.
    fn child_guardian(&self, allowed_paths: Vec<PathBuf>) -> ToolGuardian {
        ToolGuardian::new(
            allowed_paths,
            self.security_profile.clone(),
            self.permission_mode.clone(),
        )
    }

    /// A child-level service at `depth + 1`, used to give in-process children a
    /// recursive `spawn_subagents` tool without unbounded nesting.
    fn child_service(&self) -> AgentService {
        let mut child = self.clone();
        child.depth = self.depth + 1;
        child
    }

    // ─── Public API ────────────────────────────────────────────

    /// Execute a plan and return one [`ChildOutcome`] per requested child, in
    /// request order. Correlated envelopes stream to `ctx.envelope_sink` as the
    /// run progresses; the returned outcomes are the durable record.
    pub async fn execute(&self, plan: ChildPlan, ctx: ChildExecContext) -> Vec<ChildOutcome> {
        self.execute_with_runs(plan, ctx).await.0
    }

    /// Like [`execute`](Self::execute), also returning each child's registry
    /// run id (child id to run id) so a caller can name them in an ack.
    pub async fn execute_with_runs(
        &self,
        plan: ChildPlan,
        ctx: ChildExecContext,
    ) -> (Vec<ChildOutcome>, Vec<(String, String)>) {
        // Non-blocking mode: detach the whole plan into a background task so
        // the parent turn is not held open by the children's JoinSet. Each
        // child settles on its own and reports through the run registry and
        // `ctx.completion_sink`; this call returns an empty outcome list
        // immediately. The recursion guard still applies, so nothing
        // half-dispatches: either the plan is fully running detached or every
        // child is rejected before we return.
        let detached = !plan.is_blocking() && self.can_spawn();
        if detached && ctx.completion_sink.is_none() && self.db.is_none() {
            let rejected = plan
                .children
                .iter()
                .map(|c| {
                    ChildOutcome::rejected(
                        &c.id,
                        "background delivery is unavailable here: there is no completion route \
                         and no run registry, so results would be lost. Retry with blocking=true.",
                    )
                })
                .collect();
            return (rejected, Vec::new());
        }
        // Registered before dispatch and before the caller sees an ack.
        let ledger = Ledger::open(self.db.clone(), &plan, &ctx, detached);
        let run_ids: Vec<(String, String)> = plan
            .children
            .iter()
            .filter_map(|c| ledger.run_id(&c.id).map(|r| (c.id.clone(), r)))
            .collect();
        let run_ctx = RunCtx {
            parent_run_id: ctx
                .parent_run_id
                .clone()
                .unwrap_or_else(|| format!("agent-service-{}", uuid::Uuid::new_v4())),
            sink: ctx.envelope_sink.clone(),
            parent_messages: ctx.parent_messages.clone(),
            seq: Arc::new(AtomicU64::new(0)),
            ledger,
        };

        if detached {
            return (self.execute_detached(plan, run_ctx), run_ids);
        }
        (self.run_plan(plan, run_ctx).await, run_ids)
    }

    /// Run a registered plan to completion: guard, mode runner, then a final
    /// sweep that settles anything the runners built without going through
    /// `run_one_child` (rejected, blocked, cancelled race losers, panics).
    async fn run_plan(&self, plan: ChildPlan, run_ctx: RunCtx) -> Vec<ChildOutcome> {
        let ledger = run_ctx.ledger.clone();

        // Recursion guard: at or beyond the depth limit, nothing may spawn. Every
        // child is rejected (never silently run) with a clear reason.
        let mut outcomes = if !self.can_spawn() {
            plan.children
                .iter()
                .map(|c| {
                    ChildOutcome::rejected(
                        &c.id,
                        format!(
                            "recursion guard: depth {} has reached max_subagent_depth {}",
                            self.depth, self.max_depth
                        ),
                    )
                })
                .collect()
        } else {
            match plan.mode {
                ChildMode::Single => self.run_single(plan, run_ctx).await,
                ChildMode::Parallel => self.run_parallel(plan, run_ctx).await,
                ChildMode::Race => self.run_race(plan, run_ctx).await,
                ChildMode::Graph => self.run_graph(plan, run_ctx).await,
            }
        };
        for outcome in &mut outcomes {
            ledger.settle(outcome);
        }
        outcomes
    }

    // ─── Non-blocking dispatch ─────────────────────────────────

    /// Detach a registered plan into a background tokio task and return
    /// immediately. The mode runners are the same as the blocking path, so
    /// concurrency bounds, race semantics and graph scheduling apply
    /// identically; only the delivery of results changes. Each child settles
    /// and notifies the moment it finishes (see `run_one_child`), so a fast
    /// blocked child is never held behind a slow sibling.
    fn execute_detached(&self, plan: ChildPlan, run_ctx: RunCtx) -> Vec<ChildOutcome> {
        let svc = self.clone();
        let mut detached = plan;
        detached.blocking = Some(true);
        tokio::spawn(async move {
            svc.run_plan(detached, run_ctx).await;
        });
        Vec::new()
    }

    // ─── One child: governance → selection → run ───────────────

    pub(super) async fn run_one_child(
        &self,
        req: ChildRequest,
        run_ctx: RunCtx,
        cancel: Arc<AtomicBool>,
    ) -> ChildOutcome {
        // 1. Governance + backend selection, applied once, up front. A denial
        //    surfaces as a Rejected outcome — never a silent downgrade or an
        //    execution the caller wasn't permitted.
        let resolution = match self.authorize_and_resolve(&req) {
            Ok(r) => r,
            Err(reason) => {
                let mut outcome = ChildOutcome::rejected(&req.id, reason);
                run_ctx.ledger.settle(&mut outcome);
                return outcome;
            }
        };

        let (req, packet) =
            child_context::prepare_async(self.vault_path.clone(), self.db.clone(), req).await;

        // Capability negotiation: a child that needs a tool it will not have
        // is blocked up front rather than allowed to report success without
        // the deliverable. The role's tool restrictions are never widened.
        let missing = self.missing_capabilities(&req, &resolution);
        if !missing.is_empty() {
            let mut outcome = ChildOutcome::blocked(
                &req.id,
                format!("missing capabilities: {}", missing.join(", ")),
            );
            outcome.resolved_backend = resolution.resolved_backend.clone();
            run_ctx.ledger.settle(&mut outcome);
            return outcome;
        }

        if run_ctx.ledger.is_cancelled(&req.id) {
            let mut outcome = ChildOutcome::blocked(&req.id, "cancelled before it started");
            outcome.status = ChildStatus::Cancelled;
            run_ctx.ledger.settle(&mut outcome);
            return outcome;
        }

        let child_run_id = run_ctx
            .ledger
            .run_id(&req.id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let label = format!("{} [{}]", req.id, req.role());
        self.emit_child_started(&run_ctx, &child_run_id, &label);

        let timeout = resolve_timeout(&req, self.default_timeout);
        run_ctx.ledger.started(&req.id, timeout.as_secs());
        let start = Instant::now();

        // 2. Build the child session for the resolved executor.
        let mut session = self.build_child_session(&req, &resolution, &run_ctx, &child_run_id);
        let _cancel_watch = run_ctx.ledger.watch_cancel(&req.id, cancel.clone());
        session.cancel = cancel;

        // 3. Run with a wall-clock timeout.
        let run = tokio::time::timeout(timeout, session.prompt(&req.goal)).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        let outcome = match run {
            Ok(Ok(result)) => {
                let (status, error) = if result.is_failed() {
                    (
                        ChildStatus::Failed,
                        result.failure_reason().map(|s| s.to_string()),
                    )
                } else {
                    (ChildStatus::Completed, None)
                };
                ChildOutcome {
                    id: req.id.clone(),
                    status,
                    output: result.text().to_string(),
                    resolved_backend: resolution.resolved_backend.clone(),
                    fallback_used: resolution.fallback_used,
                    duration_ms,
                    error,
                    run_id: Some(child_run_id.clone()),
                    effective_model: None,
                    evidence_check: None,
                    run: None,
                }
            }
            Ok(Err(e)) => ChildOutcome {
                id: req.id.clone(),
                status: ChildStatus::Failed,
                output: String::new(),
                resolved_backend: resolution.resolved_backend.clone(),
                fallback_used: resolution.fallback_used,
                duration_ms,
                error: Some(e.to_string()),
                run_id: Some(child_run_id.clone()),
                effective_model: None,
                evidence_check: None,
                run: None,
            },
            Err(_) => ChildOutcome {
                id: req.id.clone(),
                status: ChildStatus::TimedOut,
                output: String::new(),
                resolved_backend: resolution.resolved_backend.clone(),
                fallback_used: resolution.fallback_used,
                duration_ms,
                error: Some(format!("timed out after {}s", timeout.as_secs())),
                run_id: Some(child_run_id.clone()),
                effective_model: None,
                evidence_check: None,
                run: None,
            },
        };
        let mut outcome = finish_outcome(outcome, &resolution.model);
        if let Some(packet) = packet.as_ref().filter(|_| outcome.status.is_success()) {
            outcome.evidence_check =
                child_context::check_citations(&self.vault_path, &outcome.output, packet);
        }
        run_ctx.ledger.settle(&mut outcome);

        self.emit_child_finished(&run_ctx, &child_run_id, outcome.status);
        outcome
    }

    /// Required tools the resolved executor does not provide. External
    /// backends run tool-free, so any requirement is missing there.
    fn missing_capabilities(&self, req: &ChildRequest, resolution: &Resolution) -> Vec<String> {
        if req.required_tools.is_empty() {
            return Vec::new();
        }
        let available: Vec<String> = match resolution.executor {
            ResolvedExecutor::External(_) => Vec::new(),
            ResolvedExecutor::InProcess => self
                .build_inprocess_tools(&self.session_config, &self.provider)
                .iter()
                .map(|t| t.name().to_string())
                .collect(),
        };
        req.required_tools
            .iter()
            .filter(|t| !available.contains(t))
            .cloned()
            .collect()
    }

    /// Governance + backend selection. Returns the resolved executor or a denial
    /// reason. This is the single place path allowlisting and backend policy are
    /// enforced for a child.
    fn authorize_and_resolve(&self, req: &ChildRequest) -> Result<Resolution, String> {
        let mut resolution = self.resolve_backend(req)?;
        let external = match &resolution.executor {
            ResolvedExecutor::External(_) => Some(resolution.resolved_backend.as_str()),
            ResolvedExecutor::InProcess => None,
        };
        resolution.model = resolve_child_model(
            req,
            &self.session_config.model,
            self.backend_registry.as_deref(),
            external,
        )?;
        Ok(resolution)
    }

    fn resolve_backend(&self, req: &ChildRequest) -> Result<Resolution, String> {
        // Path allowlist: a child override must stay within the service's
        // sandbox. Reject an escape rather than quietly widening access.
        let allowed_paths = match &req.allowed_paths {
            Some(overrides) => {
                for p in overrides {
                    if !path_within_any(p, &self.allowed_paths) {
                        return Err(format!(
                            "allowed_paths override '{}' escapes the caller's sandbox",
                            p.display()
                        ));
                    }
                }
                overrides.clone()
            }
            None => self.allowed_paths.clone(),
        };

        // Backend selection.
        match req.backend.as_deref() {
            Some(name) if name.eq_ignore_ascii_case(INPROCESS_BACKEND) => Ok(Resolution {
                executor: ResolvedExecutor::InProcess,
                resolved_backend: INPROCESS_BACKEND.to_string(),
                fallback_used: false,
                allowed_paths,
                model: ModelPlan::default(),
            }),
            Some(name) => {
                // Capability negotiation: a child that names specific files to
                // touch is asking for file-editing work. Reject loud, up
                // front, if the requested backend is a known leaf/text-only
                // harness (no tool loop, no file access) — never dispatch and
                // let it come back with a diff it never applied. Unknown
                // harness names default to agentic (safe: `is_agentic_harness`'s
                // own documented default), so this only fires for harnesses
                // explicitly known to be leaf inference providers.
                if !req.files_in_scope.is_empty()
                    && !hq_tools::harness_chunk::is_agentic_harness(name)
                {
                    return Err(format!(
                        "requested backend '{name}' is a text-only inference harness (no tool \
                         use, no file access) but this child names files_in_scope ({}); it would \
                         return a diff it never applied. Use a tool-capable harness (e.g. \
                         claude-code, codex, github-copilot, antigravity) or the in-process \
                         executor ('hq'), or drop files_in_scope for a pure-reasoning task.",
                        req.files_in_scope.join(", ")
                    ));
                }
                // Explicit external request: honor it or reject. Never silently
                // downgrade to in-process — the caller asked for this backend.
                let backend = self
                    .backend_registry
                    .as_ref()
                    .and_then(|r| r.get(name).map(|b| (name.to_string(), b)));
                match backend {
                    Some((resolved, backend)) => Ok(Resolution {
                        executor: ResolvedExecutor::External(backend),
                        resolved_backend: resolved,
                        fallback_used: false,
                        allowed_paths,
                        model: ModelPlan::default(),
                    }),
                    None => Err(format!(
                        "requested backend '{name}' is not available in the registry"
                    )),
                }
            }
            None => {
                // Auto policy: local-tool work stays in-process; pure
                // reasoning/drafting prefers an external backend when one exists.
                if role_needs_local_tools(req.role()) {
                    Ok(Resolution {
                        executor: ResolvedExecutor::InProcess,
                        resolved_backend: INPROCESS_BACKEND.to_string(),
                        fallback_used: false,
                        allowed_paths,
                        model: ModelPlan::default(),
                    })
                } else if let Some((name, backend)) = self
                    .backend_registry
                    .as_ref()
                    .and_then(|r| r.default_external())
                {
                    Ok(Resolution {
                        executor: ResolvedExecutor::External(backend),
                        resolved_backend: name,
                        fallback_used: false,
                        allowed_paths,
                        model: ModelPlan::default(),
                    })
                } else {
                    // No external backend configured: in-process is a safe
                    // automatic fallback, but it is recorded visibly.
                    Ok(Resolution {
                        executor: ResolvedExecutor::InProcess,
                        resolved_backend: INPROCESS_BACKEND.to_string(),
                        fallback_used: true,
                        allowed_paths,
                        model: ModelPlan::default(),
                    })
                }
            }
        }
    }

    /// Build the [`AgentSession`] for a resolved child, wiring correlation.
    fn build_child_session(
        &self,
        req: &ChildRequest,
        resolution: &Resolution,
        run_ctx: &RunCtx,
        child_run_id: &str,
    ) -> AgentSession {
        let role = req.role();
        let mut child_config = self.session_config.clone();

        // Budget: explicit per-child cap, else a fraction of the parent's.
        if let Some(cap) = req.max_budget_usd {
            child_config.max_budget_usd = Some(cap);
        } else if let Some(parent_cap) = child_config.max_budget_usd {
            child_config.max_budget_usd = Some(parent_cap * budget_fraction(role));
        }

        let external = matches!(resolution.executor, ResolvedExecutor::External(_));

        // Model routing was decided before dispatch (`resolve_child_model`):
        // explicit override, else role alias, else the parent's model.
        if let Some(model) = &resolution.model.config_model {
            child_config.model = model.clone();
        }
        let provider = resolution
            .model
            .provider
            .clone()
            .unwrap_or_else(|| self.provider.clone());

        // Tools: in-process children get the full HQ coding tool set (plus a
        // recursion-guarded spawn_subagents tool); external children run
        // tool-free (pure reasoning/drafting).
        let mut guardian = self.child_guardian(resolution.allowed_paths.clone());
        guardian.set_denial_notifier(crate::builder::mailbox_denial_notifier(
            self.vault_path.clone(),
        ));
        guardian.set_taint(self.taint.clone());
        let mut tools = if external {
            Vec::new()
        } else {
            self.build_inprocess_tools(&child_config, &provider)
        };
        // External children are tool-free, so a catalog they cannot load from is noise.
        let skill_index = (!external)
            .then(|| {
                hq_tools::skills::SkillHintIndex::build(&hq_core::skills_dir(&self.vault_path))
            })
            .filter(|index| !index.is_empty());
        if skill_index.is_some() {
            tools.push(Box::new(crate::builder::HqToolAdapter {
                inner: self.load_skill_tool(child_run_id),
            }));
        }
        // child_config is cloned from the parent's session_config above, so a
        // subagent spawned from a live session inherits that liveness — a
        // human is still supervising the top-level conversation that spawned it.
        let governed = guardian.build_registry(
            tools,
            crate::governance::LiveUserTurn::from_session_config(&child_config),
        );

        let mut session = AgentSession::new(provider, governed, child_config);
        if let ResolvedExecutor::External(backend) = &resolution.executor {
            session.set_backend(backend.clone());
        }
        session.set_vault_path(self.vault_path.clone());
        if let Some(ref db) = self.db {
            session.set_telemetry_db(db.clone());
        }

        // Correlation: pin the announced run id and tag the parent so forwarded
        // envelopes join the parent's stream.
        session.set_parent_run_id(Some(run_ctx.parent_run_id.clone()));
        session.set_run_id(child_run_id.to_string());
        let ledger = run_ctx.ledger.clone();
        let child_id = req.id.clone();
        let sink = run_ctx.sink.clone();
        session.on_envelope(move |env| {
            ledger.touch(&child_id);
            if let Some(sink) = &sink {
                sink(env);
            }
        });

        // System prompt + optional fork context.
        let soul_summary = hq_vault::system::get_soul_summary(&self.vault_path);
        let context = self.compose_context(req);
        let system = crate::subagent::build_subagent_system_prompt(
            &context,
            role,
            self.depth,
            self.max_depth,
            &soul_summary,
        );
        session.set_system_prompt(system.clone());
        if let Some(index) = skill_index {
            let max_tokens = crate::builder::prompt::MAX_SKILL_TOKENS;
            let (enriched, _) =
                hq_tools::skills::enrich_system_prompt(&index, &system, "", None, Some(max_tokens));
            session.set_system_prompt(enriched);
            session.set_skill_index(Arc::new(index), max_tokens);
        }

        if let Some(parent_msgs) = &run_ctx.parent_messages {
            session.inject_fork_context(parent_msgs.clone());
        }

        session
    }

    /// `load_skill` for a child, logged under the child's run id when telemetry is on.
    fn load_skill_tool(&self, child_run_id: &str) -> Box<dyn hq_tools::HqTool> {
        let skills_dir = hq_core::skills_dir(&self.vault_path);
        match &self.db {
            Some(db) => Box::new(hq_tools::skills::LoadSkillTool::with_telemetry(
                skills_dir,
                db.clone(),
                child_run_id,
            )),
            None => Box::new(hq_tools::skills::LoadSkillTool::new(skills_dir)),
        }
    }

    /// Build the in-process coding tool set for a child, mirroring the tool list
    /// `SpawnSubagentTool::run_inprocess` installs. A recursion-guarded
    /// `spawn_subagents` tool is added when depth+1 is still under the limit.
    fn build_inprocess_tools(
        &self,
        child_config: &SessionConfig,
        provider: &Arc<dyn LlmProvider>,
    ) -> Vec<Box<dyn crate::tools::AgentTool>> {
        let state_cache = hq_tools::file_edit::FileStateCache::default();
        let history = hq_tools::file_edit::FileHistory::default();
        let mut tools: Vec<Box<dyn crate::tools::AgentTool>> = vec![
            Box::new(crate::coding::BashTool::new(self.bash_settings.clone())),
            Box::new(crate::coding::ReadTool::new(state_cache.clone())),
            Box::new(crate::coding::WriteTool::new(
                state_cache.clone(),
                history.clone(),
            )),
            Box::new(crate::coding::EditTool::new(
                state_cache.clone(),
                history.clone(),
            )),
            Box::new(crate::builder::HqToolAdapter {
                inner: Box::new(hq_tools::coding::BatchEditTool::new(
                    state_cache.clone(),
                    history.clone(),
                )),
            }),
            Box::new(crate::coding::RollbackTool::new(state_cache, history)),
            Box::new(crate::coding::FindTool),
            Box::new(crate::coding::GrepTool),
            Box::new(crate::coding::LsTool),
            Box::new(crate::builder::HqToolAdapter {
                inner: Box::new(hq_tools::coding::TodoWriteTool::new(
                    hq_tools::coding::TodoStore::new(),
                )),
            }),
        ];

        let lsp_manager = crate::lsp_tools::shared_lsp_manager();
        crate::lsp_tools::register_lsp_tools(&mut tools, lsp_manager);

        // Recursion-guarded decomposition tool. A child at `depth + 1` may spawn
        // further children only while that is still under `max_depth`.
        let child_service = self.child_service();
        if child_service.can_spawn() {
            let mut recursive = child_service;
            recursive.session_config = child_config.clone();
            recursive.provider = provider.clone();
            tools.push(Box::new(super::tool::SpawnSubagentsTool::new(Arc::new(
                recursive,
            ))));
        }

        tools
    }

    /// Compose the context block threaded into a child's system prompt from its
    /// explicit context plus files-in-scope / success-criteria hints.
    fn compose_context(&self, req: &ChildRequest) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(ctx) = &req.context
            && !ctx.is_empty()
        {
            parts.push(ctx.clone());
        }
        if !req.files_in_scope.is_empty() {
            parts.push(format!("Files in scope: {}", req.files_in_scope.join(", ")));
        }
        if !req.success_criteria.is_empty() {
            let criteria = req
                .success_criteria
                .iter()
                .enumerate()
                .map(|(i, c)| format!("  {}. {}", i + 1, c))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!("Success criteria:\n{criteria}"));
        }
        parts.push(REPORTING_PROTOCOL.to_string());
        parts.join("\n\n")
    }

    // ─── Envelope correlation ──────────────────────────────────

    fn emit_child_started(&self, run_ctx: &RunCtx, child_run_id: &str, label: &str) {
        let Some(sink) = &run_ctx.sink else {
            return;
        };
        let env = SessionEventEnvelope::lifecycle(
            run_ctx.parent_run_id.clone(),
            None,
            run_ctx.seq.fetch_add(1, Ordering::Relaxed),
            EventSource::Session,
            EnvelopeKind::ChildStarted {
                child_run_id: child_run_id.to_string(),
                label: label.to_string(),
            },
        );
        sink(env);
    }

    fn emit_child_finished(&self, run_ctx: &RunCtx, child_run_id: &str, status: ChildStatus) {
        let Some(sink) = &run_ctx.sink else {
            return;
        };
        let outcome = match status {
            ChildStatus::Completed => "complete",
            ChildStatus::Failed => "failed",
            ChildStatus::TimedOut => "timed_out",
            ChildStatus::Blocked => "blocked",
            ChildStatus::Rejected => "rejected",
            ChildStatus::Cancelled => "cancelled",
        };
        let env = SessionEventEnvelope::lifecycle(
            run_ctx.parent_run_id.clone(),
            None,
            run_ctx.seq.fetch_add(1, Ordering::Relaxed),
            EventSource::Session,
            EnvelopeKind::ChildFinished {
                child_run_id: child_run_id.to_string(),
                outcome: outcome.to_string(),
            },
        );
        sink(env);
    }
}

// ─── Free helpers ──────────────────────────────────────────────

/// Whether a role's work needs local file/tool access (Read/Edit/Write/Bash).
/// Planners produce plans (pure drafting) and can run on an external backend;
/// every other role touches the local tree.
fn role_needs_local_tools(role: &str) -> bool {
    !matches!(role, "planner")
}

/// Fraction of the parent's USD budget a child of this role receives.
fn budget_fraction(role: &str) -> f64 {
    match role {
        "explorer" | "verifier" => 0.10,
        "planner" => 0.15,
        _ => 0.25,
    }
}

/// Whether `path` is equal to or nested under any base in `bases` (lexical).
fn path_within_any(path: &Path, bases: &[PathBuf]) -> bool {
    bases
        .iter()
        .any(|base| path == base || path.starts_with(base))
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod capability_negotiation_tests;
