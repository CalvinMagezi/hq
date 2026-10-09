//! FR-065: per-child model selection, visibility, validation and fallback.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use hq_core::types::{ChatMessage, MessageRole, SecurityProfile};
use hq_llm::provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};
use serde_json::json;
use tokio_stream::Stream;

use crate::backend::{
    ApiBackend, BackendCapabilities, BackendError, BackendRegistry, BackendRequest, SessionBackend,
};
use crate::session::SessionConfig;

use super::model_select::{ModelSource, OnModelUnavailable};
use super::service::AgentService;
use super::tool::SpawnSubagentsTool;
use super::types::{ChildExecContext, ChildOutcome, ChildPlan, ChildRequest, ChildStatus};

/// A provider that records the model of every request it serves, and can be
/// told to fail with an error carrying a credential-shaped string.
struct Recorder {
    seen: Mutex<Vec<String>>,
    fail_with: Option<String>,
}

impl Recorder {
    fn ok() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            fail_with: None,
        })
    }

    fn failing(message: &str) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            fail_with: Some(message.to_string()),
        })
    }

    fn models(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for Recorder {
    fn name(&self) -> &str {
        "recorder"
    }

    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        self.seen.lock().unwrap().push(request.model.clone());
        if let Some(message) = &self.fail_with {
            return Err(LlmError::Auth {
                status: 500,
                message: message.clone(),
            }
            .into());
        }
        Ok(ChatResponse {
            message: ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::Assistant,
                content: "done".to_string(),
                tool_calls: Vec::new(),
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
        _request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        Ok(Box::pin(tokio_stream::iter(vec![Ok(StreamChunk::Done)])))
    }
}

/// A backend with a pinned model but no tool loop (like the Copilot CLI).
struct NoToolsBackend;

#[async_trait]
impl SessionBackend for NoToolsBackend {
    fn name(&self) -> &str {
        "cli"
    }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::buffered_cli()
    }
    fn pinned_model(&self) -> Option<String> {
        Some("m-cli".to_string())
    }
    async fn start(
        &self,
        _request: &BackendRequest,
    ) -> Result<crate::backend::BackendEventStream, BackendError> {
        Err(BackendError::Unavailable("not used".to_string()))
    }
}

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

fn service(provider: Arc<dyn LlmProvider>) -> AgentService {
    AgentService::new(
        provider,
        std::env::temp_dir(),
        vec![std::env::temp_dir()],
        fast_config(),
        SecurityProfile::Guarded,
        0,
        3,
        Duration::from_secs(30),
    )
}

/// Two API backends (alpha primary, beta), each with its own recorder and
/// pinned model, plus a tool-less CLI backend.
fn chain_service(
    default: Arc<Recorder>,
    alpha: Arc<Recorder>,
    beta: Arc<Recorder>,
) -> AgentService {
    let api = |name: &str, provider: Arc<Recorder>, model: &str| {
        Arc::new(
            ApiBackend::new(name, provider as Arc<dyn LlmProvider>).with_model(Some(model.into())),
        ) as Arc<dyn SessionBackend>
    };
    let entries: Vec<(String, Arc<dyn SessionBackend>)> = vec![
        ("alpha".into(), api("alpha", alpha, "m-a")),
        ("beta".into(), api("beta", beta, "m-b")),
        ("cli".into(), Arc::new(NoToolsBackend)),
    ];
    let registry = Arc::new(BackendRegistry::from_backends(entries, "alpha"));
    service(default).with_backend_registry(Some(registry))
}

fn child(model: Option<&str>, policy: OnModelUnavailable) -> ChildRequest {
    let mut req = ChildRequest::new("t1", "do the thing");
    req.backend = Some(super::INPROCESS_BACKEND.to_string());
    req.model = model.map(str::to_string);
    req.on_model_unavailable = policy;
    req
}

async fn run(svc: &AgentService, req: ChildRequest) -> ChildOutcome {
    let mut outcomes = svc
        .execute(ChildPlan::single(req), ChildExecContext::default())
        .await;
    outcomes.pop().expect("one outcome")
}

fn fake_key() -> String {
    ["sk", "or", "v1", &"a1b2".repeat(16)].join("-")
}

#[tokio::test]
async fn no_model_inherits_the_parent_model() {
    let provider = Recorder::ok();
    let out = run(
        &service(provider.clone()),
        child(None, OnModelUnavailable::Reject),
    )
    .await;

    assert_eq!(out.status, ChildStatus::Completed, "{out:?}");
    let eff = out.effective_model.expect("effective model reported");
    assert_eq!(eff.source, ModelSource::Inherited);
    assert_eq!(eff.model, fast_config().model);
    assert_eq!(eff.fallback_from, None);
    assert_eq!(provider.models(), vec![fast_config().model]);
}

#[tokio::test]
async fn explicit_override_is_used_and_marked_as_override() {
    let provider = Recorder::ok();
    let out = run(
        &service(provider.clone()),
        child(Some("some-model"), OnModelUnavailable::Reject),
    )
    .await;

    assert_eq!(out.status, ChildStatus::Completed, "{out:?}");
    let eff = out.effective_model.unwrap();
    assert_eq!(
        (eff.source, eff.model.as_str()),
        (ModelSource::Override, "some-model")
    );
    assert_eq!(provider.models(), vec!["some-model"]);
}

#[tokio::test]
async fn chain_override_routes_to_the_matching_backend_by_model_or_name() {
    for selector in ["m-b", "beta"] {
        let (default, alpha, beta) = (Recorder::ok(), Recorder::ok(), Recorder::ok());
        let svc = chain_service(default.clone(), alpha.clone(), beta.clone());
        let out = run(&svc, child(Some(selector), OnModelUnavailable::Reject)).await;

        assert_eq!(out.status, ChildStatus::Completed, "{selector}: {out:?}");
        let eff = out.effective_model.unwrap();
        assert_eq!(eff.backend.as_deref(), Some("beta"));
        assert_eq!(
            (eff.source, eff.model.as_str()),
            (ModelSource::Override, "m-b")
        );
        assert_eq!(beta.models(), vec!["m-b"], "{selector}");
        assert!(default.models().is_empty() && alpha.models().is_empty());
    }
}

#[tokio::test]
async fn chain_without_override_reports_the_primary_as_inherited() {
    let (default, alpha, beta) = (Recorder::ok(), Recorder::ok(), Recorder::ok());
    let svc = chain_service(default.clone(), alpha, beta);
    let out = run(&svc, child(None, OnModelUnavailable::Reject)).await;

    let eff = out.effective_model.unwrap();
    assert_eq!(
        (eff.source, eff.backend.as_deref()),
        (ModelSource::Inherited, Some("alpha"))
    );
    assert_eq!(default.models().len(), 1);
}

#[tokio::test]
async fn unsupported_model_is_rejected_before_dispatch_and_lists_choices() {
    let (default, alpha, beta) = (Recorder::ok(), Recorder::ok(), Recorder::ok());
    let svc = chain_service(default.clone(), alpha.clone(), beta.clone());
    let out = run(&svc, child(Some("nope-9"), OnModelUnavailable::Reject)).await;

    assert_eq!(out.status, ChildStatus::Rejected);
    let err = out.error.unwrap();
    assert!(
        err.contains("nope-9") && err.contains("alpha (m-a)") && err.contains("beta (m-b)"),
        "{err}"
    );
    assert!(default.models().is_empty() && alpha.models().is_empty() && beta.models().is_empty());
    assert!(out.effective_model.is_none());
}

#[tokio::test]
async fn backend_without_a_tool_loop_is_rejected_for_a_child() {
    let svc = chain_service(Recorder::ok(), Recorder::ok(), Recorder::ok());
    let out = run(&svc, child(Some("cli"), OnModelUnavailable::Reject)).await;

    assert_eq!(out.status, ChildStatus::Rejected);
    assert!(out.error.unwrap().contains("tool loop"));
}

#[tokio::test]
async fn inherit_policy_falls_back_visibly_never_silently() {
    let (default, alpha, beta) = (Recorder::ok(), Recorder::ok(), Recorder::ok());
    let svc = chain_service(default.clone(), alpha, beta);
    let out = run(&svc, child(Some("nope-9"), OnModelUnavailable::Inherit)).await;

    assert_eq!(out.status, ChildStatus::Completed, "{out:?}");
    let eff = out.effective_model.unwrap();
    assert_eq!(eff.source, ModelSource::Inherited);
    assert_eq!(eff.fallback_from.as_deref(), Some("nope-9"));
    assert_eq!(default.models().len(), 1);
}

#[tokio::test]
async fn provider_failure_names_the_override_and_never_switches_model() {
    let leaked = format!("upstream said bad key {}", fake_key());
    let (default, alpha) = (Recorder::ok(), Recorder::ok());
    let beta = Recorder::failing(&leaked);
    let svc = chain_service(default.clone(), alpha.clone(), beta.clone());
    let out = run(&svc, child(Some("m-b"), OnModelUnavailable::Inherit)).await;

    assert_eq!(out.status, ChildStatus::Failed, "{out:?}");
    let eff = out.effective_model.unwrap();
    assert_eq!((eff.model.as_str(), eff.fallback_from), ("m-b", None));
    assert!(default.models().is_empty() && alpha.models().is_empty());
    let shown = format!("{:?} {}", out.error, out.output);
    assert!(!shown.contains(&fake_key()), "credential leaked: {shown}");
}

#[tokio::test]
async fn credential_shaped_model_id_is_rejected_and_never_echoed() {
    let key = fake_key();
    let out = run(
        &service(Recorder::ok()),
        child(Some(&key), OnModelUnavailable::Reject),
    )
    .await;

    assert_eq!(out.status, ChildStatus::Rejected);
    let shown = format!("{:?} {}", out.error, out.output);
    assert!(!shown.contains(&key), "credential echoed: {shown}");
}

#[test]
fn tool_parses_the_fallback_policy_and_rejects_unknown_values() {
    let schema = super::tool::task_schema();
    assert!(schema.get("on_model_unavailable").is_some());

    let parse = |v: &str| {
        super::tool::parse_child_request(&json!({"goal": "g", "on_model_unavailable": v}), 0)
    };
    assert_eq!(
        parse("inherit").unwrap().on_model_unavailable,
        OnModelUnavailable::Inherit
    );
    assert_eq!(
        parse("reject").unwrap().on_model_unavailable,
        OnModelUnavailable::Reject
    );
    assert!(parse("silently").is_err());
    let default = super::tool::parse_child_request(&json!({"goal": "g"}), 0).unwrap();
    assert_eq!(default.on_model_unavailable, OnModelUnavailable::Reject);
}

#[tokio::test]
async fn tool_result_shows_effective_model_and_source() {
    use crate::tools::AgentTool;
    let svc = Arc::new(service(Recorder::ok()));
    let tool = SpawnSubagentsTool::new(svc);
    let result = tool
        .execute(
            "id",
            json!({"task": {"goal": "g", "backend": "hq", "model": "some-model"}}),
        )
        .await
        .unwrap();
    let text = &result.content[0].text;
    assert!(text.contains("[model some-model, override]"), "{text}");
}
