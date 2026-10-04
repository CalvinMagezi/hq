use super::*;
use hq_core::types::ToolDefinition;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A scripted backend for chain tests.
struct ScriptBackend {
    name: String,
    capabilities: BackendCapabilities,
    /// Error returned from `start()` itself (simulates a startup failure).
    start_error: Option<BackendError>,
    /// Events yielded by the stream when `start()` succeeds.
    events: Vec<Result<BackendEvent, BackendError>>,
    /// Counts how many times `start()` was invoked.
    started: Arc<AtomicUsize>,
}

impl ScriptBackend {
    fn stream(name: &str, events: Vec<Result<BackendEvent, BackendError>>) -> Self {
        Self {
            name: name.to_string(),
            capabilities: BackendCapabilities::full_api(),
            start_error: None,
            events,
            started: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn start_failure(name: &str, err: BackendError) -> Self {
        Self {
            name: name.to_string(),
            capabilities: BackendCapabilities::full_api(),
            start_error: Some(err),
            events: Vec::new(),
            started: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn with_capabilities(mut self, caps: BackendCapabilities) -> Self {
        self.capabilities = caps;
        self
    }

    fn counter(&self) -> Arc<AtomicUsize> {
        self.started.clone()
    }
}

#[async_trait]
impl SessionBackend for ScriptBackend {
    fn name(&self) -> &str {
        &self.name
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
    }
    async fn start(&self, _request: &BackendRequest) -> Result<BackendEventStream, BackendError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        if let Some(err) = &self.start_error {
            return Err(err.clone());
        }
        Ok(Box::pin(tokio_stream::iter(self.events.clone())))
    }
}

fn req() -> BackendRequest {
    BackendRequest {
        stream: true,
        ..Default::default()
    }
}

fn req_with_tools() -> BackendRequest {
    BackendRequest {
        tools: vec![ToolDefinition {
            name: "t".into(),
            description: "d".into(),
            parameters: serde_json::json!({}),
        }],
        ..Default::default()
    }
}

async fn drain(stream: BackendEventStream) -> Vec<Result<BackendEvent, BackendError>> {
    stream.collect().await
}

#[test]
fn root_capabilities_track_the_primary_not_the_union() {
    // Primary is a buffered CLI (no tools/streaming); fallback is a full API.
    let cli = ScriptBackend::stream("cli", vec![Ok(BackendEvent::Done)])
        .with_capabilities(BackendCapabilities::buffered_cli());
    let api = ScriptBackend::stream("api", vec![Ok(BackendEvent::Done)])
        .with_capabilities(BackendCapabilities::full_api());
    let chain = ProviderChain::new("chain", vec![Arc::new(cli), Arc::new(api)]);

    // The union advertises tools+streaming (the API can),
    assert!(chain.capabilities().tools);
    assert!(chain.capabilities().streaming);
    // but the *root* selection follows the CLI primary: no tools, no stream.
    assert!(!chain.root_capabilities().tools);
    assert!(!chain.root_capabilities().streaming);
}

#[test]
fn cli_only_chain_has_no_utility_provider_but_owns_failover() {
    // A pure-CLI chain exposes no borrowable provider (the session then routes
    // compaction through the backend), yet still owns its own failover so the
    // session must not wrap it in legacy retries.
    let cli = ScriptBackend::stream("cli", vec![Ok(BackendEvent::Done)])
        .with_capabilities(BackendCapabilities::buffered_cli());
    let chain = ProviderChain::new("cli-only", vec![Arc::new(cli)]);
    assert!(chain.utility_provider().is_none());
    assert!(chain.owns_failover());
}

/// Counts calls and answers, or fails with a 503 when `fail` is set.
struct CountingProvider {
    fail: bool,
    calls: AtomicUsize,
}

#[async_trait]
impl hq_llm::provider::LlmProvider for CountingProvider {
    fn name(&self) -> &str {
        "counting"
    }

    async fn chat(
        &self,
        request: &hq_llm::provider::ChatRequest,
    ) -> anyhow::Result<hq_llm::provider::ChatResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(!self.fail, "503 service unavailable");
        Ok(hq_llm::provider::ChatResponse {
            message: hq_core::types::ChatMessage {
                role: hq_core::types::MessageRole::Assistant,
                content: "ok".into(),
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: None,
                image_parts: Vec::new(),
            },
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            model: request.model.clone(),
        })
    }

    async fn chat_stream(
        &self,
        _request: &hq_llm::provider::ChatRequest,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<
                dyn tokio_stream::Stream<Item = anyhow::Result<hq_llm::provider::StreamChunk>>
                    + Send,
            >,
        >,
    > {
        unimplemented!("not used by these tests")
    }
}

#[tokio::test]
async fn utility_provider_falls_back_when_the_first_api_backend_errors() {
    let counting = |fail| {
        Arc::new(CountingProvider {
            fail,
            calls: AtomicUsize::new(0),
        })
    };
    let (down, up) = (counting(true), counting(false));
    let chain = ProviderChain::new(
        "chain",
        vec![
            Arc::new(crate::backend::ApiBackend::new("down", down.clone())),
            Arc::new(crate::backend::ApiBackend::new("up", up.clone())),
        ],
    );

    let util = chain
        .utility_provider()
        .expect("api chain has a utility provider");
    let resp = util
        .chat(&hq_llm::provider::ChatRequest::default())
        .await
        .unwrap();

    assert_eq!(resp.message.content, "ok");
    assert_eq!(down.calls.load(Ordering::SeqCst), 1);
    assert_eq!(up.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn primary_used_and_fallback_untouched_when_primary_produces_output() {
    let primary = ScriptBackend::stream(
        "primary",
        vec![
            Ok(BackendEvent::TextDelta("hi".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let fallback = ScriptBackend::stream("fallback", vec![Ok(BackendEvent::Message("no".into()))]);
    let p_count = primary.counter();
    let f_count = fallback.counter();

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::TextDelta(t)) if t == "hi"))
    );
    assert_eq!(p_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        f_count.load(Ordering::SeqCst),
        0,
        "fallback must not be started"
    );
}

#[tokio::test]
async fn incompatible_primary_is_skipped_for_a_capable_fallback() {
    // Primary can't do tools; request requires tools → skip to fallback.
    let primary = ScriptBackend::stream("cli", vec![Ok(BackendEvent::Message("x".into()))])
        .with_capabilities(BackendCapabilities::buffered_cli());
    let fallback = ScriptBackend::stream(
        "api",
        vec![
            Ok(BackendEvent::Message("tools-ok".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let p_count = primary.counter();
    let f_count = fallback.counter();

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req_with_tools()).await.unwrap()).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::Message(m)) if m == "tools-ok"))
    );
    assert_eq!(
        p_count.load(Ordering::SeqCst),
        0,
        "incompatible primary never started"
    );
    assert_eq!(f_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn startup_failover_when_primary_start_errors() {
    let primary = ScriptBackend::start_failure("primary", BackendError::Transient("down".into()));
    let fallback = ScriptBackend::stream(
        "fallback",
        vec![
            Ok(BackendEvent::TextDelta("recovered".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let p_count = primary.counter();
    let f_count = fallback.counter();

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::TextDelta(t)) if t == "recovered"))
    );
    assert_eq!(p_count.load(Ordering::SeqCst), 1);
    assert_eq!(f_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn startup_failover_when_stream_errors_before_output() {
    let primary = ScriptBackend::stream(
        "primary",
        vec![Err(BackendError::Transient("mid-startup".into()))],
    );
    let fallback = ScriptBackend::stream(
        "fallback",
        vec![
            Ok(BackendEvent::Message("recovered".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let f_count = fallback.counter();

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::Message(m)) if m == "recovered"))
    );
    assert_eq!(f_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn empty_text_delta_does_not_commit_the_primary() {
    let primary = ScriptBackend::stream(
        "primary",
        vec![
            Ok(BackendEvent::TextDelta(String::new())),
            Err(BackendError::Transient("connection reset".into())),
        ],
    );
    let fallback = ScriptBackend::stream(
        "fallback",
        vec![
            Ok(BackendEvent::TextDelta("recovered".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let fallback_count = fallback.counter();

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, Ok(BackendEvent::TextDelta(text)) if text == "recovered"))
    );
    assert_eq!(fallback_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn committed_stream_announces_the_selected_backend() {
    // The primary produces output, so the chain commits to it. The first
    // committed event must announce the primary as the selected backend.
    let primary = ScriptBackend::stream(
        "primary",
        vec![
            Ok(BackendEvent::TextDelta("hi".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let fallback = ScriptBackend::stream("fallback", vec![Ok(BackendEvent::Message("no".into()))]);
    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(matches!(
        events.first(),
        Some(Ok(BackendEvent::BackendSelected(name))) if name == "primary"
    ));
}

#[tokio::test]
async fn selected_backend_reports_the_fallback_after_failover() {
    // Primary fails over before output; the committed stream must announce the
    // *fallback* as the selected backend so the substitution is observable.
    let primary =
        ScriptBackend::stream("primary", vec![Err(BackendError::Transient("boom".into()))]);
    let fallback = ScriptBackend::stream(
        "fallback",
        vec![
            Ok(BackendEvent::TextDelta("recovered".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    // The failover is announced before the eventual selection — a caller
    // that only inspects the committed winner (BackendSelected) would
    // otherwise have no way to tell the primary was ever tried and failed.
    assert!(matches!(
        events.first(),
        Some(Ok(BackendEvent::Failover(name))) if name == "primary"
    ));
    assert!(matches!(
        events.get(1),
        Some(Ok(BackendEvent::BackendSelected(name))) if name == "fallback"
    ));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Ok(BackendEvent::TextDelta(text)) if text == "recovered"))
    );
}

#[tokio::test]
async fn buffered_progress_is_discarded_on_failover() {
    // Progress is NOT output: an error after Progress still fails over, and
    // the discarded Progress must not leak into the committed stream.
    let primary = ScriptBackend::stream(
        "primary",
        vec![
            Ok(BackendEvent::Progress("working".into())),
            Err(BackendError::Transient("boom".into())),
        ],
    );
    let fallback = ScriptBackend::stream(
        "fallback",
        vec![
            Ok(BackendEvent::Message("recovered".into())),
            Ok(BackendEvent::Done),
        ],
    );

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::Progress(_)))),
        "discarded primary Progress must not appear"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::Message(m)) if m == "recovered"))
    );
}

#[tokio::test]
async fn buffered_progress_then_message_commits_without_failover() {
    let primary = ScriptBackend::stream(
        "primary",
        vec![
            Ok(BackendEvent::Progress("working".into())),
            Ok(BackendEvent::Message("done".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let fallback = ScriptBackend::stream("fallback", vec![Ok(BackendEvent::Message("no".into()))]);
    let f_count = fallback.counter();

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::Progress(p)) if p == "working"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::Message(m)) if m == "done"))
    );
    assert_eq!(f_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn no_failover_after_output_error_propagates() {
    // Output THEN error: the error must flow through, fallback untouched.
    let primary = ScriptBackend::stream(
        "primary",
        vec![
            Ok(BackendEvent::TextDelta("partial".into())),
            Err(BackendError::Transient("late failure".into())),
        ],
    );
    let fallback = ScriptBackend::stream("fallback", vec![Ok(BackendEvent::Message("no".into()))]);
    let f_count = fallback.counter();

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::TextDelta(t)) if t == "partial"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Err(BackendError::Transient(_))))
    );
    assert_eq!(
        f_count.load(Ordering::SeqCst),
        0,
        "no failover after output"
    );
}

#[tokio::test]
async fn non_failoverable_startup_error_stops_the_chain() {
    // Context overflow before output is NOT failoverable: deliver it, do not
    // touch the fallback.
    let primary = ScriptBackend::stream(
        "primary",
        vec![Err(BackendError::ContextOverflow("too big".into()))],
    );
    let fallback = ScriptBackend::stream("fallback", vec![Ok(BackendEvent::Message("no".into()))]);
    let f_count = fallback.counter();

    let chain = ProviderChain::new("chain", vec![Arc::new(primary), Arc::new(fallback)]);
    let events = drain(chain.start(&req()).await.unwrap()).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, Err(BackendError::ContextOverflow(_))))
    );
    assert_eq!(f_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn exhausted_chain_yields_last_error_through_the_stream() {
    let only = ScriptBackend::start_failure("only", BackendError::Transient("gone".into()));
    let chain = ProviderChain::new("chain", vec![Arc::new(only)]);
    // `start` no longer blocks: failover is lazy, so the last backend's
    // failoverable startup error surfaces as the stream's terminal item.
    let events = drain(chain.start(&req()).await.unwrap()).await;
    assert!(matches!(
        events.as_slice(),
        [Err(BackendError::Transient(_))]
    ));
}

#[tokio::test]
async fn empty_chain_reports_no_backend_available() {
    let chain = ProviderChain::new("empty", vec![]);
    // The one eager failure: an empty chain has nothing to poll.
    let err = match chain.start(&req()).await {
        Ok(_) => panic!("expected an error from an empty chain"),
        Err(e) => e,
    };
    assert!(matches!(err, BackendError::NoBackendAvailable(_)));
}

#[tokio::test]
async fn start_is_lazy_backends_untouched_until_the_stream_is_polled() {
    // The chain must not block until first output: `start` returns immediately
    // and no constituent backend is started until the returned stream is
    // polled. This is what lets the session race polling against cancellation.
    let primary = ScriptBackend::stream(
        "primary",
        vec![
            Ok(BackendEvent::TextDelta("hi".into())),
            Ok(BackendEvent::Done),
        ],
    );
    let p_count = primary.counter();
    let chain = ProviderChain::new("chain", vec![Arc::new(primary)]);

    let stream = chain.start(&req()).await.unwrap();
    assert_eq!(
        p_count.load(Ordering::SeqCst),
        0,
        "backend must not be started before the stream is polled"
    );

    let events = drain(stream).await;
    assert_eq!(p_count.load(Ordering::SeqCst), 1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(BackendEvent::TextDelta(t)) if t == "hi"))
    );
}

#[tokio::test]
async fn dropping_the_committed_stream_drops_the_backend_stream() {
    use std::sync::atomic::AtomicBool;

    // A stream item that flips a flag when dropped — stands in for a CLI child
    // killed on drop. Proves the chain's lazy generator holds the committed
    // backend stream in scope, so dropping the chain stream cancels it.
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let guard = DropFlag(dropped.clone());

    struct GuardedBackend {
        guard: std::sync::Mutex<Option<DropFlag>>,
    }
    #[async_trait]
    impl SessionBackend for GuardedBackend {
        fn name(&self) -> &str {
            "guarded"
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::full_api()
        }
        async fn start(
            &self,
            _request: &BackendRequest,
        ) -> Result<BackendEventStream, BackendError> {
            let guard = self.guard.lock().unwrap().take();
            // An endless stream that yields one output then parks forever,
            // keeping `guard` alive inside the stream state until dropped.
            let stream = async_stream::stream! {
                yield Ok(BackendEvent::TextDelta("committed".into()));
                let _held = guard; // moved into the generator scope
                futures::future::pending::<()>().await;
                // Unreachable; keeps `_held` alive until the stream is dropped.
                yield Ok(BackendEvent::Done);
            };
            Ok(Box::pin(stream))
        }
    }

    let backend = GuardedBackend {
        guard: std::sync::Mutex::new(Some(guard)),
    };
    let chain = ProviderChain::new("chain", vec![Arc::new(backend)]);

    let mut stream = chain.start(&req()).await.unwrap();
    // Poll until the committed output arrives (backend committed, guard held).
    let mut saw_output = false;
    for _ in 0..8 {
        if let Some(Ok(BackendEvent::TextDelta(t))) = stream.next().await
            && t == "committed"
        {
            saw_output = true;
            break;
        }
    }
    assert!(saw_output, "expected committed output before the park");
    assert!(
        !dropped.load(Ordering::SeqCst),
        "guard alive while streaming"
    );

    // Dropping the chain stream must drop the backend stream (and its guard).
    drop(stream);
    assert!(
        dropped.load(Ordering::SeqCst),
        "dropping the chain stream must cancel the in-flight backend stream"
    );
}

#[test]
fn capabilities_are_the_union_of_members() {
    let cli = Arc::new(
        ScriptBackend::stream("cli", vec![]).with_capabilities(BackendCapabilities::buffered_cli()),
    ) as Arc<dyn SessionBackend>;
    let api = Arc::new(ScriptBackend::stream("api", vec![])) as Arc<dyn SessionBackend>;
    let chain = ProviderChain::new("chain", vec![cli, api]);
    let caps = chain.capabilities();
    assert!(caps.streaming); // from api
    assert!(caps.tools); // from api
}
