//! A budget refusal stops one attempt, lets the chain try another backend, and says what it was.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio_stream::Stream;

use crate::backend_chain::ChainProvider;
use crate::budget::{Admission, BudgetBlocked, BudgetGate, GateRequest};
use crate::instrument::Instruments;
use crate::outcome_sink::{OutcomeEvent, TaskOutcomeSink};
use crate::provider::{ChatRequest, ChatResponse, LlmProvider, StreamChunk};
use hq_core::types::{ChatMessage, MessageRole};

use super::{CostTier, LlmRouter};

const MODEL: &str = "anthropic/claude-haiku-5.5";

struct Answers;

#[async_trait]
impl LlmProvider for Answers {
    fn name(&self) -> &str {
        "answers"
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
            input_tokens: 10,
            output_tokens: 5,
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
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        let resp = self.chat(request).await?;
        Ok(super::strategy::response_to_stream(resp))
    }
}

/// Refuses every attempt on the named providers, allows the rest, and downgrades one.
struct Gate {
    deny: Vec<&'static str>,
    downgrade: Option<&'static str>,
    asked: Mutex<Vec<String>>,
}

impl Gate {
    fn denying(deny: &[&'static str]) -> Arc<Self> {
        Arc::new(Self {
            deny: deny.to_vec(),
            downgrade: None,
            asked: Mutex::default(),
        })
    }
}

#[async_trait]
impl BudgetGate for Gate {
    async fn admit(&self, req: &GateRequest<'_>) -> Admission {
        self.asked.lock().unwrap().push(req.provider.to_string());
        if self.deny.contains(&req.provider) {
            return Admission::Deny(BudgetBlocked {
                budget: "month".into(),
                spent_usd: 20.0,
                limit_usd: 20.0,
                message: "budget 'month' is used up".into(),
            });
        }
        match self.downgrade {
            Some(model) => Admission::Downgrade {
                model: model.into(),
            },
            None => Admission::Allow,
        }
    }
}

#[derive(Default)]
struct Rows(Mutex<Vec<OutcomeEvent>>);

#[async_trait]
impl TaskOutcomeSink for Rows {
    async fn record(&self, event: OutcomeEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn request() -> ChatRequest {
    ChatRequest {
        model: MODEL.into(),
        ..Default::default()
    }
}

fn chained(gate: Arc<Gate>, sink: Arc<Rows>) -> LlmRouter {
    let instruments = Instruments::new();
    let chain = ChainProvider::from_links(
        vec![
            ("primary", Arc::new(Answers), MODEL),
            ("fallback", Arc::new(Answers), MODEL),
        ],
        instruments.clone(),
    );
    let mut router = LlmRouter::with_instruments(instruments);
    router.add_provider("backends", Arc::new(chain));
    router.add_route("*", "backends", "", CostTier::Budget);
    router.set_budget_gate(gate);
    router.set_outcome_sink(sink);
    router
}

#[tokio::test]
async fn a_refused_backend_hands_the_call_to_the_next_one_in_the_chain() {
    let gate = Gate::denying(&["primary"]);
    let sink = Arc::new(Rows::default());
    let router = chained(gate.clone(), sink.clone());

    router.chat(&request()).await.unwrap();

    assert_eq!(*gate.asked.lock().unwrap(), vec!["primary", "fallback"]);
    tokio::task::yield_now().await;
    let rows = sink.0.lock().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].provider, "fallback");
}

#[tokio::test]
async fn when_every_backend_is_refused_the_caller_sees_the_budget_not_a_generic_failure() {
    let router = chained(Gate::denying(&["primary", "fallback"]), Arc::new(Rows::default()));

    let chat_err = router.chat(&request()).await.unwrap_err();
    let stream_err = router.chat_stream(&request()).await.err().unwrap();

    for err in [chat_err, stream_err] {
        let blocked = err.downcast_ref::<BudgetBlocked>().expect("typed refusal");
        assert_eq!(blocked.budget, "month");
        assert!(err.to_string().contains("used up"));
    }
}

#[tokio::test]
async fn a_refusal_is_not_recorded_as_a_provider_failure() {
    let sink = Arc::new(Rows::default());
    let gate = Gate::denying(&["solo"]);
    let mut router = LlmRouter::new();
    router.add_provider("solo", Arc::new(Answers));
    router.add_route("*", "solo", MODEL, CostTier::Budget);
    router.set_budget_gate(gate);
    router.set_outcome_sink(sink.clone());

    assert!(router.chat(&request()).await.is_err());

    tokio::task::yield_now().await;
    assert!(sink.0.lock().unwrap().is_empty(), "no call was made, so no ledger row");
    let health = router.health.lock().unwrap();
    let solo = health.iter().find(|(n, _)| n == "solo").unwrap();
    assert_eq!(solo.1.total_failures, 0);
}

#[tokio::test]
async fn a_downgrade_sends_the_attempt_to_the_cheaper_model() {
    let gate = Arc::new(Gate {
        deny: vec![],
        downgrade: Some("cheap/model"),
        asked: Mutex::default(),
    });
    let mut router = LlmRouter::new();
    router.add_provider("solo", Arc::new(Answers));
    router.add_route("*", "solo", MODEL, CostTier::Budget);
    router.set_budget_gate(gate);

    let resp = router.chat(&request()).await.unwrap();

    assert_eq!(resp.message.content, "cheap/model");
}

#[tokio::test]
async fn with_no_gate_nothing_changes() {
    let mut router = LlmRouter::new();
    router.add_provider("solo", Arc::new(Answers));
    router.add_route("*", "solo", MODEL, CostTier::Budget);

    assert_eq!(router.chat(&request()).await.unwrap().message.content, MODEL);
}

/// Blocks the original model, downgrades it, then blocks the cheaper one too.
struct DowngradeIntoABlock;

#[async_trait]
impl BudgetGate for DowngradeIntoABlock {
    async fn admit(&self, req: &GateRequest<'_>) -> Admission {
        if req.model == "cheap/model" {
            return Admission::Deny(BudgetBlocked {
                budget: "cheap".into(),
                spent_usd: 0.0,
                limit_usd: 0.0,
                message: "the cheaper model is over a budget too".into(),
            });
        }
        Admission::Downgrade {
            model: "cheap/model".into(),
        }
    }
}

#[tokio::test]
async fn a_downgraded_call_is_judged_again_on_the_model_that_will_run() {
    let mut router = LlmRouter::new();
    router.add_provider("solo", Arc::new(Answers));
    router.add_route("*", "solo", MODEL, CostTier::Budget);
    router.set_budget_gate(Arc::new(DowngradeIntoABlock));

    let err = router.chat(&request()).await.unwrap_err();

    assert!(err.to_string().contains("cheaper model"), "{err}");
}
