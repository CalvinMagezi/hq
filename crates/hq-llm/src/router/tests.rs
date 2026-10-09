use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;
use async_trait::async_trait;

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};
use hq_core::types::{ChatMessage, MessageRole};
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

use super::LlmRouter;
use super::health::ProviderHealth;
use super::strategy::response_to_stream;
use super::types::{CostTier, TaskHint};

use crate::provider::LlmError;

struct MockProvider {
    name: String,
    fail_count: AtomicU32,
    fail_error: String,
    call_count: AtomicU32,
}

impl MockProvider {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            fail_count: AtomicU32::new(0),
            fail_error: "mock error".to_string(),
            call_count: AtomicU32::new(0),
        }
    }

    fn failing(name: &str, fail_count: u32, error: &str) -> Self {
        Self {
            name: name.to_string(),
            fail_count: AtomicU32::new(fail_count),
            fail_error: error.to_string(),
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        self.call_count.fetch_add(1, AtomicOrdering::Relaxed);
        let remaining = self.fail_count.load(AtomicOrdering::Relaxed);
        if remaining > 0 {
            self.fail_count.fetch_sub(1, AtomicOrdering::Relaxed);
            bail!("{}", self.fail_error)
        }
        Ok(ChatResponse {
            message: ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Assistant,
                content: format!("response from {}", self.name),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            },
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            provider_cost_usd: None,
            model: format!("{}-model", self.name),
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn tokio_stream::Stream<Item = anyhow::Result<StreamChunk>> + Send>>>
    {
        let resp = self.chat(request).await?;
        Ok(response_to_stream(resp))
    }
}

#[derive(Clone)]
enum MockErrorKind {
    RateLimit(Option<Duration>),
    Auth(u16),
}

impl MockErrorKind {
    fn to_llm_error(&self) -> LlmError {
        match self {
            MockErrorKind::RateLimit(d) => LlmError::RateLimit { retry_after: *d },
            MockErrorKind::Auth(status) => LlmError::Auth {
                status: *status,
                message: "invalid key".to_string(),
            },
        }
    }
}

struct ErrorProvider {
    name: String,
    error_kind: MockErrorKind,
    call_count: AtomicU32,
}

impl ErrorProvider {
    fn rate_limited(name: &str, retry_after: Option<Duration>) -> Self {
        Self {
            name: name.to_string(),
            error_kind: MockErrorKind::RateLimit(retry_after),
            call_count: AtomicU32::new(0),
        }
    }

    fn auth_error(name: &str) -> Self {
        Self {
            name: name.to_string(),
            error_kind: MockErrorKind::Auth(401),
            call_count: AtomicU32::new(0),
        }
    }

    /// Mirrors DeepSeek's actual failure mode: HTTP 402 "insufficient credits",
    /// mapped by `openai_compat.rs`'s `chat()` to `LlmError::Auth { status: 402, .. }`.
    fn payment_required(name: &str) -> Self {
        Self {
            name: name.to_string(),
            error_kind: MockErrorKind::Auth(402),
            call_count: AtomicU32::new(0),
        }
    }

    fn calls(&self) -> u32 {
        self.call_count.load(AtomicOrdering::Relaxed)
    }
}

#[async_trait]
impl LlmProvider for ErrorProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        self.call_count.fetch_add(1, AtomicOrdering::Relaxed);
        Err(self.error_kind.to_llm_error().into())
    }

    async fn chat_stream(
        &self,
        _request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn tokio_stream::Stream<Item = anyhow::Result<StreamChunk>> + Send>>>
    {
        self.call_count.fetch_add(1, AtomicOrdering::Relaxed);
        Err(self.error_kind.to_llm_error().into())
    }
}

fn make_request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.to_string(),
        messages: vec![ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: "hello".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }],
        tools: vec![],
        temperature: None,
        max_tokens: None,
    }
}

#[tokio::test]
async fn test_basic_routing() {
    let mut router = LlmRouter::new();
    let provider = Arc::new(MockProvider::new("test")) as Arc<dyn LlmProvider>;
    router.add_provider("test", provider);
    router.add_route("hello", "test", "actual-model", CostTier::Free);

    let resp = router.chat(&make_request("hello")).await.unwrap();
    assert_eq!(resp.message.content, "response from test");
}

#[tokio::test]
async fn test_wildcard_routing() {
    let mut router = LlmRouter::new();
    let provider = Arc::new(MockProvider::new("cerebras")) as Arc<dyn LlmProvider>;
    router.add_provider("cerebras", provider);
    router.add_route("cerebras/*", "cerebras", "", CostTier::Free);

    let resp = router
        .chat(&make_request("cerebras/llama3.1-8b"))
        .await
        .unwrap();
    assert_eq!(resp.message.content, "response from cerebras");
}

#[tokio::test]
async fn test_provider_prefix_routing() {
    let mut router = LlmRouter::new();
    let provider = Arc::new(MockProvider::new("groq")) as Arc<dyn LlmProvider>;
    router.add_provider("groq", provider);

    let resp = router.chat(&make_request("groq/some-model")).await.unwrap();
    assert_eq!(resp.message.content, "response from groq");
}

#[tokio::test]
async fn test_free_provider_preferred_for_bulk_tasks() {
    let mut router = LlmRouter::new();
    let free_provider = Arc::new(MockProvider::new("free_p")) as Arc<dyn LlmProvider>;
    let paid_provider = Arc::new(MockProvider::new("paid_p")) as Arc<dyn LlmProvider>;

    router.add_provider("free_p", free_provider);
    router.add_provider("paid_p", paid_provider);

    router.add_route("fast", "paid_p", "paid-model", CostTier::Standard);
    router.add_route("fast", "free_p", "free-model", CostTier::Free);

    let resp = router.chat(&make_request("fast")).await.unwrap();
    assert_eq!(resp.message.content, "response from free_p");
}

/// Regression test for the gemma-4-31b critic incident: a Cerebras response
/// that fences its JSON and truncates at the critic's token budget parses as
/// nothing, and unlike a dead provider it never fails over. Registration
/// order can't fix that (score-based selection ignores it); CostTier can.
/// Looped because `resolve_scored` adds up to ±2.5 jitter per candidate, so
/// a single call could pass by luck even if the tier gap were too small.
#[test]
fn groq_outranks_cerebras_on_critic_and_verify_at_current_tiers() {
    let mut router = LlmRouter::new();
    router.add_provider(
        "cerebras",
        Arc::new(MockProvider::new("cerebras")) as Arc<dyn LlmProvider>,
    );
    router.add_provider(
        "groq",
        Arc::new(MockProvider::new("groq")) as Arc<dyn LlmProvider>,
    );

    for alias in ["critic", "verify"] {
        router.add_route(alias, "cerebras", "gemma-4-31b", CostTier::Standard);
        router.add_route(alias, "groq", "llama-3.1-8b-instant", CostTier::Free);

        for _ in 0..50 {
            let candidates = router.resolve_scored(alias, TaskHint::Bulk);
            assert_eq!(
                candidates[0].provider_name, "groq",
                "groq should win the {alias} alias even under jitter"
            );
        }
    }
}

#[tokio::test]
async fn test_paid_provider_preferred_for_tooluse() {
    let mut router = LlmRouter::new();
    let free_provider = Arc::new(MockProvider::new("free_p")) as Arc<dyn LlmProvider>;
    let paid_provider = Arc::new(MockProvider::new("paid_p")) as Arc<dyn LlmProvider>;

    router.add_provider("free_p", free_provider);
    router.add_provider("paid_p", paid_provider);

    router.add_route("relay", "paid_p", "paid-model", CostTier::Standard);
    router.add_route("relay", "free_p", "free-model", CostTier::Free);

    let resp = router.chat(&make_request("relay")).await.unwrap();
    assert_eq!(resp.message.content, "response from paid_p");
}

#[tokio::test]
async fn test_failover_to_next_provider() {
    let mut router = LlmRouter::new();
    let failing =
        Arc::new(MockProvider::failing("primary", 1, "server error")) as Arc<dyn LlmProvider>;
    let backup = Arc::new(MockProvider::new("backup")) as Arc<dyn LlmProvider>;

    router.add_provider("primary", failing);
    router.add_provider("backup", backup);

    router.add_route("test", "primary", "model-a", CostTier::Free);
    router.add_route("test", "backup", "model-b", CostTier::Free);

    let resp = router.chat(&make_request("test")).await.unwrap();
    assert_eq!(resp.message.content, "response from backup");
}

#[tokio::test]
async fn test_health_recorded_on_success() {
    let mut router = LlmRouter::new();
    let provider = Arc::new(MockProvider::new("test")) as Arc<dyn LlmProvider>;
    router.add_provider("test", provider);
    router.add_route("hello", "test", "model", CostTier::Free);

    router.chat(&make_request("hello")).await.unwrap();

    let snapshot = router.health_snapshot();
    let (_, requests, failures, tokens, healthy, _latency) = &snapshot[0];
    assert_eq!(*requests, 1);
    assert_eq!(*failures, 0);
    assert_eq!(*tokens, 30); // 10 input + 20 output
    assert!(*healthy);
}

#[tokio::test]
async fn test_health_recorded_on_failure() {
    let mut router = LlmRouter::new();
    let provider =
        Arc::new(MockProvider::failing("test", 100, "always fails")) as Arc<dyn LlmProvider>;
    router.add_provider("test", provider);
    router.add_route("hello", "test", "model", CostTier::Free);

    let _ = router.chat(&make_request("hello")).await;

    let snapshot = router.health_snapshot();
    let (_, requests, failures, _, _, _) = &snapshot[0];
    assert_eq!(*requests, 1);
    assert_eq!(*failures, 1);
}

#[tokio::test]
async fn test_rate_limited_provider_skipped() {
    let mut router = LlmRouter::new();
    let rate_limited = Arc::new(ErrorProvider::rate_limited(
        "limited",
        Some(Duration::from_secs(60)),
    )) as Arc<dyn LlmProvider>;
    let backup = Arc::new(MockProvider::new("backup")) as Arc<dyn LlmProvider>;

    router.add_provider("limited", rate_limited);
    router.add_provider("backup", backup);

    router.add_route("test", "limited", "model-a", CostTier::Free);
    router.add_route("test", "backup", "model-b", CostTier::Free);

    let resp = router.chat(&make_request("test")).await.unwrap();
    assert_eq!(resp.message.content, "response from backup");

    let resp2 = router.chat(&make_request("test")).await.unwrap();
    assert_eq!(resp2.message.content, "response from backup");
}

#[tokio::test]
async fn test_auth_error_long_cooldown() {
    use std::time::Instant;

    let mut router = LlmRouter::new();
    let auth_err = Arc::new(ErrorProvider::auth_error("bad_key")) as Arc<dyn LlmProvider>;
    let backup = Arc::new(MockProvider::new("backup")) as Arc<dyn LlmProvider>;

    router.add_provider("bad_key", auth_err);
    router.add_provider("backup", backup);

    router.add_route("test", "bad_key", "model-a", CostTier::Free);
    router.add_route("test", "backup", "model-b", CostTier::Free);

    // Routing through the router cannot prove this: score jitter
    // (selection.rs:74) decides which provider is tried first, and once the
    // backup succeeds it stays ahead, so `bad_key` may never be exercised at
    // all. The claim under test is about how an auth error is *recorded*, so
    // record one directly and assert the cooldown it earns.
    let resp = router.chat(&make_request("test")).await.unwrap();
    assert!(!resp.message.content.is_empty());

    let mut health = router.health.lock().unwrap();
    let (_, h) = health.iter_mut().find(|(n, _)| n == "bad_key").unwrap();
    h.record_failure(
        TaskHint::Simple,
        &LlmError::Auth {
            status: 401,
            message: "bad_key".into(),
        },
    );
    assert!(h.is_cooling_down());
    // An auth error is not transient: the cooldown must be hours, not seconds.
    assert!(h.cooldown_until.unwrap() > Instant::now() + Duration::from_secs(3500));
}

/// Regression test for the DeepSeek 402 incident: `record_failure` correctly
/// classifies a 402 as `LlmError::Auth` and sets an hour-long `cooldown_until`
/// (both already proven by `test_auth_error_long_cooldown` above), but
/// `resolve_scored` only demoted a cooling-down provider's score to the back
/// of the list — it never excluded it from the candidate list outright. With
/// several healthy alternatives on the same alias (as "relay" has in
/// production: deepseek, novita, cerebras, groq, gemini, openai...), the
/// cooling-down provider was still tried, and failed, on *every single
/// request* for the full hour, instead of being skipped like a rate-limited
/// or overloaded provider is.
#[tokio::test]
async fn test_cooling_down_provider_excluded_from_candidates() {
    let mut router = LlmRouter::new();
    let deepseek = Arc::new(ErrorProvider::payment_required("deepseek"));
    let healthy = Arc::new(MockProvider::new("healthy"));

    router.add_provider("deepseek", deepseek.clone());
    router.add_provider("healthy", healthy);

    router.add_route("relay", "deepseek", "deepseek-chat", CostTier::Budget);
    router.add_route("relay", "healthy", "healthy-model", CostTier::Free);

    // First request: deepseek is tried (it's a live candidate), returns 402,
    // gets a 1-hour cooldown, and the router falls through to "healthy".
    let r1 = router.chat(&make_request("relay")).await.unwrap();
    assert_eq!(r1.message.content, "response from healthy");
    assert_eq!(
        deepseek.calls(),
        1,
        "deepseek should be tried once while healthy"
    );

    {
        let health = router.health.lock().unwrap();
        let (_, h) = health.iter().find(|(n, _)| n == "deepseek").unwrap();
        assert!(h.is_cooling_down(), "402 should have triggered a cooldown");
    }

    // Second request, still within the cooldown window: deepseek must be
    // excluded from resolve_scored's candidates outright, not merely
    // outscored, so it must NOT be called again.
    let r2 = router.chat(&make_request("relay")).await.unwrap();
    assert_eq!(r2.message.content, "response from healthy");
    assert_eq!(
        deepseek.calls(),
        1,
        "a cooling-down provider must not be retried on subsequent requests"
    );
}

#[test]
fn test_task_type_failure_affects_score() {
    let health = ProviderHealth::default();
    assert_eq!(health.task_success_rate(TaskHint::ToolUse), 0.5);

    let mut h = ProviderHealth::default();
    let error = LlmError::Other(anyhow::anyhow!("tool call failed"));
    h.record_failure(TaskHint::ToolUse, &error);
    h.record_failure(TaskHint::ToolUse, &error);
    h.record_success(TaskHint::Simple, 100, Duration::from_millis(200));

    assert!((h.task_success_rate(TaskHint::ToolUse) - 0.3).abs() < 0.01);
    assert!((h.task_success_rate(TaskHint::Simple) - 0.6).abs() < 0.01);
}

#[test]
fn test_task_hint_from_model_alias() {
    assert_eq!(
        TaskHint::from_request(&make_request("fast")),
        TaskHint::Bulk
    );
    assert_eq!(
        TaskHint::from_request(&make_request("plan")),
        TaskHint::Planning
    );
    assert_eq!(
        TaskHint::from_request(&make_request("code")),
        TaskHint::Coding
    );
    assert_eq!(
        TaskHint::from_request(&make_request("relay")),
        TaskHint::ToolUse
    );
}

#[test]
fn test_task_hint_from_tools() {
    use hq_core::types::ToolDefinition;

    let mut req = make_request("some-model");
    req.tools = vec![ToolDefinition {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        parameters: serde_json::json!({}),
    }];
    assert_eq!(TaskHint::from_request(&req), TaskHint::ToolUse);
}

#[test]
fn test_task_hint_simple() {
    assert_eq!(
        TaskHint::from_request(&make_request("some-model")),
        TaskHint::Simple
    );
}

#[test]
fn test_score_free_beats_paid() {
    let free_score = LlmRouter::compute_score(CostTier::Free, false, None, TaskHint::Simple);
    let paid_score = LlmRouter::compute_score(CostTier::Premium, false, None, TaskHint::Simple);
    assert!(
        free_score > paid_score,
        "Free ({}) should score higher than Premium ({})",
        free_score,
        paid_score
    );
}

#[test]
fn test_score_healthy_beats_unhealthy() {
    let mut healthy = ProviderHealth::default();
    healthy.record_success(TaskHint::Simple, 100, Duration::from_millis(200));

    let mut unhealthy = ProviderHealth::default();
    for _ in 0..6 {
        unhealthy.record_failure(
            TaskHint::Simple,
            &LlmError::ServerError {
                status: 500,
                message: "error".into(),
            },
        );
    }

    let healthy_score =
        LlmRouter::compute_score(CostTier::Free, false, Some(&healthy), TaskHint::Simple);
    let unhealthy_score =
        LlmRouter::compute_score(CostTier::Free, false, Some(&unhealthy), TaskHint::Simple);
    assert!(
        healthy_score > unhealthy_score,
        "Healthy ({}) should score higher than unhealthy ({})",
        healthy_score,
        unhealthy_score
    );
}

#[test]
fn test_score_reliable_task_type_preferred() {
    let mut reliable = ProviderHealth::default();
    for _ in 0..10 {
        reliable.record_success(TaskHint::ToolUse, 100, Duration::from_millis(200));
    }

    let mut unreliable = ProviderHealth::default();
    for _ in 0..10 {
        unreliable.record_failure(
            TaskHint::ToolUse,
            &LlmError::Other(anyhow::anyhow!("tool fail")),
        );
    }

    let reliable_score =
        LlmRouter::compute_score(CostTier::Free, false, Some(&reliable), TaskHint::ToolUse);
    let unreliable_score =
        LlmRouter::compute_score(CostTier::Free, false, Some(&unreliable), TaskHint::ToolUse);
    assert!(
        reliable_score > unreliable_score,
        "Reliable ({}) should score higher than unreliable ({})",
        reliable_score,
        unreliable_score
    );
}

#[test]
fn test_score_cloud_free_beats_local_free() {
    let cloud_score = LlmRouter::compute_score(CostTier::Free, false, None, TaskHint::Bulk);
    let local_score = LlmRouter::compute_score(CostTier::Free, true, None, TaskHint::Bulk);
    assert!(
        cloud_score > local_score,
        "Cloud free ({}) should score higher than local free ({})",
        cloud_score,
        local_score
    );
}

#[test]
fn test_score_fast_provider_preferred() {
    let mut fast = ProviderHealth::default();
    fast.record_success(TaskHint::Bulk, 100, Duration::from_millis(100));

    let mut slow = ProviderHealth::default();
    slow.record_success(TaskHint::Bulk, 100, Duration::from_millis(10000));

    let fast_score = LlmRouter::compute_score(CostTier::Free, false, Some(&fast), TaskHint::Bulk);
    let slow_score = LlmRouter::compute_score(CostTier::Free, false, Some(&slow), TaskHint::Bulk);
    assert!(
        fast_score > slow_score,
        "Fast provider ({}) should score higher than slow ({})",
        fast_score,
        slow_score
    );
}

#[test]
fn test_health_cooldown() {
    use std::time::Instant;

    let mut h = ProviderHealth::default();
    assert!(!h.is_cooling_down());

    h.cooldown_until = Some(Instant::now() + Duration::from_secs(60));
    assert!(h.is_cooling_down());

    h.cooldown_until = Some(Instant::now() - Duration::from_secs(1));
    assert!(!h.is_cooling_down());
}

#[test]
fn test_health_success_resets_failures() {
    let mut h = ProviderHealth::default();
    let error = LlmError::ServerError {
        status: 500,
        message: "err".into(),
    };
    h.record_failure(TaskHint::Simple, &error);
    h.record_failure(TaskHint::Simple, &error);
    assert_eq!(h.consecutive_failures, 2);
    assert_eq!(h.backoff_exponent, 2);

    h.record_success(TaskHint::Simple, 100, Duration::from_millis(200));
    assert_eq!(h.consecutive_failures, 0);
    assert_eq!(h.backoff_exponent, 0);
    assert!(!h.is_cooling_down());
}

#[test]
fn test_health_exponential_backoff() {
    use std::time::Instant;

    let mut h = ProviderHealth::default();
    let error = LlmError::RateLimit { retry_after: None };

    h.record_failure(TaskHint::Simple, &error);
    assert!(h.is_cooling_down());
    assert_eq!(h.backoff_exponent, 1);

    h.cooldown_until = Some(Instant::now() - Duration::from_secs(1));
    h.record_failure(TaskHint::Simple, &error);
    assert_eq!(h.backoff_exponent, 2);

    h.cooldown_until = Some(Instant::now() - Duration::from_secs(1));
    h.record_failure(TaskHint::Simple, &error);
    assert_eq!(h.backoff_exponent, 3);
}

#[test]
fn test_health_auth_error_long_cooldown() {
    use std::time::Instant;

    let mut h = ProviderHealth::default();
    h.record_failure(
        TaskHint::Simple,
        &LlmError::Auth {
            status: 401,
            message: "bad key".into(),
        },
    );
    assert!(h.is_cooling_down());
    assert!(h.cooldown_until.unwrap() > Instant::now() + Duration::from_secs(3500));
}

#[tokio::test]
async fn test_all_providers_fail() {
    let mut router = LlmRouter::new();
    let p1 = Arc::new(MockProvider::failing("p1", 100, "fail")) as Arc<dyn LlmProvider>;
    let p2 = Arc::new(MockProvider::failing("p2", 100, "fail")) as Arc<dyn LlmProvider>;

    router.add_provider("p1", p1);
    router.add_provider("p2", p2);
    router.add_route("test", "p1", "model", CostTier::Free);
    router.add_route("test", "p2", "model", CostTier::Free);

    let result = router.chat(&make_request("test")).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("All LLM providers failed")
    );
}

#[tokio::test]
async fn test_empty_router_fails() {
    let router = LlmRouter::new();
    let result = router.chat(&make_request("any-model")).await;
    assert!(result.is_err());
}

#[test]
fn test_cost_tier_ordering() {
    assert!(CostTier::Free < CostTier::Budget);
    assert!(CostTier::Budget < CostTier::Standard);
    assert!(CostTier::Standard < CostTier::Premium);
}

#[tokio::test]
async fn test_streaming_with_fallback() {
    use futures::StreamExt;

    let mut router = LlmRouter::new();
    let provider = Arc::new(MockProvider::new("test")) as Arc<dyn LlmProvider>;
    router.add_provider("test", provider);
    router.add_route("hello", "test", "model", CostTier::Free);

    let stream = router.chat_stream(&make_request("hello")).await.unwrap();
    let chunks: Vec<_> = stream.collect().await;
    assert!(!chunks.is_empty());

    let mut has_text = false;
    let mut has_done = false;
    for chunk in &chunks {
        match chunk.as_ref().unwrap() {
            StreamChunk::Text(t) if !t.is_empty() => has_text = true,
            StreamChunk::Done => has_done = true,
            _ => {}
        }
    }
    assert!(has_text);
    assert!(has_done);
}

#[tokio::test]
async fn test_score_distributes_after_failures() {
    let mut router = LlmRouter::new();

    let a = Arc::new(MockProvider::failing("a", 2, "rate limited")) as Arc<dyn LlmProvider>;
    let b = Arc::new(MockProvider::new("b")) as Arc<dyn LlmProvider>;
    let c = Arc::new(MockProvider::new("c")) as Arc<dyn LlmProvider>;

    router.add_provider("a", a);
    router.add_provider("b", b);
    router.add_provider("c", c);

    router.add_route("relay", "a", "model-a", CostTier::Free);
    router.add_route("relay", "b", "model-b", CostTier::Free);
    router.add_route("relay", "c", "model-c", CostTier::Free);

    // Selection adds ±2.5 jitter so equally-scored providers share load
    // (selection.rs:74). Asserting a *specific* healthy provider therefore
    // tests the RNG, not the router — the real invariant is that the failing
    // provider is avoided and a healthy one always answers.
    for _ in 0..3 {
        let resp = router.chat(&make_request("relay")).await.unwrap();
        assert!(
            resp.message.content == "response from b" || resp.message.content == "response from c",
            "a failing provider was served instead of a healthy one: {}",
            resp.message.content
        );
    }
}

#[tokio::test]
async fn test_self_correction_lifecycle() {
    let mut router = LlmRouter::new();

    let primary =
        Arc::new(MockProvider::failing("primary", 3, "rate limited 429")) as Arc<dyn LlmProvider>;
    let backup = Arc::new(MockProvider::new("backup")) as Arc<dyn LlmProvider>;

    router.add_provider("primary", primary.clone());
    router.add_provider("backup", backup.clone());

    router.add_route("test", "primary", "model-p", CostTier::Free);
    router.add_route("test", "backup", "model-b", CostTier::Budget);

    let r1 = router.chat(&make_request("test")).await.unwrap();
    assert_eq!(r1.message.content, "response from backup");

    {
        let health = router.health.lock().unwrap();
        let (_, h) = health.iter().find(|(n, _)| n == "primary").unwrap();
        assert_eq!(h.consecutive_failures, 1);
        assert!(h.is_cooling_down());
    }

    let r2 = router.chat(&make_request("test")).await.unwrap();
    assert_eq!(r2.message.content, "response from backup");

    let r3 = router.chat(&make_request("test")).await.unwrap();
    assert_eq!(r3.message.content, "response from backup");

    {
        let health = router.health.lock().unwrap();
        let (_, h) = health.iter().find(|(n, _)| n == "primary").unwrap();
        assert_eq!(h.consecutive_failures, 1);
        assert_eq!(h.total_failures, 1);
    }

    {
        let mut health = router.health.lock().unwrap();
        let (_, h) = health.iter_mut().find(|(n, _)| n == "primary").unwrap();
        h.cooldown_until = None;
    }

    let r4 = router.chat(&make_request("test")).await.unwrap();
    assert_eq!(r4.message.content, "response from backup");

    {
        let mut health = router.health.lock().unwrap();
        let (_, h) = health.iter_mut().find(|(n, _)| n == "primary").unwrap();
        h.cooldown_until = None;
    }

    let r5 = router.chat(&make_request("test")).await.unwrap();
    assert_eq!(r5.message.content, "response from backup");

    {
        let mut health = router.health.lock().unwrap();
        let (_, h) = health.iter_mut().find(|(n, _)| n == "primary").unwrap();
        h.cooldown_until = None;
    }

    let r6 = router.chat(&make_request("test")).await.unwrap();
    assert!(!r6.message.content.is_empty());

    {
        let health = router.health.lock().unwrap();
        let (_, h) = health.iter().find(|(n, _)| n == "primary").unwrap();
        assert!(!h.is_cooling_down());
    }

    {
        let mut health = router.health.lock().unwrap();
        let (_, h) = health.iter_mut().find(|(n, _)| n == "primary").unwrap();
        h.record_success(TaskHint::Simple, 100, Duration::from_millis(200));
        assert_eq!(h.consecutive_failures, 0);
        assert!(h.avg_latency_ms > 0.0);
    }
}

#[tokio::test]
async fn test_task_type_isolation() {
    let mut router = LlmRouter::new();

    let flaky = Arc::new(MockProvider::new("flaky")) as Arc<dyn LlmProvider>;
    let reliable = Arc::new(MockProvider::new("reliable")) as Arc<dyn LlmProvider>;

    router.add_provider("flaky", flaky);
    router.add_provider("reliable", reliable);

    router.add_route("relay", "flaky", "model-f", CostTier::Free);
    router.add_route("relay", "reliable", "model-r", CostTier::Free);
    router.add_route("simple", "flaky", "model-f", CostTier::Free);

    {
        let mut health = router.health.lock().unwrap();
        let (_, h) = health.iter_mut().find(|(n, _)| n == "flaky").unwrap();
        let error = LlmError::Other(anyhow::anyhow!("tool call parsing failed"));
        for _ in 0..4 {
            h.record_failure(TaskHint::ToolUse, &error);
        }
        h.consecutive_failures = 0;
        for _ in 0..5 {
            h.record_success(TaskHint::Simple, 50, Duration::from_millis(100));
        }
    }

    {
        let mut health = router.health.lock().unwrap();
        let (_, h) = health.iter_mut().find(|(n, _)| n == "reliable").unwrap();
        for _ in 0..5 {
            h.record_success(TaskHint::ToolUse, 50, Duration::from_millis(100));
        }
    }

    let r1 = router.chat(&make_request("relay")).await.unwrap();
    assert_eq!(r1.message.content, "response from reliable");

    let r2 = router.chat(&make_request("simple")).await.unwrap();
    assert_eq!(r2.message.content, "response from flaky");
}

#[test]
fn test_jitter_distributes_scoring() {
    let mut router = LlmRouter::new();
    let a = Arc::new(MockProvider::new("a")) as Arc<dyn LlmProvider>;
    let b = Arc::new(MockProvider::new("b")) as Arc<dyn LlmProvider>;
    let c = Arc::new(MockProvider::new("c")) as Arc<dyn LlmProvider>;

    router.add_provider("a", a);
    router.add_provider("b", b);
    router.add_provider("c", c);

    router.add_route("test", "a", "model-a", CostTier::Free);
    router.add_route("test", "b", "model-b", CostTier::Free);
    router.add_route("test", "c", "model-c", CostTier::Free);

    {
        let mut health = router.health.lock().unwrap();
        for (_, h) in health.iter_mut() {
            for _ in 0..5 {
                h.record_success(TaskHint::Simple, 10, Duration::from_millis(200));
            }
        }
    }

    let mut winners: std::collections::HashSet<String> = std::collections::HashSet::new();
    for _ in 0..100 {
        let candidates = router.resolve_scored("test", TaskHint::Simple);
        if let Some(top) = candidates.first() {
            winners.insert(top.provider_name.clone());
        }
    }

    assert!(
        winners.len() >= 2,
        "Jitter should cause different providers to win, but only {:?} won",
        winners
    );
}

#[test]
fn test_sliding_window_forgets_old_failures() {
    let mut h = ProviderHealth::default();
    let error = LlmError::Other(anyhow::anyhow!("fail"));

    for _ in 0..10 {
        h.record_failure(TaskHint::ToolUse, &error);
    }
    h.consecutive_failures = 0;
    assert_eq!(h.task_success_rate(TaskHint::ToolUse), 0.0);

    for _ in 0..20 {
        h.record_success(TaskHint::ToolUse, 10, Duration::from_millis(100));
    }

    assert_eq!(
        h.task_success_rate(TaskHint::ToolUse),
        1.0,
        "After 20 successes, old failures should be forgotten"
    );
}

#[test]
fn test_sliding_window_size_capped() {
    use super::types::RELIABILITY_WINDOW;

    let mut h = ProviderHealth::default();
    for _ in 0..30 {
        h.record_success(TaskHint::Simple, 10, Duration::from_millis(100));
    }
    assert_eq!(
        h.task_window[TaskHint::Simple as usize].len(),
        RELIABILITY_WINDOW
    );
}

#[test]
fn test_daily_budget_penalty_in_scoring() {
    use std::time::Instant;

    let h = ProviderHealth {
        daily_token_limit: 1000,
        daily_reset_at: Some(Instant::now() + Duration::from_secs(86400)),
        ..Default::default()
    };

    let mut h_low = h.clone();
    h_low.daily_tokens_used = 700;
    let score_low = LlmRouter::compute_score(CostTier::Free, false, Some(&h_low), TaskHint::Simple);

    let mut h_high = h.clone();
    h_high.daily_tokens_used = 900;
    let score_high =
        LlmRouter::compute_score(CostTier::Free, false, Some(&h_high), TaskHint::Simple);

    assert!(
        score_low > score_high,
        "Score at 70% budget ({}) should be higher than at 90% ({})",
        score_low,
        score_high
    );
}

#[test]
fn test_daily_budget_exhausted_reads_as_full() {
    use std::time::Instant;

    let h = ProviderHealth {
        daily_token_limit: 1000,
        daily_tokens_used: 1100,
        daily_reset_at: Some(Instant::now() + Duration::from_secs(86400)),
        ..Default::default()
    };

    // An exhausted cap only lowers the score (see the penalty test above);
    // selection never excludes the provider for it.
    assert!(h.daily_budget_ratio() >= 1.0);
}

#[test]
fn test_daily_budget_resets_after_period() {
    use std::time::Instant;

    let h = ProviderHealth {
        daily_token_limit: 1000,
        daily_tokens_used: 1100,
        daily_reset_at: Some(Instant::now() - Duration::from_secs(1)),
        ..Default::default()
    };

    assert_eq!(h.daily_budget_ratio(), 0.0);
}

#[test]
fn a_single_candidate_is_never_scored() {
    let mut router = LlmRouter::new();
    router.add_provider(
        "backends",
        Arc::new(MockProvider::new("backends")) as Arc<dyn LlmProvider>,
    );
    router.add_route("*", "backends", "", CostTier::Standard);
    for _ in 0..20 {
        let candidates = router.resolve_scored("anything", TaskHint::ToolUse);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].score, super::selection::UNSCORED);
    }
}

#[tokio::test]
async fn test_provider_prefix_uses_registered_tier() {
    let mut router = LlmRouter::new();
    let provider = Arc::new(MockProvider::new("groq")) as Arc<dyn LlmProvider>;
    router.add_provider("groq", provider);

    router.add_route("groq/*", "groq", "", CostTier::Free);

    let candidates = router.resolve_scored("groq/some-new-model", TaskHint::Simple);
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].cost_tier,
        CostTier::Free,
        "Provider prefix route should inherit tier from wildcard route"
    );
}

mod live_tests {
    use super::*;
    use crate::provider::{ChatRequest, LlmProvider, StreamChunk};
    use hq_core::types::{ChatMessage, MessageRole};
    use std::collections::HashMap;

    fn make_live_request(model: &str) -> ChatRequest {
        ChatRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: "Reply with exactly one word: hello".to_string(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            }],
            tools: vec![],
            temperature: Some(0.0),
            max_tokens: Some(10),
        }
    }

    #[tokio::test]
    #[ignore] // requires CEREBRAS_API_KEY and GROQ_API_KEY
    async fn test_live_router_from_env() {
        let router = LlmRouter::from_env();
        assert!(
            router.provider_count() >= 2,
            "Need at least Cerebras + Groq for live test"
        );

        let resp = router.chat(&make_live_request("fast")).await.unwrap();
        assert!(!resp.message.content.is_empty());
        assert!(resp.input_tokens > 0);
        assert!(resp.output_tokens > 0);
        eprintln!(
            "[live] fast: model={}, tokens={}/{}",
            resp.model, resp.input_tokens, resp.output_tokens
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_specific_providers() {
        let router = LlmRouter::from_env();

        if router.providers.iter().any(|(n, _)| n == "cerebras") {
            let resp = router
                .chat(&make_live_request("cerebras/llama3.1-8b"))
                .await
                .unwrap();
            assert!(!resp.message.content.is_empty());
            eprintln!("[live] cerebras: model={}", resp.model);
        }

        if router.providers.iter().any(|(n, _)| n == "groq") {
            match router
                .chat(&make_live_request("groq/llama-3.1-8b-instant"))
                .await
            {
                Ok(resp) => {
                    assert!(!resp.message.content.is_empty());
                    eprintln!("[live] groq: model={}", resp.model);
                }
                Err(e) => {
                    eprintln!(
                        "[live] groq: expected failure (model may be unavailable): {}",
                        e
                    );
                }
            }
        }
    }

    #[tokio::test]
    #[ignore] // requires KIMI_CODE_API_KEY (or a config.yaml kimi_code_api_key) — a real sk-kimi-* key
    async fn test_live_kimi_code() {
        let router = LlmRouter::from_env();
        assert!(
            router.providers.iter().any(|(n, _)| n == "kimi-code"),
            "kimi-code provider not registered — is kimi_code_api_key set in config.yaml \
             or KIMI_CODE_API_KEY in the environment, and does it start with sk-kimi-?"
        );

        // kimi-for-coding rejects any temperature but 1 (confirmed live,
        // 2026-07-26: "invalid temperature: only 1 is allowed for this
        // model"; it was 0.6 under K2.5). OpenRouterProvider forces the
        // accepted value for the kimi.com/coding base URL regardless of what
        // the caller sends, so this deliberately
        // uses make_live_request's default (0.0) — a value that would 400
        // without that fix — to prove callers no longer need to know about
        // the quirk (MemoryLlm, for one, hardcodes 0.3 for structured JSON).
        let req = make_live_request("kimi-code/kimi-for-coding");
        assert_eq!(
            req.temperature,
            Some(0.0),
            "sanity: test must send a temperature Kimi would reject unpatched"
        );

        let resp = router.chat(&req).await.unwrap();
        assert!(
            !resp.message.content.is_empty(),
            "kimi-for-coding returned empty content — thinking-mode-disable guard in \
             openai_compat.rs (keyed on api_base containing \"kimi\") may not be firing"
        );
        eprintln!(
            "[live] kimi-code: model={} content={:?}",
            resp.model, resp.message.content
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_streaming() {
        use futures::StreamExt;

        let router = LlmRouter::from_env();
        let stream = router
            .chat_stream(&make_live_request("fast"))
            .await
            .unwrap();

        let chunks: Vec<_> = stream.collect().await;
        assert!(!chunks.is_empty());

        let mut has_text = false;
        for chunk in &chunks {
            if let Ok(StreamChunk::Text(t)) = chunk
                && !t.is_empty()
            {
                has_text = true;
                eprintln!("[live] stream text: {}", t);
            }
        }
        assert!(has_text, "streaming should produce text");
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_health_tracking_after_requests() {
        let router = LlmRouter::from_env();

        for _ in 0..3 {
            let _ = router.chat(&make_live_request("fast")).await;
        }

        let snapshot = router.health_snapshot();
        let total_requests: u64 = snapshot.iter().map(|(_, r, _, _, _, _)| r).sum();
        assert!(
            total_requests >= 3,
            "Should have tracked at least 3 requests, got {}",
            total_requests
        );
        eprintln!("[live] health snapshot: {:?}", snapshot);
    }

    #[tokio::test]
    #[ignore]
    async fn test_live_failover_with_bad_model() {
        let router = LlmRouter::from_env();

        let mut req = make_live_request("fast");
        req.model = "mid".to_string();

        let resp = router.chat(&req).await.unwrap();
        assert!(!resp.message.content.is_empty());
        eprintln!("[live] mid alias: model={}", resp.model);
    }

    #[tokio::test]
    #[ignore]
    async fn test_battle_all_aliases() {
        let router = LlmRouter::from_env();
        let aliases = [
            "fast", "bulk", "mid", "relay", "premium", "plan", "code", "verify", "critic",
        ];

        eprintln!("\n=== BATTLE TEST: All aliases ===");
        eprintln!("Providers: {}", router.provider_count());
        eprintln!("Routes: {}", router.route_count());
        eprintln!();

        let mut results: Vec<(&str, String, u32, u32, bool)> = Vec::new();

        for alias in &aliases {
            let req = make_live_request(alias);
            let start = std::time::Instant::now();
            match router.chat(&req).await {
                Ok(resp) => {
                    let elapsed = start.elapsed();
                    eprintln!(
                        "  {} -> model={} tokens={}/{} time={}ms",
                        alias,
                        resp.model,
                        resp.input_tokens,
                        resp.output_tokens,
                        elapsed.as_millis()
                    );
                    results.push((
                        alias,
                        resp.model,
                        resp.input_tokens,
                        resp.output_tokens,
                        true,
                    ));
                }
                Err(e) => {
                    eprintln!("  {} -> FAILED: {}", alias, e);
                    results.push((alias, "FAILED".into(), 0, 0, false));
                }
            }
        }

        eprintln!("\n=== HEALTH SNAPSHOT ===");
        for (name, reqs, fails, tokens, healthy, latency) in router.health_snapshot() {
            eprintln!(
                "  {} reqs={} fails={} tokens={} healthy={} avg_latency={}ms",
                name, reqs, fails, tokens, healthy, latency as u64
            );
        }

        let successes = results.iter().filter(|r| r.4).count();
        assert!(
            successes >= aliases.len() / 2,
            "Only {}/{} aliases succeeded",
            successes,
            aliases.len()
        );

        eprintln!(
            "\n=== {}/{} aliases succeeded ===\n",
            successes,
            aliases.len()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_battle_rapid_fire() {
        let router = LlmRouter::from_env();
        let rounds = 15;

        eprintln!(
            "\n=== BATTLE TEST: {} rapid-fire 'fast' requests ===",
            rounds
        );

        let mut successes = 0;
        let mut providers_used: HashMap<String, u32> = HashMap::new();

        for i in 0..rounds {
            let req = make_live_request("fast");
            let start = std::time::Instant::now();
            match router.chat(&req).await {
                Ok(resp) => {
                    let elapsed = start.elapsed();
                    *providers_used.entry(resp.model.clone()).or_default() += 1;
                    eprintln!(
                        "  [{}/{}] model={} time={}ms",
                        i + 1,
                        rounds,
                        resp.model,
                        elapsed.as_millis()
                    );
                    successes += 1;
                }
                Err(e) => {
                    eprintln!("  [{}/{}] FAILED: {}", i + 1, rounds, e);
                }
            }
        }

        eprintln!("\n=== HEALTH AFTER RAPID-FIRE ===");
        for (name, reqs, fails, tokens, healthy, latency) in router.health_snapshot() {
            eprintln!(
                "  {} reqs={} fails={} tokens={} healthy={} avg_latency={}ms",
                name, reqs, fails, tokens, healthy, latency as u64
            );
        }

        eprintln!("\n=== DISTRIBUTION ===");
        for (model, count) in &providers_used {
            eprintln!("  {} -> {} requests", model, count);
        }

        assert!(
            successes >= rounds / 2,
            "Only {}/{} rapid-fire requests succeeded",
            successes,
            rounds
        );
        eprintln!("\n=== {}/{} rapid-fire succeeded ===\n", successes, rounds);
    }

    #[tokio::test]
    #[ignore]
    async fn test_battle_concurrent() {
        let router = Arc::new(LlmRouter::from_env());
        let concurrent = 5;

        eprintln!("\n=== BATTLE TEST: {} concurrent requests ===", concurrent);

        let mut handles = Vec::new();
        for i in 0..concurrent {
            let r = router.clone();
            let alias = match i % 3 {
                0 => "fast",
                1 => "mid",
                _ => "verify",
            };
            handles.push(tokio::spawn(async move {
                let req = make_live_request(alias);
                let start = std::time::Instant::now();
                let result = r.chat(&req).await;
                let elapsed = start.elapsed();
                (i, alias, result, elapsed)
            }));
        }

        let mut successes = 0;
        for handle in handles {
            let (i, alias, result, elapsed) = handle.await.unwrap();
            match result {
                Ok(resp) => {
                    eprintln!(
                        "  [{}] {} -> model={} time={}ms",
                        i,
                        alias,
                        resp.model,
                        elapsed.as_millis()
                    );
                    successes += 1;
                }
                Err(e) => {
                    eprintln!(
                        "  [{}] {} -> FAILED: {} time={}ms",
                        i,
                        alias,
                        e,
                        elapsed.as_millis()
                    );
                }
            }
        }

        eprintln!("\n=== HEALTH AFTER CONCURRENT ===");
        for (name, reqs, fails, tokens, healthy, latency) in router.health_snapshot() {
            eprintln!(
                "  {} reqs={} fails={} tokens={} healthy={} avg_latency={}ms",
                name, reqs, fails, tokens, healthy, latency as u64
            );
        }

        assert!(
            successes >= concurrent / 2,
            "Only {}/{} concurrent requests succeeded",
            successes,
            concurrent
        );
        eprintln!(
            "\n=== {}/{} concurrent succeeded ===\n",
            successes, concurrent
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_battle_rate_limit_recovery() {
        let router = LlmRouter::from_env();
        let rounds = 30;

        eprintln!(
            "\n=== BATTLE TEST: Rate limit stress ({} requests) ===",
            rounds
        );

        let mut providers_seen: Vec<String> = Vec::new();
        let mut successes = 0;
        let mut rate_limit_failovers = 0;

        for i in 0..rounds {
            let req = make_live_request("fast");
            let start = std::time::Instant::now();
            match router.chat(&req).await {
                Ok(resp) => {
                    let elapsed = start.elapsed();

                    if let Some(last) = providers_seen.last() {
                        if last != &resp.model {
                            rate_limit_failovers += 1;
                            eprintln!(
                                "  [{}/{}] FAILOVER {} -> {} time={}ms",
                                i + 1,
                                rounds,
                                last,
                                resp.model,
                                elapsed.as_millis()
                            );
                        } else if i % 5 == 0 {
                            eprintln!(
                                "  [{}/{}] {} time={}ms",
                                i + 1,
                                rounds,
                                resp.model,
                                elapsed.as_millis()
                            );
                        }
                    } else {
                        eprintln!(
                            "  [{}/{}] {} time={}ms",
                            i + 1,
                            rounds,
                            resp.model,
                            elapsed.as_millis()
                        );
                    }

                    providers_seen.push(resp.model);
                    successes += 1;
                }
                Err(e) => {
                    eprintln!("  [{}/{}] FAILED: {}", i + 1, rounds, e);
                }
            }
        }

        eprintln!("\n=== HEALTH AFTER STRESS ===");
        for (name, reqs, fails, tokens, healthy, latency) in router.health_snapshot() {
            eprintln!(
                "  {} reqs={} fails={} tokens={} healthy={} avg_latency={}ms",
                name, reqs, fails, tokens, healthy, latency as u64
            );
        }

        let mut unique_providers: Vec<String> = providers_seen.clone();
        unique_providers.sort();
        unique_providers.dedup();

        eprintln!("\n=== SUMMARY ===");
        eprintln!("  Successes: {}/{}", successes, rounds);
        eprintln!("  Failovers: {}", rate_limit_failovers);
        eprintln!("  Unique providers: {:?}", unique_providers);

        for p in &unique_providers {
            let count = providers_seen.iter().filter(|x| *x == p).count();
            eprintln!("  {} -> {} requests", p, count);
        }

        assert!(
            successes >= rounds * 2 / 3,
            "At least 2/3 of requests should succeed, got {}/{}",
            successes,
            rounds
        );
    }

    #[test]
    fn local_fallback_routes_point_at_a_pulled_model() {
        let router = LlmRouter::from_env();
        // ornith:latest is confirmed present via `ollama list` on this
        // deployment; qwen3:14b/qwen3:8b are not. When Ollama isn't reachable
        // (e.g. in CI), `from_env` adds no ollama routes and this loop is a
        // no-op, so the assertion only bites where it can be verified.
        let unpulled_patterns = ["relay", "code", "verify"];
        let local_fallback_routes: Vec<_> = router
            .routes
            .iter()
            .filter(|r| r.provider == "ollama" && unpulled_patterns.contains(&r.pattern.as_str()))
            .collect();
        for route in local_fallback_routes {
            assert_eq!(
                route.model_id, "ornith:latest",
                "route pattern {:?} should point at ornith:latest",
                route.pattern
            );
        }
    }

    /// Streaming previously skipped the Kimi body fixups entirely, so every
    /// streamed turn went out with the caller's temperature and was rejected.
    /// This sends a temperature the endpoint refuses to prove the clamp is
    /// applied on the streaming path too, not just the buffered one.
    #[tokio::test]
    #[ignore]
    async fn test_live_kimi_streaming_applies_the_temperature_clamp() {
        use tokio_stream::StreamExt;

        let router = LlmRouter::from_env();
        let mut req = make_live_request("kimi-code/k3-256k");
        req.temperature = Some(0.3);
        // k3 is a thinking model: reasoning tokens are drawn from the same
        // budget, so a tiny max_tokens is spent before any content is emitted.
        req.max_tokens = Some(256);

        let mut stream = router.chat_stream(&req).await.expect(
            "streaming failed — the Kimi temperature clamp is not applied on the streaming path",
        );
        let mut text = String::new();
        let mut saw_done = false;
        while let Some(chunk) = stream.next().await {
            match chunk.unwrap() {
                StreamChunk::Text(t) => text.push_str(&t),
                StreamChunk::Done => saw_done = true,
                _ => {}
            }
        }
        assert!(saw_done, "stream never terminated");
        assert!(!text.trim().is_empty(), "stream produced no text");
        eprintln!(
            "[live] kimi stream: {:?}",
            text.chars().take(80).collect::<String>()
        );
    }
}

/// With `backends:` configured, every alias (fast, bulk, a raw model id)
/// resolves to the chain, so background calls follow the configured primary.
#[test]
fn configured_backends_take_every_alias() {
    let config = hq_core::config::HqConfig {
        backends: serde_yaml::from_str(
            "primary: luna\nbackends:\n  - name: luna\n    kind: github-copilot-api\n    model: gpt-6-luna\n",
        )
        .unwrap(),
        ..Default::default()
    };
    let router = LlmRouter::from_backends(&config).expect("chain configured");
    for alias in [
        "fast",
        "bulk",
        "verify",
        "meta-llama/llama-3.3-70b-instruct:free",
    ] {
        let candidates = router.resolve_scored(alias, TaskHint::Bulk);
        assert_eq!(candidates.len(), 1, "{alias}");
        assert_eq!(candidates[0].provider_name, "backends");
    }
    assert!(LlmRouter::from_backends(&hq_core::config::HqConfig::default()).is_none());
}

struct EchoModelProvider;

#[async_trait]
impl LlmProvider for EchoModelProvider {
    fn name(&self) -> &str {
        "openrouter"
    }

    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            message: ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Assistant,
                content: request.model.clone(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            },
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            provider_cost_usd: None,
            model: request.model.clone(),
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn tokio_stream::Stream<Item = anyhow::Result<StreamChunk>> + Send>>>
    {
        Ok(response_to_stream(self.chat(request).await?))
    }
}

#[tokio::test]
async fn streaming_falls_back_to_the_only_provider_for_an_unrouted_vendor_model() {
    use tokio_stream::StreamExt;
    let mut router = LlmRouter::new();
    router.add_provider("openrouter", Arc::new(EchoModelProvider));

    // A fresh install's default model: no route, and no provider is named "anthropic".
    let mut stream = router
        .chat_stream(&make_request("anthropic/claude-sonnet-4"))
        .await
        .unwrap();
    let first = stream.next().await.unwrap().unwrap();
    assert!(format!("{first:?}").contains("anthropic/claude-sonnet-4"));
}

#[tokio::test]
async fn streaming_with_no_providers_reports_that_none_is_configured() {
    let router = LlmRouter::new();
    let err = router
        .chat_stream(&make_request("anthropic/claude-sonnet-4"))
        .await
        .err()
        .expect("no providers must error");
    assert!(err.to_string().contains("none is configured"));
}

#[tokio::test]
async fn fallback_keeps_a_vendor_model_name_even_when_the_provider_has_an_alias_route() {
    let mut router = LlmRouter::new();
    router.add_provider("openrouter", Arc::new(EchoModelProvider));
    router.add_route(
        "relay",
        "openrouter",
        "some/alias-target",
        CostTier::Standard,
    );

    let resp = router
        .chat(&make_request("anthropic/claude-sonnet-4"))
        .await
        .unwrap();
    assert_eq!(resp.message.content, "anthropic/claude-sonnet-4");
}

struct FailsOnFirstChunk;

#[async_trait]
impl LlmProvider for FailsOnFirstChunk {
    fn name(&self) -> &str {
        "cerebras"
    }

    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        bail!("404 Not Found")
    }

    async fn chat_stream(
        &self,
        _request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn tokio_stream::Stream<Item = anyhow::Result<StreamChunk>> + Send>>>
    {
        // Opens fine, then fails on the first chunk, as a 404 for an unknown model does.
        Ok(Box::pin(tokio_stream::once(Err(anyhow::anyhow!(
            "404 Not Found"
        )))))
    }
}

#[tokio::test]
async fn streaming_fallback_skips_a_provider_whose_stream_fails_on_the_first_chunk() {
    use tokio_stream::StreamExt;
    let mut router = LlmRouter::new();
    router.add_provider("cerebras", Arc::new(FailsOnFirstChunk));
    router.add_provider("openrouter", Arc::new(EchoModelProvider));

    let mut stream = router
        .chat_stream(&make_request("anthropic/claude-sonnet-4"))
        .await
        .unwrap();
    let first = stream.next().await.unwrap().unwrap();
    assert!(format!("{first:?}").contains("anthropic/claude-sonnet-4"));
}
