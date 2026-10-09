//! Every concrete provider is wrapped once, here, so each call it makes reaches the spend ledger and
//! the budget gate no matter who drives it: the router, the backend chain or a session's own
//! backend. Recording at the provider is what makes that true; recording higher up would miss
//! whichever path skips the layer.

use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use tokio_stream::Stream;

use crate::budget::{Admission, GateRequest, SharedGate, estimate_cost, note_blocked};
use crate::cost::{ProviderClass, Usage, price_outcome};
use crate::outcome_sink::{OutcomeEvent, SessionContext, SharedSink, context_for_record, current_context};
use crate::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};
use crate::router::{TaskHint, classify_error_for_telemetry};
use crate::tap::OutcomeTap;

/// Where recorded calls go and what is asked before a call. Set once at startup; providers read it
/// at call time, so installing a sink or a gate later still takes effect.
#[derive(Default)]
pub struct Instruments {
    sink: RwLock<Option<SharedSink>>,
    gate: RwLock<Option<SharedGate>>,
}

impl Instruments {
    /// A handle of its own, for a router or a test that must not share the process-wide one.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The process-wide handle production providers share.
    pub fn global() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<Instruments>> = OnceLock::new();
        GLOBAL.get_or_init(Instruments::new).clone()
    }

    pub fn set_sink(&self, sink: SharedSink) {
        *self.sink.write().unwrap() = Some(sink);
    }

    pub fn set_gate(&self, gate: SharedGate) {
        *self.gate.write().unwrap() = Some(gate);
    }

    fn sink(&self) -> Option<SharedSink> {
        self.sink.read().unwrap().clone()
    }

    fn gate(&self) -> Option<SharedGate> {
        self.gate.read().unwrap().clone()
    }
}

/// What the ledger needs to know about one finished call.
pub(crate) struct OutcomeInput<'a> {
    pub provider: &'a str,
    pub class: ProviderClass,
    pub model: &'a str,
    pub task: TaskHint,
    pub latency: Duration,
    pub usage: Option<Usage>,
    /// Error class for a failed call; `None` means it succeeded.
    pub error: Option<&'a str>,
    /// The caller stopped reading a stream that was working. Not a provider failure.
    pub cancelled: bool,
}

/// Price a finished call and hand it to the sink without blocking the caller.
pub(crate) fn emit_outcome(sink: &SharedSink, ctx: SessionContext, call: OutcomeInput<'_>) {
    let usage = call.usage;
    let priced = price_outcome(call.class, call.model, usage.as_ref(), call.error.is_some());
    let u = usage.unwrap_or_default();
    let event = OutcomeEvent {
        session_id: ctx.session_id,
        turn_idx: ctx.turn_idx,
        model: call.model.to_string(),
        provider: call.provider.to_string(),
        task_hint: call.task.as_str(),
        latency_ms: call.latency.as_millis().min(i64::MAX as u128) as i64,
        input_tokens: usage.map(|u| u.input as i64),
        output_tokens: usage.map(|u| u.output as i64),
        cache_read_tokens: u.cache_read as i64,
        cache_write_tokens: u.cache_write as i64,
        reasoning_tokens: u.reasoning as i64,
        cost_usd: priced.usd,
        provider_cost_usd: u.billed_usd,
        cost_source: priced.source.as_str(),
        origin: ctx.origin,
        success: call.error.is_none(),
        error_class: call
            .error
            .map(str::to_string)
            .or(call.cancelled.then(|| "cancelled".to_string())),
    };
    let sink = sink.clone();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            sink.record(event).await;
        });
    } else {
        tracing::warn!("LLM outcome dropped: no Tokio runtime active");
    }
}

/// The model the provider says it ran, falling back to the one requested.
pub(crate) fn answered_model<'a>(resp: &'a ChatResponse, requested: &'a str) -> &'a str {
    if resp.model.is_empty() {
        requested
    } else {
        &resp.model
    }
}

pub(crate) fn usage_of(resp: &ChatResponse) -> Usage {
    Usage {
        input: resp.input_tokens,
        output: resp.output_tokens,
        cache_read: resp.cache_read_tokens,
        cache_write: resp.cache_write_tokens,
        reasoning: resp.reasoning_tokens,
        billed_usd: resp.provider_cost_usd,
    }
}

/// A provider that asks the budget gate before each attempt and records each attempt's outcome.
pub struct InstrumentedProvider {
    inner: Arc<dyn LlmProvider>,
    /// The backend or provider name rows are filed under.
    name: String,
    class: ProviderClass,
    instruments: Arc<Instruments>,
}

impl InstrumentedProvider {
    pub fn wrap(
        inner: Arc<dyn LlmProvider>,
        name: &str,
        class: ProviderClass,
        instruments: Arc<Instruments>,
    ) -> Arc<dyn LlmProvider> {
        if inner.is_instrumented() {
            return inner;
        }
        Arc::new(Self {
            inner,
            name: name.to_string(),
            class,
            instruments,
        })
    }

    /// The request to send, possibly on a cheaper model, or the refusal.
    async fn admit(&self, request: &ChatRequest) -> Result<Option<ChatRequest>> {
        let Some(gate) = self.instruments.gate() else {
            return Ok(None);
        };
        let verdict = gate
            .admit(&GateRequest {
                provider: &self.name,
                class: self.class,
                model: &request.model,
                origin: current_context().origin,
                estimate_usd: estimate_cost(self.class, request),
            })
            .await;
        match verdict {
            Admission::Allow => Ok(None),
            Admission::Downgrade { model } => Ok(Some(ChatRequest {
                model,
                ..request.clone()
            })),
            Admission::Deny(blocked) => {
                note_blocked(&blocked);
                Err(blocked.into())
            }
        }
    }

    fn record(
        &self,
        request: &ChatRequest,
        started: Instant,
        result: std::result::Result<&ChatResponse, &anyhow::Error>,
    ) {
        let Some(sink) = self.instruments.sink() else {
            return;
        };
        let (model, usage, error) = match result {
            Ok(resp) => (
                answered_model(resp, &request.model).to_string(),
                Some(usage_of(resp)),
                None,
            ),
            Err(e) => (
                request.model.clone(),
                None,
                Some(classify_error_for_telemetry(e)),
            ),
        };
        emit_outcome(
            &sink,
            context_for_record(),
            OutcomeInput {
                provider: &self.name,
                class: self.class,
                model: &model,
                task: TaskHint::from_request(request),
                latency: started.elapsed(),
                usage,
                error: error.as_deref(),
                cancelled: false,
            },
        );
    }
}

#[async_trait]
impl LlmProvider for InstrumentedProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn is_instrumented(&self) -> bool {
        true
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let downgraded = self.admit(request).await?;
        let request = downgraded.as_ref().unwrap_or(request);
        let started = Instant::now();
        let result = self.inner.chat(request).await;
        self.record(request, started, result.as_ref());
        result
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let downgraded = self.admit(request).await?;
        let request = downgraded.as_ref().unwrap_or(request);
        let started = Instant::now();
        match self.inner.chat_stream(request).await {
            Ok(stream) => Ok(match self.instruments.sink() {
                Some(sink) => OutcomeTap::wrap(
                    stream,
                    sink,
                    context_for_record(),
                    &self.name,
                    self.class,
                    &request.model,
                    TaskHint::from_request(request),
                    started,
                ),
                None => stream,
            }),
            Err(e) => {
                self.record(request, started, Err(&e));
                Err(e)
            }
        }
    }
}
