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
    /// How close each provider's budgets are to running out, `0.0` to `1.0`, keyed by provider
    /// name; [`GLOBAL_PRESSURE_KEY`] covers every provider.
    pressure: RwLock<std::collections::HashMap<String, f64>>,
}

/// Pressure from a budget that covers all providers.
pub const GLOBAL_PRESSURE_KEY: &str = "*";

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

    /// Record how close a budget is to running out. The router weighs cost more for a provider
    /// under pressure, so calls drift to cheaper ones before a limit blocks them.
    pub fn set_pressure(&self, key: &str, pressure: f64) {
        self.pressure
            .write()
            .unwrap()
            .insert(key.to_string(), pressure.clamp(0.0, 1.0));
    }

    pub fn pressure_for(&self, provider: &str) -> f64 {
        let map = self.pressure.read().unwrap();
        let get = |k: &str| map.get(k).copied().unwrap_or(0.0);
        get(GLOBAL_PRESSURE_KEY).max(get(provider))
    }

    fn sink(&self) -> Option<SharedSink> {
        self.sink.read().unwrap().clone()
    }

    fn gate(&self) -> Option<SharedGate> {
        self.gate.read().unwrap().clone()
    }
}

/// Dollars assumed for a call whose cost is unknown in advance (an image, an embedding), so a budget
/// that is already used up refuses it but an unpriced model is not treated as unknown.
const UNKNOWN_EXTERNAL_ESTIMATE_USD: f64 = 0.0001;

/// A call made outside `LlmProvider` (image generation, embeddings) that still belongs in the ledger.
pub struct ExternalCall<'a> {
    pub provider: &'a str,
    pub class: ProviderClass,
    pub model: &'a str,
    pub origin: &'static str,
    pub task_hint: &'static str,
}

impl Instruments {
    /// Ask the budget gate about an external call. A refusal says which budget stopped it.
    pub async fn admit_external(&self, call: &ExternalCall<'_>) -> Result<(), crate::budget::BudgetBlocked> {
        let Some(gate) = self.gate() else {
            return Ok(());
        };
        let verdict = gate
            .admit(&GateRequest {
                provider: call.provider,
                class: call.class,
                model: call.model,
                origin: call.origin,
                estimate_usd: Some(UNKNOWN_EXTERNAL_ESTIMATE_USD),
            })
            .await;
        match verdict {
            Admission::Deny(blocked) => Err(blocked),
            Admission::Allow | Admission::Downgrade { .. } => Ok(()),
        }
    }

    /// Record a finished external call. `usage` carries the provider's own billed figure when the
    /// response had one; `error` is the failure class for a call that failed.
    pub fn record_external(
        &self,
        call: &ExternalCall<'_>,
        usage: Option<Usage>,
        latency: Duration,
        error: Option<&str>,
    ) {
        let Some(sink) = self.sink() else {
            return;
        };
        let mut ctx = context_for_record();
        ctx.origin = call.origin;
        let priced = price_outcome(call.class, call.model, usage.as_ref(), error.is_some());
        let u = usage.unwrap_or_default();
        let event = OutcomeEvent {
            session_id: ctx.session_id,
            turn_idx: ctx.turn_idx,
            model: call.model.to_string(),
            provider: call.provider.to_string(),
            task_hint: call.task_hint,
            latency_ms: latency.as_millis().min(i64::MAX as u128) as i64,
            input_tokens: usage.map(|u| u.input as i64),
            output_tokens: usage.map(|u| u.output as i64),
            cache_read_tokens: u.cache_read as i64,
            cache_write_tokens: u.cache_write as i64,
            reasoning_tokens: u.reasoning as i64,
            cost_usd: priced.usd,
            provider_cost_usd: u.billed_usd,
            cost_source: priced.source.as_str(),
            origin: call.origin,
            success: error.is_none(),
            error_class: error.map(str::to_string),
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                sink.record(event).await;
            });
        }
    }
}

/// OpenRouter's `usage` object as a [`Usage`]: tokens plus the billed `cost`.
pub fn usage_from_openrouter(usage: &serde_json::Value) -> Usage {
    let n = |p: &str| usage.pointer(p).and_then(|v| v.as_u64()).map_or(0, |v| v.min(u64::from(u32::MAX)) as u32);
    Usage {
        input: n("/prompt_tokens"),
        output: n("/completion_tokens"),
        cache_read: n("/prompt_tokens_details/cached_tokens"),
        reasoning: n("/completion_tokens_details/reasoning_tokens"),
        billed_usd: usage.get("cost").and_then(|c| c.as_f64()),
        ..Usage::default()
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

fn refuse(blocked: crate::budget::BudgetBlocked) -> anyhow::Error {
    note_blocked(&blocked);
    blocked.into()
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

    /// Ask the gate about `request`, once.
    async fn ask(&self, gate: &SharedGate, request: &ChatRequest) -> Admission {
        gate.admit(&GateRequest {
            provider: &self.name,
            class: self.class,
            model: &request.model,
            origin: current_context().origin,
            estimate_usd: estimate_cost(self.class, request),
        })
        .await
    }

    /// The request to send, possibly on a cheaper model, or the refusal. A downgraded call is
    /// asked about again on its new model, so a blocking budget still judges what will really run.
    async fn admit(&self, request: &ChatRequest) -> Result<Option<ChatRequest>> {
        let Some(gate) = self.instruments.gate() else {
            return Ok(None);
        };
        match self.ask(&gate, request).await {
            Admission::Allow => Ok(None),
            Admission::Deny(blocked) => Err(refuse(blocked)),
            Admission::Downgrade { model } => {
                let downgraded = ChatRequest {
                    model,
                    ..request.clone()
                };
                match self.ask(&gate, &downgraded).await {
                    Admission::Deny(blocked) => Err(refuse(blocked)),
                    _ => Ok(Some(downgraded)),
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::{BudgetBlocked, BudgetGate};
    use crate::outcome_sink::TaskOutcomeSink;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Rows(Mutex<Vec<OutcomeEvent>>);

    #[async_trait]
    impl TaskOutcomeSink for Rows {
        async fn record(&self, event: OutcomeEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    struct Refuse;

    #[async_trait]
    impl BudgetGate for Refuse {
        async fn admit(&self, req: &GateRequest<'_>) -> Admission {
            assert_eq!(req.origin, "imagegen");
            assert!(req.estimate_usd.is_some_and(|e| e > 0.0), "unknown cost must not read as free or unpriced");
            Admission::Deny(BudgetBlocked {
                budget: "img".into(),
                spent_usd: 1.0,
                limit_usd: 1.0,
                message: "used up".into(),
            })
        }
    }

    fn call() -> ExternalCall<'static> {
        ExternalCall {
            provider: "openrouter",
            class: ProviderClass::Metered,
            model: "some/image-model",
            origin: crate::outcome_sink::origin::IMAGEGEN,
            task_hint: "image",
        }
    }

    #[test]
    fn an_openrouter_usage_object_gives_tokens_and_the_billed_cost() {
        let u = usage_from_openrouter(&serde_json::json!({
            "prompt_tokens": 12, "completion_tokens": 3, "cost": 0.04,
            "prompt_tokens_details": {"cached_tokens": 2}, "completion_tokens_details": {"reasoning_tokens": 1}
        }));
        assert_eq!((u.input, u.output, u.cache_read, u.reasoning, u.billed_usd), (12, 3, 2, 1, Some(0.04)));
        assert_eq!(usage_from_openrouter(&serde_json::json!({})), Usage::default());
    }

    #[tokio::test]
    async fn an_external_call_is_recorded_with_its_origin_and_billed_cost() {
        let rows = Arc::new(Rows::default());
        let instruments = Instruments::new();
        instruments.set_sink(rows.clone());
        let usage = Usage { billed_usd: Some(0.04), ..Usage::default() };
        instruments.record_external(&call(), Some(usage), Duration::from_millis(5), None);
        tokio::task::yield_now().await;
        let rows = rows.0.lock().unwrap();
        assert_eq!(
            (rows[0].origin, rows[0].cost_source, rows[0].cost_usd, rows[0].task_hint),
            ("imagegen", "provider", 0.04, "image")
        );
    }

    #[tokio::test]
    async fn an_origin_budget_that_is_used_up_refuses_an_external_call() {
        let instruments = Instruments::new();
        assert!(instruments.admit_external(&call()).await.is_ok(), "no gate, no limit");
        instruments.set_gate(Arc::new(Refuse));
        let err = instruments.admit_external(&call()).await.unwrap_err();
        assert_eq!(err.budget, "img");
    }
}
