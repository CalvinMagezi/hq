//! The ledger records what a call really consumed, once, when it ends.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio_stream::{Stream, StreamExt};

use crate::outcome_sink::{OutcomeEvent, TaskOutcomeSink, origin, with_origin};
use crate::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};
use hq_core::types::{ChatMessage, MessageRole};

use super::LlmRouter;

const PRICED_MODEL: &str = "anthropic/claude-haiku-5.5";
const UNPRICED_MODEL: &str = "nobody/never-heard-of-it";
const SETTLE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct CaptureSink(Mutex<Vec<OutcomeEvent>>);

#[async_trait]
impl TaskOutcomeSink for CaptureSink {
    async fn record(&self, event: OutcomeEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl CaptureSink {
    async fn settled(&self, expected: usize) -> Vec<OutcomeEvent> {
        let deadline = tokio::time::Instant::now() + SETTLE_TIMEOUT;
        while self.0.lock().unwrap().len() < expected && tokio::time::Instant::now() < deadline {
            tokio::task::yield_now().await;
        }
        self.0.lock().unwrap().clone()
    }
}

struct Cached;

fn assistant(content: &str) -> ChatMessage {
    ChatMessage {
        image_parts: Vec::new(),
        role: MessageRole::Assistant,
        content: content.into(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    }
}

#[async_trait]
impl LlmProvider for Cached {
    fn name(&self) -> &str {
        "openrouter"
    }

    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        Ok(ChatResponse {
            message: assistant("hi"),
            input_tokens: 1000,
            output_tokens: 50,
            cache_read_tokens: 900,
            cache_write_tokens: 0,
            model: request.model.clone(),
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        let chunks = vec![
            Ok(StreamChunk::Text("hi".into())),
            Ok(StreamChunk::Usage {
                input_tokens: 1000,
                output_tokens: 50,
                cache_read_tokens: 900,
                cache_write_tokens: 0,
            }),
            Ok(StreamChunk::ModelInfo(request.model.clone())),
            Ok(StreamChunk::Done),
        ];
        Ok(Box::pin(tokio_stream::iter(chunks)))
    }
}

fn router_with(sink: &Arc<CaptureSink>) -> LlmRouter {
    let mut router = LlmRouter::new();
    router.add_provider("openrouter", Arc::new(Cached));
    router.set_outcome_sink(sink.clone());
    router
}

fn request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_streamed_call_is_recorded_once_with_its_usage_and_cache_split() {
    let sink = Arc::new(CaptureSink::default());
    let router = router_with(&sink);

    let mut stream = with_origin(origin::MEMORY, router.chat_stream(&request(PRICED_MODEL)))
        .await
        .unwrap();
    while stream.next().await.is_some() {}

    let rows = sink.settled(1).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(
        (row.input_tokens, row.output_tokens, row.cache_read_tokens),
        (Some(1000), Some(50), 900)
    );
    assert_eq!(
        (row.cost_source, row.origin, row.success),
        ("table", origin::MEMORY, true)
    );
    assert!(row.cost_usd > 0.0);
}

#[tokio::test]
async fn the_ledger_and_the_session_price_a_cached_call_identically() {
    let sink = Arc::new(CaptureSink::default());
    let router = router_with(&sink);
    let mut stream = router.chat_stream(&request(PRICED_MODEL)).await.unwrap();
    while stream.next().await.is_some() {}

    let usage = crate::cost::Usage {
        input: 1000,
        output: 50,
        cache_read: 900,
        ..Default::default()
    };
    let session = crate::cost::price_call(
        crate::cost::ProviderClass::Metered,
        PRICED_MODEL,
        &usage,
        None,
    )
    .usd;
    assert_eq!(sink.settled(1).await[0].cost_usd, session);
}

#[tokio::test]
async fn a_stream_dropped_early_is_recorded_as_cancelled_not_as_a_provider_failure() {
    let sink = Arc::new(CaptureSink::default());
    let router = router_with(&sink);
    let mut stream = router.chat_stream(&request(PRICED_MODEL)).await.unwrap();
    let _ = stream.next().await;
    drop(stream);

    let rows = sink.settled(1).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].success,
            rows[0].error_class.as_deref(),
            rows[0].cost_source
        ),
        (true, Some("cancelled"), "unpriced")
    );
}

#[tokio::test]
async fn an_unknown_model_is_recorded_as_unpriced_not_free() {
    let sink = Arc::new(CaptureSink::default());
    let router = router_with(&sink);
    let mut stream = router.chat_stream(&request(UNPRICED_MODEL)).await.unwrap();
    while stream.next().await.is_some() {}

    assert_eq!(sink.settled(1).await[0].cost_source, "unpriced");
}

#[tokio::test]
async fn a_buffered_call_carries_its_cache_tokens_too() {
    let sink = Arc::new(CaptureSink::default());
    let router = router_with(&sink);
    router.chat(&request(PRICED_MODEL)).await.unwrap();

    let rows = sink.settled(1).await;
    assert_eq!(
        (rows[0].cache_read_tokens, rows[0].cost_source),
        (900, "table")
    );
}

#[tokio::test]
async fn a_call_nobody_scoped_is_counted() {
    let sink = Arc::new(CaptureSink::default());
    let router = router_with(&sink);
    let before = crate::outcome_sink::unscoped_calls();
    router.chat(&request(PRICED_MODEL)).await.unwrap();
    assert!(crate::outcome_sink::unscoped_calls() > before);
    assert_eq!(sink.settled(1).await[0].origin, origin::UNKNOWN);
}

struct Down;

#[async_trait]
impl LlmProvider for Down {
    fn name(&self) -> &str {
        "down"
    }

    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("503 service unavailable")
    }

    async fn chat_stream(
        &self,
        _request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        anyhow::bail!("503 service unavailable")
    }
}

#[tokio::test]
async fn a_chained_call_is_filed_under_the_backend_that_answered() {
    use crate::backend_chain::ChainProvider;
    let chain = ChainProvider::from_links(vec![
        ("primary", Arc::new(Down), PRICED_MODEL),
        ("fallback", Arc::new(Cached), PRICED_MODEL),
    ]);
    let sink = Arc::new(CaptureSink::default());
    let mut router = LlmRouter::new();
    router.add_provider("backends", Arc::new(chain));
    router.add_route("*", "backends", "", crate::router::CostTier::Budget);
    router.set_outcome_sink(sink.clone());

    router
        .chat(&request("alias-the-chain-ignores"))
        .await
        .unwrap();

    let rows = sink.settled(1).await;
    assert_eq!(
        (rows[0].provider.as_str(), rows[0].cost_source),
        ("fallback", "table")
    );
}

#[tokio::test]
async fn a_stream_cancelled_after_usage_arrived_is_still_billed() {
    let sink = Arc::new(CaptureSink::default());
    let router = router_with(&sink);
    let mut stream = router.chat_stream(&request(PRICED_MODEL)).await.unwrap();
    // Text, then the usage chunk; the reader leaves before Done.
    let _ = stream.next().await;
    let _ = stream.next().await;
    drop(stream);

    let rows = sink.settled(1).await;
    assert_eq!(
        (rows[0].success, rows[0].cost_source, rows[0].cache_read_tokens),
        (true, "table", 900)
    );
    assert!(rows[0].cost_usd > 0.0);
}

struct NoUsageLocal;

#[async_trait]
impl LlmProvider for NoUsageLocal {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        anyhow::bail!("unused")
    }

    async fn chat_stream(
        &self,
        _request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        let chunks = vec![Ok(StreamChunk::Text("hi".into())), Ok(StreamChunk::Done)];
        Ok(Box::pin(tokio_stream::iter(chunks)))
    }
}

#[tokio::test]
async fn a_local_stream_with_no_usage_is_free_not_unpriced() {
    let sink = Arc::new(CaptureSink::default());
    let mut router = LlmRouter::new();
    router.add_provider("ollama", Arc::new(NoUsageLocal));
    router.set_outcome_sink(sink.clone());

    let mut stream = router.chat_stream(&request("gemma4:e4b")).await.unwrap();
    while stream.next().await.is_some() {}

    assert_eq!(sink.settled(1).await[0].cost_source, "free");
}
