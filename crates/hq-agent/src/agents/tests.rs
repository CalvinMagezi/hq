//! Tests for the unified [`AgentService`](super::AgentService) runtime.

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use hq_core::types::{
    ChatMessage, EnvelopeKind, MessageRole, SecurityProfile, SessionEventEnvelope, SubagentType,
};
use hq_llm::provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};
use tokio_stream::Stream;

use crate::backend::{ApiBackend, BackendRegistry, SessionBackend};
use crate::session::SessionConfig;

use super::service::AgentService;
use super::tool::SpawnSubagentsTool;
use super::types::{
    ChildCompletionEvent, ChildExecContext, ChildPlan, ChildRequest, ChildStatus, EnvelopeSink,
};

// ─── Marker-driven mock provider ───────────────────────────────
//
// Behavior is chosen by markers embedded in the child's goal text:
//   "[[ERR]]"   → return an error (fast Failed with max_retries=0)
//   "[[SLEEP]]" → sleep 3s then complete (race losers / timeouts)
//   otherwise   → complete immediately, incrementing the completion counter.
// A concurrency tracker records the peak overlap so graph mode can assert it
// respects `max_concurrent`.

#[derive(Default)]
struct ConcurrencyTracker {
    current: AtomicUsize,
    peak: AtomicUsize,
}

impl ConcurrencyTracker {
    fn enter(&self) {
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
    }
    fn exit(&self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }
    fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

pub(super) struct MarkerProvider {
    name: String,
    pub(super) completions: Arc<AtomicUsize>,
    tracker: Arc<ConcurrencyTracker>,
    /// Per-completion delay to force overlap when tracking concurrency.
    work: Duration,
}

impl MarkerProvider {
    pub(super) fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            completions: Arc::new(AtomicUsize::new(0)),
            tracker: Arc::new(ConcurrencyTracker::default()),
            work: Duration::from_millis(0),
        })
    }

    fn with_work(name: &str, work: Duration) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            completions: Arc::new(AtomicUsize::new(0)),
            tracker: Arc::new(ConcurrencyTracker::default()),
            work,
        })
    }

    fn combined_user_text(request: &ChatRequest) -> String {
        request
            .messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::User))
            .map(|m| m.content.clone())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[async_trait]
impl LlmProvider for MarkerProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        let text = Self::combined_user_text(request);
        if text.contains("[[PANIC]]") {
            panic!("marker-provider forced panic");
        }
        if text.contains("[[ERR]]") {
            return Err(LlmError::Auth {
                status: 500,
                message: "marker-provider forced error".to_string(),
            }
            .into());
        }
        self.tracker.enter();
        if text.contains("[[SLEEP]]") {
            tokio::time::sleep(Duration::from_secs(3)).await;
        } else if !self.work.is_zero() {
            tokio::time::sleep(self.work).await;
        }
        self.tracker.exit();
        self.completions.fetch_add(1, Ordering::SeqCst);
        Ok(ChatResponse {
            message: ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Assistant,
                content: format!("done: {}", text.trim()),
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: None,
            },
            input_tokens: 5,
            output_tokens: 3,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: "marker-model".to_string(),
        })
    }

    async fn chat_stream(
        &self,
        _request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        Ok(Box::pin(tokio_stream::iter(vec![Ok(StreamChunk::Done)])))
    }
}

// ─── Test fixtures ─────────────────────────────────────────────

fn fast_config() -> SessionConfig {
    let mut cfg = SessionConfig {
        max_retries: 0,
        retry_base_delay: Duration::from_millis(1),
        max_duration_secs: None,
        ..Default::default()
    };
    cfg.self_healing.enabled = false;
    cfg
}

pub(super) fn service_with(
    provider: Arc<dyn LlmProvider>,
    depth: u32,
    max_depth: u32,
    allowed: Vec<PathBuf>,
) -> AgentService {
    AgentService::new(
        provider,
        std::env::temp_dir(),
        allowed,
        fast_config(),
        SecurityProfile::Guarded,
        depth,
        max_depth,
        Duration::from_secs(30),
    )
}

pub(super) fn goal_task(id: &str, goal: &str) -> ChildRequest {
    ChildRequest::new(id, goal)
}

// ─── 1. Single mode + governance rejection ─────────────────────

#[tokio::test]
async fn single_mode_runs_in_process_to_completion() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let outcomes = svc
        .execute(
            ChildPlan::single(goal_task("t1", "summarize the module")),
            ChildExecContext::default(),
        )
        .await;

    assert_eq!(outcomes.len(), 1);
    let o = &outcomes[0];
    assert_eq!(o.status, ChildStatus::Completed, "outcome: {o:?}");
    assert_eq!(o.resolved_backend, "hq");
    assert!(!o.fallback_used);
    assert!(o.run_id.is_some());
    assert_eq!(provider.completions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn governance_rejects_allowed_path_escape() {
    let provider = MarkerProvider::new("hq-mock");
    let sandbox = std::env::temp_dir().join("agent-service-sandbox");
    let svc = service_with(provider.clone(), 0, 3, vec![sandbox]);

    let mut req = goal_task("t1", "read a secret");
    // An override pointing outside the sandbox must be rejected, not widened.
    req.allowed_paths = Some(vec![PathBuf::from("/etc")]);

    let outcomes = svc
        .execute(ChildPlan::single(req), ChildExecContext::default())
        .await;

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, ChildStatus::Rejected);
    assert!(
        outcomes[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("escapes"),
        "reason: {:?}",
        outcomes[0].error
    );
    // The child never ran.
    assert_eq!(provider.completions.load(Ordering::SeqCst), 0);
}

// ─── 2. Parallel isolation ─────────────────────────────────────

#[tokio::test]
async fn parallel_isolation_one_failure_does_not_cancel_siblings() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let plan = ChildPlan::parallel(
        vec![
            goal_task("ok-1", "do the first thing"),
            goal_task("bad", "this one [[ERR]] blows up"),
            goal_task("ok-2", "do the third thing"),
        ],
        3,
    );

    let outcomes = svc.execute(plan, ChildExecContext::default()).await;
    assert_eq!(outcomes.len(), 3);

    let status = |id: &str| outcomes.iter().find(|o| o.id == id).unwrap().status;
    assert_eq!(status("ok-1"), ChildStatus::Completed);
    assert_eq!(status("ok-2"), ChildStatus::Completed);
    assert_eq!(status("bad"), ChildStatus::Failed);
    // Both healthy siblings completed despite the failure.
    assert_eq!(provider.completions.load(Ordering::SeqCst), 2);
}

// ─── 3. Race: first success wins, losers cancelled ─────────────

#[tokio::test]
async fn race_first_success_cancels_losers() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    // The winner completes immediately; the two losers sleep 3s and would only
    // increment the completion counter if they were *not* cancelled.
    let plan = ChildPlan::race(vec![
        goal_task("slow-a", "take your time [[SLEEP]]"),
        goal_task("winner", "answer now"),
        goal_task("slow-b", "also slow [[SLEEP]]"),
    ]);

    let start = std::time::Instant::now();
    let outcomes = svc.execute(plan, ChildExecContext::default()).await;
    let elapsed = start.elapsed();

    assert_eq!(outcomes.len(), 3);
    let status = |id: &str| outcomes.iter().find(|o| o.id == id).unwrap().status;
    assert_eq!(status("winner"), ChildStatus::Completed);
    assert_eq!(status("slow-a"), ChildStatus::Cancelled);
    assert_eq!(status("slow-b"), ChildStatus::Cancelled);
    // Only the winner completed; the sleepers were aborted before finishing.
    assert_eq!(provider.completions.load(Ordering::SeqCst), 1);
    // The race returns promptly rather than waiting out the 3s sleepers.
    assert!(elapsed < Duration::from_secs(2), "race took {elapsed:?}");
}

// ─── 4. Graph: concurrency bound + dependency ordering ─────────

#[tokio::test]
async fn graph_respects_max_concurrent_after_dependencies_resolve() {
    // 100ms of work per child forces overlap so the peak tracker is meaningful.
    let provider = MarkerProvider::with_work("hq-mock", Duration::from_millis(120));
    let tracker = provider.tracker.clone();
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    // Root, then three children that all depend on it. After the root resolves
    // the three become ready together and must run 2-at-a-time (max_concurrent).
    let mut a = goal_task("a", "phase two a");
    a.depends_on = vec!["root".to_string()];
    let mut b = goal_task("b", "phase two b");
    b.depends_on = vec!["root".to_string()];
    let mut c = goal_task("c", "phase two c");
    c.depends_on = vec!["root".to_string()];

    let plan = ChildPlan::graph(vec![goal_task("root", "phase one"), a, b, c], 2);
    let outcomes = svc.execute(plan, ChildExecContext::default()).await;

    assert_eq!(outcomes.len(), 4);
    assert!(
        outcomes.iter().all(|o| o.status == ChildStatus::Completed),
        "outcomes: {outcomes:?}"
    );
    // Dependents ran concurrently, but never more than the bound at once.
    assert_eq!(tracker.peak(), 2, "peak concurrency should hit the bound");
}

#[tokio::test]
async fn graph_failed_dependency_blocks_dependent_with_reason() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let root = goal_task("root", "this fails [[ERR]]");
    let mut dependent = goal_task("dependent", "needs root");
    dependent.depends_on = vec!["root".to_string()];
    let independent = goal_task("independent", "unrelated work");

    let plan = ChildPlan::graph(vec![root, dependent, independent], 3);
    let outcomes = svc.execute(plan, ChildExecContext::default()).await;

    let get = |id: &str| outcomes.iter().find(|o| o.id == id).unwrap();
    assert_eq!(get("root").status, ChildStatus::Failed);
    assert_eq!(get("dependent").status, ChildStatus::Blocked);
    assert!(
        get("dependent")
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("root"),
        "block reason should name the failed dep: {:?}",
        get("dependent").error
    );
    // The unrelated child still ran.
    assert_eq!(get("independent").status, ChildStatus::Completed);
}

#[tokio::test]
async fn graph_missing_dependency_is_blocked_not_hung() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let mut orphan = goal_task("orphan", "waits forever");
    orphan.depends_on = vec!["does-not-exist".to_string()];

    let outcomes = svc
        .execute(
            ChildPlan::graph(vec![orphan], 2),
            ChildExecContext::default(),
        )
        .await;

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, ChildStatus::Blocked);
    assert!(
        outcomes[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("missing dependency")
    );
}

// ─── 5. Recursion guard ────────────────────────────────────────

#[tokio::test]
async fn recursion_guard_rejects_at_max_depth() {
    let provider = MarkerProvider::new("hq-mock");
    // depth == max_depth: can_spawn() is false.
    let svc = service_with(provider.clone(), 2, 2, vec![std::env::temp_dir()]);
    assert!(!svc.can_spawn());

    let outcomes = svc
        .execute(
            ChildPlan::single(goal_task("t1", "spawn me")),
            ChildExecContext::default(),
        )
        .await;

    assert_eq!(outcomes[0].status, ChildStatus::Rejected);
    assert!(
        outcomes[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("recursion guard")
    );
    assert_eq!(provider.completions.load(Ordering::SeqCst), 0);
}

// ─── 6. Backend selection (explicit vs auto, visible resolution) ─

fn external_registry() -> Arc<BackendRegistry> {
    let ext = MarkerProvider::new("ext-mock");
    let backend: Arc<dyn SessionBackend> =
        Arc::new(ApiBackend::new("ext", ext as Arc<dyn LlmProvider>));
    Arc::new(BackendRegistry::from_backends(
        vec![("ext".to_string(), backend)],
        "ext",
    ))
}

#[tokio::test]
async fn explicit_backend_is_honored_and_recorded() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider, 0, 3, vec![std::env::temp_dir()])
        .with_backend_registry(Some(external_registry()));

    let mut req = goal_task("t1", "reason about this");
    req.backend = Some("ext".to_string());

    let outcomes = svc
        .execute(ChildPlan::single(req), ChildExecContext::default())
        .await;

    assert_eq!(
        outcomes[0].status,
        ChildStatus::Completed,
        "{:?}",
        outcomes[0]
    );
    assert_eq!(outcomes[0].resolved_backend, "ext");
    assert!(!outcomes[0].fallback_used);
}

#[tokio::test]
async fn unknown_explicit_backend_is_rejected_not_downgraded() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()])
        .with_backend_registry(Some(external_registry()));

    let mut req = goal_task("t1", "reason about this");
    req.backend = Some("ghost".to_string());

    let outcomes = svc
        .execute(ChildPlan::single(req), ChildExecContext::default())
        .await;

    assert_eq!(outcomes[0].status, ChildStatus::Rejected);
    assert!(
        outcomes[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("not available")
    );
    // Never silently ran in-process instead.
    assert_eq!(provider.completions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn auto_selects_external_for_pure_reasoning_role() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()])
        .with_backend_registry(Some(external_registry()));

    // Planner is pure drafting → external backend preferred when available.
    let mut req = goal_task("t1", "draft a plan");
    req.agent_type = SubagentType::Planner;

    let outcomes = svc
        .execute(ChildPlan::single(req), ChildExecContext::default())
        .await;

    assert_eq!(outcomes[0].status, ChildStatus::Completed);
    assert_eq!(outcomes[0].resolved_backend, "ext");
    assert!(!outcomes[0].fallback_used);
    // The in-process provider was not used for a planner with an external option.
    assert_eq!(provider.completions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn auto_falls_back_to_in_process_visibly_when_no_external() {
    let provider = MarkerProvider::new("hq-mock");
    // No registry: a planner has no external option and falls back in-process.
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let mut req = goal_task("t1", "draft a plan");
    req.agent_type = SubagentType::Planner;

    let outcomes = svc
        .execute(ChildPlan::single(req), ChildExecContext::default())
        .await;

    assert_eq!(outcomes[0].status, ChildStatus::Completed);
    assert_eq!(outcomes[0].resolved_backend, "hq");
    assert!(
        outcomes[0].fallback_used,
        "in-process fallback for a reasoning role must be visible"
    );
    assert_eq!(provider.completions.load(Ordering::SeqCst), 1);
}

// ─── 7. Child envelope correlation ─────────────────────────────

#[tokio::test]
async fn child_envelopes_are_correlated_to_the_parent() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider, 0, 3, vec![std::env::temp_dir()]);

    let collected: Arc<std::sync::Mutex<Vec<SessionEventEnvelope>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink_store = collected.clone();
    let sink: EnvelopeSink = Arc::new(move |env| {
        sink_store.lock().unwrap().push(env);
    });

    let ctx = ChildExecContext {
        parent_run_id: Some("parent-123".to_string()),
        envelope_sink: Some(sink),
        parent_messages: None,
        ..Default::default()
    };

    let outcomes = svc
        .execute(ChildPlan::single(goal_task("t1", "correlate me")), ctx)
        .await;
    let child_run_id = outcomes[0].run_id.clone().expect("child run id");

    let envs = collected.lock().unwrap();

    // ChildStarted / ChildFinished markers, keyed to the child's run id.
    let started = envs.iter().find_map(|e| match &e.kind {
        EnvelopeKind::ChildStarted {
            child_run_id,
            label,
        } => Some((child_run_id.clone(), label.clone())),
        _ => None,
    });
    let finished = envs.iter().find_map(|e| match &e.kind {
        EnvelopeKind::ChildFinished {
            child_run_id,
            outcome,
        } => Some((child_run_id.clone(), outcome.clone())),
        _ => None,
    });
    let (started_id, _label) = started.expect("ChildStarted observed");
    let (finished_id, finished_outcome) = finished.expect("ChildFinished observed");
    assert_eq!(started_id, child_run_id);
    assert_eq!(finished_id, child_run_id);
    assert_eq!(finished_outcome, "complete");

    // At least one forwarded child event carries the pinned run id and points
    // back at the parent run.
    let forwarded = envs.iter().any(|e| {
        e.run_id == child_run_id
            && e.parent_run_id.as_deref() == Some("parent-123")
            && matches!(
                e.kind,
                EnvelopeKind::Session(_) | EnvelopeKind::RunStarted { .. }
            )
    });
    assert!(
        forwarded,
        "expected forwarded child envelopes tagged to the parent"
    );
}

// ─── 8. spawn_subagents tool end-to-end ────────────────────────

#[tokio::test]
async fn spawn_subagents_tool_single_task() {
    use crate::tools::AgentTool;

    let provider = MarkerProvider::new("hq-mock");
    let svc = Arc::new(service_with(
        provider.clone(),
        0,
        3,
        vec![std::env::temp_dir()],
    ));
    let tool = SpawnSubagentsTool::new(svc);

    let result = tool
        .execute(
            "call-1",
            serde_json::json!({
                "task": { "id": "only", "goal": "do the single thing" }
            }),
        )
        .await
        .expect("tool executes");

    let text = &result.content[0].text;
    assert!(text.contains("single mode"), "summary: {text}");
    assert!(text.contains("1/1 completed"), "summary: {text}");

    let details = result.details.expect("details json");
    assert_eq!(details["completed"], 1);
    assert_eq!(details["results"][0]["status"], "completed");
    assert_eq!(provider.completions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn spawn_subagents_tool_parallel_list() {
    use crate::tools::AgentTool;

    let provider = MarkerProvider::new("hq-mock");
    let svc = Arc::new(service_with(
        provider.clone(),
        0,
        3,
        vec![std::env::temp_dir()],
    ));
    let tool = SpawnSubagentsTool::new(svc);

    let result = tool
        .execute(
            "call-1",
            serde_json::json!({
                "mode": "parallel",
                "tasks": [
                    { "id": "a", "goal": "first" },
                    { "id": "b", "goal": "second" }
                ],
                "max_concurrent": 2
            }),
        )
        .await
        .expect("tool executes");

    let details = result.details.expect("details json");
    assert_eq!(details["mode"], "parallel");
    assert_eq!(details["completed"], 2);
    assert_eq!(provider.completions.load(Ordering::SeqCst), 2);
}

// ─── Panic handling: a lost/panicked child must be surfaced, never dropped ──
//
// A bare `JoinError` carries no request id. Regression coverage for the fix:
// Parallel/Graph/Race must reconcile the submitted id set against what came
// back from the `JoinSet`, or (a) Parallel silently loses the result, and (b)
// Graph considers the id "not yet run" forever and resubmits it every loop
// iteration, spinning indefinitely.

#[tokio::test]
async fn parallel_child_panic_is_reported_not_dropped() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let plan = ChildPlan::parallel(
        vec![
            goal_task("ok-1", "do the first thing"),
            goal_task("boom", "this one [[PANIC]] blows up"),
            goal_task("ok-2", "do the third thing"),
        ],
        3,
    );

    let outcomes = svc.execute(plan, ChildExecContext::default()).await;
    assert_eq!(
        outcomes.len(),
        3,
        "a panicked child must not vanish from the result set"
    );

    let status = |id: &str| outcomes.iter().find(|o| o.id == id).unwrap().status;
    assert_eq!(status("ok-1"), ChildStatus::Completed);
    assert_eq!(status("ok-2"), ChildStatus::Completed);
    assert_eq!(status("boom"), ChildStatus::Failed);
    assert_eq!(provider.completions.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn graph_child_panic_does_not_loop_forever_and_is_reported() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let root = goal_task("root", "this panics [[PANIC]]");
    let mut dependent = goal_task("dependent", "needs root");
    dependent.depends_on = vec!["root".to_string()];

    let plan = ChildPlan::graph(vec![root, dependent], 3);

    // Bound the run: before the fix, a panic left the root "not yet run"
    // forever and the scheduling loop resubmitted it on every iteration,
    // hanging indefinitely instead of returning.
    let outcomes = tokio::time::timeout(
        Duration::from_secs(5),
        svc.execute(plan, ChildExecContext::default()),
    )
    .await
    .expect("graph mode must terminate after a child panics, not loop forever");

    let get = |id: &str| outcomes.iter().find(|o| o.id == id).unwrap();
    assert_eq!(
        get("root").status,
        ChildStatus::Failed,
        "outcomes: {outcomes:?}"
    );
    assert_eq!(get("dependent").status, ChildStatus::Blocked);
}

#[tokio::test]
async fn race_lone_panicking_child_is_reported_as_failed_not_a_race_loss() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let plan = ChildPlan::race(vec![goal_task("only", "this one [[PANIC]] blows up")]);
    let outcomes = svc.execute(plan, ChildExecContext::default()).await;

    assert_eq!(outcomes.len(), 1);
    // With no winner, a panicked contender must be reported as a genuine
    // failure, not mislabeled "cancelled: another child won the race".
    assert_eq!(outcomes[0].status, ChildStatus::Failed);
}

#[tokio::test]
async fn spawn_subagents_tool_single_mode_rejects_multiple_tasks() {
    use crate::tools::AgentTool;

    let provider = MarkerProvider::new("hq-mock");
    let svc = Arc::new(service_with(
        provider.clone(),
        0,
        3,
        vec![std::env::temp_dir()],
    ));
    let tool = SpawnSubagentsTool::new(svc);

    let result = tool
        .execute(
            "call-1",
            serde_json::json!({
                "mode": "single",
                "tasks": [
                    { "id": "a", "goal": "first" },
                    { "id": "b", "goal": "second" }
                ]
            }),
        )
        .await
        .expect("tool executes");

    // Must be rejected up front rather than silently running only the first
    // task and discarding the rest.
    let text = result
        .content
        .first()
        .map(|c| c.text.as_str())
        .unwrap_or_default();
    assert!(text.contains("Rejected"), "expected rejection, got: {text}");
    assert_eq!(provider.completions.load(Ordering::SeqCst), 0);
}

// ─── 8. Non-blocking (detached) execution ──────────────────────

/// Collect ChildCompletionEvents via a test sink.
pub(super) fn event_collector() -> (
    Arc<std::sync::Mutex<Vec<ChildCompletionEvent>>>,
    super::types::CompletionSink,
) {
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink_events = events.clone();
    let sink: super::types::CompletionSink = Arc::new(move |ev: ChildCompletionEvent| {
        sink_events.lock().unwrap().push(ev);
    });
    (events, sink)
}

/// Wait until `events` holds `n` entries or the deadline passes.
pub(super) async fn wait_for_events(
    events: &Arc<std::sync::Mutex<Vec<ChildCompletionEvent>>>,
    n: usize,
    timeout: Duration,
) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if events.lock().unwrap().len() >= n {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {n} completion events (got {})",
            events.lock().unwrap().len()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn nonblocking_parallel_returns_immediately_and_emits_completion_events() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let (events, sink) = event_collector();
    let ctx = ChildExecContext {
        parent_turn_id: Some("turn-42".to_string()),
        completion_sink: Some(sink),
        ..Default::default()
    };

    // Three 3s sleepers: a blocking parallel plan would take ~3s. Detached,
    // execute must return well under 1s with no inline outcomes.
    let mut plan = ChildPlan::parallel(
        vec![
            goal_task("nb-a", "slow [[SLEEP]]"),
            goal_task("nb-b", "slow [[SLEEP]]"),
            goal_task("nb-c", "slow [[SLEEP]]"),
        ],
        3,
    );
    plan.blocking = Some(false);

    let start = std::time::Instant::now();
    let outcomes = svc.execute(plan, ctx).await;
    let elapsed = start.elapsed();

    assert!(
        outcomes.is_empty(),
        "detached execute returns nothing inline"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "detached execute blocked for {elapsed:?}"
    );

    // Each child emits exactly one completion event carrying its id, the
    // parent turn id, and a success flag.
    wait_for_events(&events, 3, Duration::from_secs(10)).await;
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 3);
    for id in ["nb-a", "nb-b", "nb-c"] {
        let ev = events
            .iter()
            .find(|e| e.task_id == id)
            .unwrap_or_else(|| panic!("missing event for {id}"));
        assert!(ev.success, "{id} should have succeeded: {ev:?}");
        assert_eq!(ev.parent_turn_id.as_deref(), Some("turn-42"));
        assert_eq!(ev.role, "general");
        assert!(!ev.summary.is_empty());
    }
    assert_eq!(provider.completions.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn nonblocking_emits_failure_events_with_success_false() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let (events, sink) = event_collector();
    let ctx = ChildExecContext {
        completion_sink: Some(sink),
        ..Default::default()
    };

    let mut plan = ChildPlan::parallel(
        vec![
            goal_task("good", "fine"),
            goal_task("bad", "this one [[ERR]] blows up"),
        ],
        2,
    );
    plan.blocking = Some(false);

    let outcomes = svc.execute(plan, ctx).await;
    assert!(outcomes.is_empty());

    wait_for_events(&events, 2, Duration::from_secs(5)).await;
    let events = events.lock().unwrap();
    let good = events.iter().find(|e| e.task_id == "good").unwrap();
    let bad = events.iter().find(|e| e.task_id == "bad").unwrap();
    assert!(good.success);
    assert!(!bad.success, "failure must be visible in the event");
    assert!(!bad.summary.is_empty(), "error text becomes the summary");
    // No parent turn supplied: the field stays None, never fabricated.
    assert!(bad.parent_turn_id.is_none());
}

#[tokio::test]
async fn nonblocking_race_reports_win_and_cancellations_via_events() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let (events, sink) = event_collector();
    let ctx = ChildExecContext {
        parent_turn_id: Some("turn-race".to_string()),
        completion_sink: Some(sink),
        ..Default::default()
    };

    let mut plan = ChildPlan::race(vec![
        goal_task("slow", "take your time [[SLEEP]]"),
        goal_task("winner", "answer now"),
    ]);
    plan.blocking = Some(false);

    let start = std::time::Instant::now();
    let outcomes = svc.execute(plan, ctx).await;
    assert!(outcomes.is_empty());
    assert!(start.elapsed() < Duration::from_secs(1));

    // The winner reports success; the cancelled loser still reports so a
    // consumer can reconcile the full set. (Both events arrive once the race
    // resolves, promptly — losers are aborted, not awaited.)
    wait_for_events(&events, 2, Duration::from_secs(10)).await;
    let events = events.lock().unwrap();
    let winner = events.iter().find(|e| e.task_id == "winner").unwrap();
    let loser = events.iter().find(|e| e.task_id == "slow").unwrap();
    assert!(winner.success);
    assert!(!loser.success, "a cancelled race loser is not a success");
    assert_eq!(winner.parent_turn_id.as_deref(), Some("turn-race"));
}

#[tokio::test]
async fn blocking_absent_is_unchanged_when_sink_present() {
    let provider = MarkerProvider::new("hq-mock");
    let svc = service_with(provider.clone(), 0, 3, vec![std::env::temp_dir()]);

    let (events, sink) = event_collector();
    let ctx = ChildExecContext {
        completion_sink: Some(sink),
        ..Default::default()
    };

    // A default (blocking) plan returns inline even when a sink is attached:
    // events are only for detached runs.
    let outcomes = svc
        .execute(ChildPlan::single(goal_task("t1", "inline work")), ctx)
        .await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].status, ChildStatus::Completed);
    assert!(
        events.lock().unwrap().is_empty(),
        "blocking runs must not emit completion events"
    );
}

#[tokio::test]
async fn report_progress_tool_fires_event_with_sink() {
    use crate::tools::AgentTool;

    let events: Arc<std::sync::Mutex<Vec<crate::native_hq::ProgressEvent>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let store = events.clone();
    let sink: crate::native_hq::ProgressSink = Arc::new(move |ev| {
        store.lock().unwrap().push(ev);
    });

    let tool = super::tool::ReportProgressTool::new().with_context(ChildExecContext {
        parent_turn_id: Some("turn-42".to_string()),
        progress_sink: Some(sink),
        ..Default::default()
    });

    let result = tool
        .execute("call-1", serde_json::json!({ "message": "halfway there" }))
        .await
        .expect("tool executes");
    assert_eq!(result.content[0].text, "Progress reported.");

    let fired = events.lock().unwrap();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].turn_id, "turn-42");
    assert_eq!(fired[0].note.as_deref(), Some("halfway there"));
}

#[tokio::test]
async fn report_progress_tool_noop_without_sink() {
    use crate::tools::AgentTool;

    let tool = super::tool::ReportProgressTool::new();
    let result = tool
        .execute("call-1", serde_json::json!({ "message": "anyone there?" }))
        .await
        .expect("no-sink call must not fail");
    assert!(
        result.content[0]
            .text
            .contains("Progress reporting not available"),
        "got: {}",
        result.content[0].text
    );

    // Missing/empty message is also a graceful no-op.
    let result = tool
        .execute("call-2", serde_json::json!({}))
        .await
        .expect("empty-args call must not fail");
    assert!(result.content[0].text.contains("No `message` provided"));
}
