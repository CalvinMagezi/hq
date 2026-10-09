use super::*;
use crate::backend::{BackendCapabilities, ProviderChain};
use crate::backend::{BackendEvent, BackendEventStream, BackendRequest};
use crate::session::ToolCallBuilder;
use crate::session::stream::builders_into_tool_calls;
use async_trait::async_trait;
use hq_core::types::{SecurityProfile, ToolResult, ToolResultContent};
use hq_llm::provider::{ChatResponse, LlmError, LlmProvider, StreamChunk};
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio_stream::Stream;
use tokio_stream::StreamExt;

use crate::session::SessionConfig;

#[test]
fn builders_into_tool_calls_defaults_unparseable_arguments_to_an_empty_object() {
    // A no-argument tool call accumulates an empty `arguments` string (no
    // input_json_delta frames ever arrive for it). `serde_json::from_str("")`
    // fails to parse, and the fallback must produce a JSON *object* — every
    // wire format that consumes ToolCall.arguments as `tool_use.input`
    // (the Anthropic Messages API in particular) rejects anything else,
    // e.g. `{"error":"messages.1.content.1.tool_use.input: Input should be
    // an object"}`.
    let builders = vec![ToolCallBuilder {
        id: "call_1".to_string(),
        name: "no_arg_tool".to_string(),
        arguments: String::new(),
    }];

    let calls = builders_into_tool_calls(builders);

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].arguments, serde_json::json!({}));
}

#[test]
fn builders_into_tool_calls_parses_well_formed_arguments() {
    let builders = vec![ToolCallBuilder {
        id: "call_2".to_string(),
        name: "search".to_string(),
        arguments: r#"{"query":"rust"}"#.to_string(),
    }];

    let calls = builders_into_tool_calls(builders);

    assert_eq!(calls[0].arguments, serde_json::json!({"query": "rust"}));
}

#[derive(Clone)]
enum BufferedReply {
    Complete(String),
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    AuthFailure,
}

struct MockProvider {
    replies: Mutex<VecDeque<BufferedReply>>,
    streams: Mutex<VecDeque<Vec<StreamChunk>>>,
    requests: Mutex<Vec<ChatRequest>>,
    calls: AtomicUsize,
}

impl MockProvider {
    fn new(replies: Vec<BufferedReply>, streams: Vec<Vec<StreamChunk>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            streams: Mutex::new(streams.into()),
            requests: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "session-contract-mock"
    }

    async fn chat(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(request.clone());
        match self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted reply")
        {
            BufferedReply::Complete(content) => Ok(ChatResponse {
                message: ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::Assistant,
                    content,
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                },
                input_tokens: 11,
                output_tokens: 7,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                provider_cost_usd: None,
                model: "mock-model".to_string(),
            }),
            BufferedReply::ToolCall {
                id,
                name,
                arguments,
            } => Ok(ChatResponse {
                message: ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::Assistant,
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id,
                        name,
                        arguments,
                    }],
                    tool_call_id: None,
                    reasoning_content: None,
                },
                input_tokens: 11,
                output_tokens: 7,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                provider_cost_usd: None,
                model: "mock-model".to_string(),
            }),
            BufferedReply::AuthFailure => Err(LlmError::Auth {
                status: 401,
                message: "mock credentials rejected".to_string(),
            }
            .into()),
        }
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(request.clone());
        let chunks = self
            .streams
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted stream");
        Ok(Box::pin(tokio_stream::iter(chunks.into_iter().map(Ok))))
    }
}

struct ContractTool {
    cancel: Option<Arc<AtomicBool>>,
}

#[async_trait]
impl crate::tools::AgentTool for ContractTool {
    fn name(&self) -> &str {
        "contract_tool"
    }

    fn description(&self) -> &str {
        "Test-only tool for session contracts"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, _id: &str, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        Ok(ToolResult {
            content: vec![ToolResultContent {
                r#type: "text".to_string(),
                text: "tool output".to_string(),
            }],
            details: None,
            context_modifier: None,
        })
    }
}

/// A read-only tool that blocks (async sleep) for a fixed duration —
/// stands in for a slow, concurrent-safe tool (a network read, a search).
struct BlockingReadTool {
    name: &'static str,
    delay: std::time::Duration,
}

#[async_trait]
impl crate::tools::AgentTool for BlockingReadTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Blocking read-only test tool"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, _id: &str, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        tokio::time::sleep(self.delay).await;
        Ok(ToolResult {
            content: vec![ToolResultContent {
                r#type: "text".to_string(),
                text: format!("{} done", self.name),
            }],
            details: None,
            context_modifier: None,
        })
    }
}

/// A tool that records its execution order into a shared log and reports the
/// log snapshot it observed. `read_only` decides whether it is a barrier;
/// `delay` lets a *mis*ordered concurrent peer finish first, so an ordering
/// regression is observable rather than timing-dependent.
struct OrderTool {
    name: &'static str,
    read_only: bool,
    log: Arc<Mutex<Vec<String>>>,
    delay: std::time::Duration,
}

#[async_trait]
impl crate::tools::AgentTool for OrderTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Order-recording test tool"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn is_read_only(&self) -> bool {
        self.read_only
    }
    async fn execute(&self, _id: &str, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        // Record + snapshot without holding the lock across an await.
        let snapshot = {
            let mut log = self.log.lock().unwrap();
            log.push(self.name.to_string());
            log.join(",")
        };
        Ok(ToolResult {
            content: vec![ToolResultContent {
                r#type: "text".to_string(),
                text: format!("{} observed [{}]", self.name, snapshot),
            }],
            details: None,
            context_modifier: None,
        })
    }
}

fn session(
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Box<dyn crate::tools::AgentTool>>,
) -> AgentSession {
    let allowed_path = std::env::current_dir().unwrap();
    let registry = crate::governance::ToolGuardian::with_default_mode(
        vec![allowed_path],
        SecurityProfile::Guarded,
    )
    .build_registry(tools, crate::governance::LiveUserTurn::unattended());
    let config = SessionConfig {
        max_retries: 0,
        max_duration_secs: None,
        ..SessionConfig::default()
    };
    AgentSession::new(provider, registry, config)
}

fn tool_call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: format!("id-{name}"),
        name: name.to_string(),
        arguments,
    }
}

fn last_content(agent: &AgentSession) -> String {
    agent
        .messages
        .last()
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

#[test]
fn an_unloaded_matching_skill_is_suggested_once_after_the_tool_results() {
    let dir = tempfile::tempdir().unwrap();
    for (name, hint) in [("deploy-pwa", "caddy"), ("loaded-one", "hq-host")] {
        let skill = dir.path().join(name);
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            format!("---\ndescription: \"D\"\nhints:\n  - {hint}\n---\nbody"),
        )
        .unwrap();
    }
    let mut agent = session(Arc::new(MockProvider::new(vec![], vec![])), vec![]);
    agent.set_skill_index(
        Arc::new(hq_tools::skills::SkillHintIndex::build(dir.path())),
        1000,
    );

    let calls = [
        tool_call("load_skill", serde_json::json!({"name": "loaded-one"})),
        tool_call(
            "shell",
            serde_json::json!({"command": "sudo systemctl reload Caddy"}),
        ),
        tool_call("host_read", serde_json::json!({})),
    ];
    let results = calls.iter().map(|_| ("ok".to_string(), None)).collect();
    agent.process_tool_results(&calls, results);

    let tail = last_content(&agent);
    assert_eq!(
        agent.messages[agent.messages.len() - 2].role,
        MessageRole::Tool
    );
    assert!(
        tail.contains("Skill deploy-pwa covers this; call load_skill"),
        "{tail}"
    );
    assert!(
        !tail.contains("loaded-one"),
        "an already loaded skill is not suggested: {tail}"
    );

    let again = [tool_call(
        "shell",
        serde_json::json!({"command": "caddy fmt"}),
    )];
    agent.process_tool_results(&again, vec![("ok".to_string(), None)]);
    assert_eq!(
        agent.messages.last().unwrap().role,
        MessageRole::Tool,
        "suggested once per session"
    );
}

fn event_names(events: &[SessionEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            SessionEvent::TextDelta(_) => "text_delta",
            SessionEvent::Reasoning(_) => "reasoning",
            SessionEvent::TextDone(_) => "text_done",
            SessionEvent::ToolStart { .. } => "tool_start",
            SessionEvent::ToolProgress { .. } => "tool_progress",
            SessionEvent::ToolEnd { .. } => "tool_end",
            SessionEvent::TurnEnd { .. } => "turn_end",
            SessionEvent::Error(_) => "error",
            SessionEvent::Compaction { .. } => "compaction",
            SessionEvent::PreemptiveSummaryReady => "preemptive_summary",
            SessionEvent::RetryAttempt { .. } => "retry_attempt",
            SessionEvent::ContextOverflowRecovery => "context_overflow_recovery",
            SessionEvent::PlanModeEntered { .. } => "plan_mode_entered",
            SessionEvent::PlanModeExited { .. } => "plan_mode_exited",
            SessionEvent::SubagentCompleted { .. } => "subagent_completed",
            SessionEvent::BudgetExhausted { .. } => "budget_exhausted",
            SessionEvent::CostUpdate { .. } => "cost_update",
            SessionEvent::StepCredits { .. } => "step_credits",
        })
        .collect()
}

#[test]
fn cancelled_result_is_not_complete() {
    let result = SessionResult::Cancelled("partial output".to_string());
    assert!(result.text().contains("partial output"));
    assert!(!result.is_complete());
}

#[test]
fn time_limit_result_text_includes_original() {
    let result = SessionResult::TimeLimitReached(
        "last response\n\n[Session stopped: time limit reached after 0h 0m 1s]".to_string(),
    );
    assert!(result.text().contains("last response"));
    assert!(!result.is_complete());
}

#[tokio::test]
async fn buffered_and_streaming_sessions_preserve_final_text_and_usage() {
    let buffered_provider = Arc::new(MockProvider::new(
        vec![BufferedReply::Complete("migration contract".to_string())],
        vec![],
    ));
    let streaming_provider = Arc::new(MockProvider::new(
        vec![],
        vec![vec![
            StreamChunk::Text("migration ".to_string()),
            StreamChunk::Text("contract".to_string()),
            StreamChunk::Usage {
                input_tokens: 11,
                output_tokens: 7,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            },
            StreamChunk::ModelInfo("mock-model".to_string()),
            StreamChunk::Done,
        ]],
    ));
    let buffered_events = Arc::new(Mutex::new(Vec::new()));
    let streaming_events = Arc::new(Mutex::new(Vec::new()));

    let mut buffered = session(buffered_provider, vec![]);
    let buffered_events_sink = buffered_events.clone();
    buffered.on_event(move |event| buffered_events_sink.lock().unwrap().push(event));

    let mut streaming = session(streaming_provider, vec![]);
    let streaming_events_sink = streaming_events.clone();
    streaming.on_event(move |event| streaming_events_sink.lock().unwrap().push(event));

    let buffered_result = buffered.prompt("answer").await.unwrap();
    let streaming_result = streaming.prompt_stream("answer").await.unwrap();

    assert_eq!(buffered_result.text(), "migration contract");
    assert_eq!(streaming_result.text(), buffered_result.text());
    assert_eq!(
        streaming.stats().total_input_tokens,
        buffered.stats().total_input_tokens
    );
    assert_eq!(
        streaming.stats().total_output_tokens,
        buffered.stats().total_output_tokens
    );
    // One engine, one canonical order: content deltas first (buffered emits
    // the final answer as one delta; streaming emits many), then the cost
    // snapshot, then the terminal text_done + turn_end.
    assert_eq!(
        event_names(&buffered_events.lock().unwrap()),
        vec!["text_delta", "cost_update", "text_done", "turn_end"]
    );
    assert_eq!(
        event_names(&streaming_events.lock().unwrap()),
        vec![
            "text_delta",
            "text_delta",
            "cost_update",
            "text_done",
            "turn_end"
        ]
    );
}

/// Memory extraction used to fire from the tool path with the last three
/// raw messages, so plain replies and multi-tool turns never reached it.
#[tokio::test]
async fn post_turn_callback_fires_once_with_the_prompt_and_final_reply() {
    let provider = Arc::new(MockProvider::new(
        vec![
            BufferedReply::ToolCall {
                id: "call-1".to_string(),
                name: "contract_tool".to_string(),
                arguments: serde_json::json!({"input": "value"}),
            },
            BufferedReply::Complete("continued after tool".to_string()),
        ],
        vec![],
    ));
    let mut agent = session(provider, vec![Box::new(ContractTool { cancel: None })]);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.set_post_turn_callback(Arc::new(move |exchange| {
        let _ = tx.send(exchange);
    }));

    agent.prompt("use the tool").await.unwrap();

    let exchange = rx.recv().await.expect("callback fired");
    assert_eq!(
        exchange,
        vec![
            ("user".to_string(), "use the tool".to_string()),
            ("assistant".to_string(), "continued after tool".to_string()),
        ]
    );
    drop(agent);
    assert!(rx.recv().await.is_none(), "callback fired more than once");
}

#[tokio::test]
async fn buffered_session_continues_after_tool_result() {
    let provider = Arc::new(MockProvider::new(
        vec![
            BufferedReply::ToolCall {
                id: "call-1".to_string(),
                name: "contract_tool".to_string(),
                arguments: serde_json::json!({"input": "value"}),
            },
            BufferedReply::Complete("continued after tool".to_string()),
        ],
        vec![],
    ));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut agent = session(
        provider.clone(),
        vec![Box::new(ContractTool { cancel: None })],
    );
    let sink = events.clone();
    agent.on_event(move |event| sink.lock().unwrap().push(event));

    let result = agent.prompt("use the tool").await.unwrap();

    assert_eq!(result.text(), "continued after tool");
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
    assert!(
        provider.requests.lock().unwrap()[1]
            .messages
            .iter()
            .any(|message| message.role == MessageRole::Tool && message.content == "tool output")
    );
    assert_eq!(
        event_names(&events.lock().unwrap()),
        vec![
            // Turn 0: tool call (no answer text) → cost, then tool lifecycle.
            "cost_update",
            "tool_start",
            "tool_progress",
            "tool_end",
            "turn_end",
            // Turn 1: buffered final answer as one delta, then cost + done.
            "text_delta",
            "cost_update",
            "text_done",
            "turn_end",
        ]
    );
}

#[tokio::test]
async fn cancellation_after_a_tool_stops_before_the_next_provider_call() {
    let provider = Arc::new(MockProvider::new(
        vec![BufferedReply::ToolCall {
            id: "call-1".to_string(),
            name: "contract_tool".to_string(),
            arguments: serde_json::json!({}),
        }],
        vec![],
    ));
    let cancel = Arc::new(AtomicBool::new(false));
    let mut agent = session(
        provider.clone(),
        vec![Box::new(ContractTool {
            cancel: Some(cancel.clone()),
        })],
    );
    agent.cancel = cancel;

    let result = agent.prompt("cancel after tool").await.unwrap();

    assert!(matches!(result, SessionResult::Cancelled(_)));
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    assert_eq!(agent.stats().tool_call_count, 1);
}

#[tokio::test]
async fn cancel_during_tool_execution_stops_before_the_tool_finishes() {
    // A tool call in flight used to be un-raced against cancel — the loop
    // only checked cancel before starting a new provider call. This
    // exercises the select! added around execute_tools_parallel_traced:
    // cancel trips ~30ms in, well inside the tool's 300ms sleep, and the
    // turn must resolve without waiting for the tool to return on its own.
    let provider = Arc::new(MockProvider::new(
        vec![BufferedReply::ToolCall {
            id: "call-1".to_string(),
            name: "slow_tool".to_string(),
            arguments: serde_json::json!({}),
        }],
        vec![],
    ));
    let mut agent = session(
        provider.clone(),
        vec![Box::new(BlockingReadTool {
            name: "slow_tool",
            delay: std::time::Duration::from_millis(300),
        })],
    );
    let cancel = agent.cancel_handle();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        cancel.store(true, Ordering::Relaxed);
    });

    let start = std::time::Instant::now();
    let result = agent.prompt("run the slow tool").await.unwrap();
    let elapsed = start.elapsed();

    assert!(matches!(result, SessionResult::Cancelled(_)));
    assert!(
        elapsed < std::time::Duration::from_millis(200),
        "expected cancel to cut the 300ms tool short, took {elapsed:?}"
    );
}

#[tokio::test]
async fn steer_mid_stream_aborts_cleanly_without_a_truncation_error() {
    // A stream that yields one delta then hangs forever (no `Done`) —
    // stands in for an LLM response still in flight when a steer message
    // lands. Exercises the truncation guard fix directly: before it,
    // `!cancelled && error.is_none() && !saw_done` mislabeled this exact
    // shape (aborted, no Done, no error) as a truncated backend response.
    let provider = Arc::new(MockProvider::new(vec![], vec![]));
    let agent = session(provider.clone(), vec![]);

    let stream: BackendEventStream = Box::pin(
        tokio_stream::once(Ok(BackendEvent::TextDelta("partial".to_string())))
            .chain(futures::stream::pending()),
    );

    let steer = agent.steer_handle();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        *steer.lock().unwrap() = Some("actually, do this instead".to_string());
    });

    let output = agent.consume_backend_stream(stream).await;

    assert!(!output.cancelled);
    assert_eq!(output.steered.as_deref(), Some("actually, do this instead"));
    assert!(output.error.is_none(), "got error: {:?}", output.error);
    assert_eq!(output.content, "partial");
}

#[tokio::test]
async fn provider_auth_failure_is_terminal_without_a_partial_session_event() {
    let provider = Arc::new(MockProvider::new(vec![BufferedReply::AuthFailure], vec![]));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut agent = session(provider.clone(), vec![]);
    let sink = events.clone();
    agent.on_event(move |event| sink.lock().unwrap().push(event));

    let error = agent.prompt("start").await.unwrap_err();

    assert!(
        error
            .to_string()
            .contains("auth error (401): mock credentials rejected")
    );
    assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    assert!(events.lock().unwrap().is_empty());
}

// ── Backend-driven contract tests (one engine, real SessionBackend) ──────

/// A scripted [`SessionBackend`] for engine-level contract tests: it records
/// each received request and replays a preset event vector per `start` call.
struct ScriptedBackend {
    caps: BackendCapabilities,
    streams: Mutex<VecDeque<Vec<Result<BackendEvent, BackendError>>>>,
    start_error: Mutex<Option<BackendError>>,
    requests: Mutex<Vec<BackendRequest>>,
}

impl ScriptedBackend {
    fn new(
        caps: BackendCapabilities,
        streams: Vec<Vec<Result<BackendEvent, BackendError>>>,
    ) -> Self {
        Self {
            caps,
            streams: Mutex::new(streams.into()),
            start_error: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn start_failure(caps: BackendCapabilities, err: BackendError) -> Self {
        Self {
            caps,
            streams: Mutex::new(VecDeque::new()),
            start_error: Mutex::new(Some(err)),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl crate::backend::SessionBackend for ScriptedBackend {
    fn name(&self) -> &str {
        "scripted-backend"
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.caps
    }
    async fn start(
        &self,
        request: &BackendRequest,
    ) -> std::result::Result<BackendEventStream, BackendError> {
        self.requests.lock().unwrap().push(request.clone());
        if let Some(err) = self.start_error.lock().unwrap().take() {
            return Err(err);
        }
        let events = self
            .streams
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![Ok(BackendEvent::Done)]);
        Ok(Box::pin(tokio_stream::iter(events)))
    }
}

/// A streaming backend that flips the session's cancel flag *while* its first
/// delta is produced, so a later event is never delivered — exercising
/// cancellation during stream consumption (not merely between turns).
struct CancelDuringStream {
    cancel: Arc<AtomicBool>,
}

#[async_trait]
impl crate::backend::SessionBackend for CancelDuringStream {
    fn name(&self) -> &str {
        "cancel-during-stream"
    }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::full_api()
    }
    async fn start(
        &self,
        _request: &BackendRequest,
    ) -> std::result::Result<BackendEventStream, BackendError> {
        let cancel = self.cancel.clone();
        let stream = futures::stream::unfold(0usize, move |i| {
            let cancel = cancel.clone();
            async move {
                match i {
                    0 => {
                        // Produce the first token and trip cancel in the same
                        // step; the engine must stop before the second token.
                        cancel.store(true, Ordering::Relaxed);
                        Some((Ok(BackendEvent::TextDelta("first".into())), 1usize))
                    }
                    1 => Some((Ok(BackendEvent::TextDelta("second".into())), 2usize)),
                    2 => Some((Ok(BackendEvent::Done), 3usize)),
                    _ => None,
                }
            }
        });
        Ok(Box::pin(stream))
    }
}

/// A no-op provider handle for sessions whose turns run through a custom
/// backend (compaction/summarization is never triggered by these tests).
fn idle_provider() -> Arc<dyn LlmProvider> {
    Arc::new(MockProvider::new(vec![], vec![]))
}

#[tokio::test]
async fn cancellation_during_stream_stops_before_the_next_event() {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(Arc::new(CancelDuringStream {
        cancel: cancel.clone(),
    }));
    agent.cancel = cancel;

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    agent.on_event(move |event| sink.lock().unwrap().push(event));

    let result = agent.prompt_stream("go").await.unwrap();

    assert!(matches!(result, SessionResult::Cancelled(_)));
    let seen = events.lock().unwrap();
    let deltas: Vec<String> = seen
        .iter()
        .filter_map(|e| match e {
            SessionEvent::TextDelta(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    // The first token surfaced; the stream was abandoned before the second.
    assert!(deltas.contains(&"first".to_string()));
    assert!(!deltas.contains(&"second".to_string()));
}

#[tokio::test]
async fn buffered_cli_backend_runs_a_terminal_turn_with_progress_then_text() {
    // A buffered CLI primary: no tools, no streaming. It emits a progress
    // note, then one final message — the engine must not offer HQ tools and
    // must end the turn terminally (no tool_start/tool_end).
    let backend = Arc::new(ScriptedBackend::new(
        BackendCapabilities::buffered_cli(),
        vec![vec![
            Ok(BackendEvent::Progress("dispatching to harness".into())),
            Ok(BackendEvent::Message("final harness answer".into())),
            Ok(BackendEvent::Done),
        ]],
    ));
    let mut agent = session(
        idle_provider(),
        vec![Box::new(ContractTool { cancel: None })],
    );
    agent.set_backend(backend.clone());

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    agent.on_event(move |event| sink.lock().unwrap().push(event));

    let result = agent.prompt("do it").await.unwrap();

    assert_eq!(result.text(), "final harness answer");
    // No HQ tool schemas were offered to the backend-managed CLI primary.
    assert!(backend.requests.lock().unwrap()[0].tools.is_empty());
    assert_eq!(
        event_names(&events.lock().unwrap()),
        vec![
            "tool_progress",
            "text_delta",
            "cost_update",
            "text_done",
            "turn_end"
        ]
    );
}

#[tokio::test]
async fn failover_before_output_shows_committed_text_exactly_once() {
    // Primary buffers a pre-output progress note, then a failoverable error.
    // The chain must discard that prelude and commit to the fallback, so the
    // session sees the answer once — no duplicate, and no primary progress.
    let primary = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![
            Ok(BackendEvent::Progress("primary warming up".into())),
            Err(BackendError::Transient("primary 503".into())),
        ]],
    ));
    let fallback = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![
            Ok(BackendEvent::TextDelta("recovered answer".into())),
            Ok(BackendEvent::Usage {
                input_tokens: 4,
                output_tokens: 2,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }),
            Ok(BackendEvent::Done),
        ]],
    ));
    let chain = ProviderChain::new("test-chain", vec![primary, fallback]);

    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(Arc::new(chain));

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    agent.on_event(move |event| sink.lock().unwrap().push(event));

    let result = agent.prompt_stream("go").await.unwrap();

    assert_eq!(result.text(), "recovered answer");
    let seen = events.lock().unwrap();
    let delta_count = seen
        .iter()
        .filter(|e| matches!(e, SessionEvent::TextDelta(t) if t == "recovered answer"))
        .count();
    assert_eq!(delta_count, 1, "committed output must appear exactly once");
    // The failed primary's pre-output progress note was never surfaced.
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolProgress { message, .. } if message.contains("warming up")))
    );
}

#[tokio::test]
async fn startup_failover_reaches_the_fallback_backend() {
    // A failoverable startup error on the primary hands off to the fallback,
    // which produces the committed answer — one engine, chain owns failover.
    let primary = Arc::new(ScriptedBackend::start_failure(
        BackendCapabilities::full_api(),
        BackendError::Unavailable("primary offline".into()),
    ));
    let fallback = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![
            Ok(BackendEvent::Message("fallback served".into())),
            Ok(BackendEvent::Done),
        ]],
    ));
    let chain = ProviderChain::new("startup-chain", vec![primary, fallback.clone()]);

    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(Arc::new(chain));

    let result = agent.prompt("go").await.unwrap();

    assert_eq!(result.text(), "fallback served");
    assert_eq!(fallback.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn buffered_backend_usage_accounts_cache_tokens() {
    // A backend that reports a prompt-cache split must have those tokens
    // accumulated into session stats (buffered API fidelity preserved).
    let backend = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![
            Ok(BackendEvent::Message("cached answer".into())),
            Ok(BackendEvent::Usage {
                input_tokens: 100,
                output_tokens: 20,
                cache_read_tokens: 80,
                cache_write_tokens: 5,
            }),
            Ok(BackendEvent::Done),
        ]],
    ));
    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(backend);

    let _ = agent.prompt("go").await.unwrap();

    let stats = agent.stats();
    assert_eq!(stats.total_input_tokens, 100);
    assert_eq!(stats.total_output_tokens, 20);
    assert_eq!(stats.total_cache_read_tokens, 80);
    assert_eq!(stats.total_cache_write_tokens, 5);
}

#[tokio::test]
async fn envelope_subscriber_receives_correlated_run_lifecycle() {
    use hq_core::types::{EnvelopeKind, EventSource};

    let provider = Arc::new(MockProvider::new(
        vec![BufferedReply::Complete("enveloped".to_string())],
        vec![],
    ));
    let mut agent = session(provider, vec![]);

    let envelopes = Arc::new(Mutex::new(Vec::new()));
    let sink = envelopes.clone();
    agent.on_envelope(move |env| sink.lock().unwrap().push(env));

    let result = agent.prompt("answer").await.unwrap();
    assert_eq!(result.text(), "enveloped");

    let seen = envelopes.lock().unwrap();
    // First envelope is the RunStarted lifecycle marker, sourced from the backend.
    assert!(matches!(
        seen.first().map(|e| &e.kind),
        Some(EnvelopeKind::RunStarted { .. })
    ));
    assert!(matches!(
        seen.first().map(|e| &e.source),
        Some(EventSource::Backend(_))
    ));
    // Last envelope is the RunFinished marker tagged with the outcome.
    assert!(matches!(
        seen.last().map(|e| &e.kind),
        Some(EnvelopeKind::RunFinished { outcome }) if outcome == "complete"
    ));
    // Sequence numbers are strictly increasing, and one run id throughout.
    let run_id = &seen[0].run_id;
    assert!(seen.iter().all(|e| &e.run_id == run_id));
    assert!(seen.windows(2).all(|w| w[0].seq < w[1].seq));
    // Standard events carry the same session event as legacy on_event.
    assert!(seen.iter().any(|e| matches!(
        e.as_session_event(),
        Some(SessionEvent::TextDone(t)) if t == "enveloped"
    )));
}

#[tokio::test]
async fn read_only_tools_execute_concurrently_not_serialized_on_the_registry_lock() {
    // Two blocking read-only tools must run in parallel. If `execute_tool`
    // held the global registry mutex across the await, the second tool could
    // not start until the first finished — doubling the wall-clock time. This
    // regression guard asserts they overlap.
    let delay = std::time::Duration::from_millis(200);
    let agent = session(
        idle_provider(),
        vec![
            Box::new(BlockingReadTool {
                name: "read_a",
                delay,
            }),
            Box::new(BlockingReadTool {
                name: "read_b",
                delay,
            }),
        ],
    );

    let calls = vec![
        ToolCall {
            id: "1".into(),
            name: "read_a".into(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            id: "2".into(),
            name: "read_b".into(),
            arguments: serde_json::json!({}),
        },
    ];

    let started = std::time::Instant::now();
    let results = agent.execute_tools_parallel(&calls).await;
    let elapsed = started.elapsed();

    assert_eq!(results.len(), 2);
    assert!(results[0].0.contains("read_a done"));
    assert!(results[1].0.contains("read_b done"));
    // Concurrent: ~200ms. Serialized (lock held across await): ~400ms. The
    // 350ms bound cleanly separates the two while tolerating scheduler jitter.
    assert!(
        elapsed < std::time::Duration::from_millis(350),
        "read-only tools serialized on the registry lock: took {elapsed:?}"
    );
}

#[tokio::test]
async fn write_then_read_preserves_execution_order() {
    // Regression: a `read` issued after a `write` must run AFTER the write so
    // it observes the write's effect. The previous strategy ran ALL read-only
    // tools in one parallel batch *before* any mutating tool, so this read
    // executed first — reading stale state. Order must now be preserved.
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let agent = session(
        idle_provider(),
        vec![
            Box::new(OrderTool {
                name: "write",
                read_only: false,
                log: log.clone(),
                // Delay the write so a mis-scheduled concurrent read (delay 0)
                // would record first if ordering were not enforced.
                delay: std::time::Duration::from_millis(120),
            }),
            Box::new(OrderTool {
                name: "read",
                read_only: true,
                log: log.clone(),
                delay: std::time::Duration::from_millis(0),
            }),
        ],
    );

    let calls = vec![
        ToolCall {
            id: "1".into(),
            name: "write".into(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            id: "2".into(),
            name: "read".into(),
            arguments: serde_json::json!({}),
        },
    ];

    let results = agent.execute_tools_parallel(&calls).await;

    // Results stay in call order.
    assert_eq!(results.len(), 2);
    assert!(results[0].0.contains("write observed"));
    assert!(results[1].0.contains("read observed"));
    // The write ran first: its snapshot has no `read` yet.
    assert!(
        !results[0].0.contains("read"),
        "write must execute before the read: {}",
        results[0].0
    );
    // The read ran after the write and observed it.
    assert!(
        results[1].0.contains("write"),
        "read must observe the preceding write: {}",
        results[1].0
    );
    // And the recorded execution order is exactly write → read.
    assert_eq!(
        *log.lock().unwrap(),
        vec!["write".to_string(), "read".to_string()]
    );
}

#[tokio::test]
async fn contiguous_reads_parallelize_around_a_write_barrier() {
    // `[read_a, read_b, write, read_c]`: the two leading reads form a
    // contiguous group and run concurrently; the write is a barrier; and the
    // trailing read runs after the write (observing it). This exercises the
    // full rule — parallelize only contiguous read-only runs separated by
    // mutating calls — while preserving order.
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let delay = std::time::Duration::from_millis(200);
    let agent = session(
        idle_provider(),
        vec![
            Box::new(OrderTool {
                name: "read_a",
                read_only: true,
                log: log.clone(),
                delay,
            }),
            Box::new(OrderTool {
                name: "read_b",
                read_only: true,
                log: log.clone(),
                delay,
            }),
            Box::new(OrderTool {
                name: "write",
                read_only: false,
                log: log.clone(),
                delay: std::time::Duration::from_millis(0),
            }),
            Box::new(OrderTool {
                name: "read_c",
                read_only: true,
                log: log.clone(),
                delay: std::time::Duration::from_millis(0),
            }),
        ],
    );

    let calls = vec![
        ToolCall {
            id: "1".into(),
            name: "read_a".into(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            id: "2".into(),
            name: "read_b".into(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            id: "3".into(),
            name: "write".into(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            id: "4".into(),
            name: "read_c".into(),
            arguments: serde_json::json!({}),
        },
    ];

    let started = std::time::Instant::now();
    let results = agent.execute_tools_parallel(&calls).await;
    let elapsed = started.elapsed();

    assert_eq!(results.len(), 4);
    // The two leading reads overlapped: ~200ms, not ~400ms serialized.
    assert!(
        elapsed < std::time::Duration::from_millis(350),
        "contiguous reads did not run concurrently: took {elapsed:?}"
    );

    let order = log.lock().unwrap().clone();
    // Both leading reads precede the write barrier.
    let write_pos = order.iter().position(|n| n == "write").unwrap();
    let read_c_pos = order.iter().position(|n| n == "read_c").unwrap();
    assert!(
        order[..write_pos].contains(&"read_a".to_string())
            && order[..write_pos].contains(&"read_b".to_string()),
        "leading reads must run before the write barrier: {order:?}"
    );
    // The trailing read runs after the write and observes it.
    assert!(
        write_pos < read_c_pos,
        "write must precede read_c: {order:?}"
    );
    assert!(
        results[3].0.contains("write"),
        "trailing read must observe the write barrier: {}",
        results[3].0
    );
}

#[tokio::test]
async fn post_output_stream_error_is_failed_not_complete() {
    // A backend that commits partial output, then errors, must resolve to the
    // non-`Complete` `Failed` result so the truncated turn is never mistaken
    // for a success — while still preserving the partial text.
    let backend = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![
            Ok(BackendEvent::TextDelta("partial answer".into())),
            Err(BackendError::Transient("connection reset".into())),
        ]],
    ));
    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(backend);

    let result = agent.prompt_stream("go").await.unwrap();

    assert!(!result.is_complete());
    assert!(result.is_failed());
    assert_eq!(result.text(), "partial answer");
    assert_eq!(result.outcome_label(), "failed");
}

#[tokio::test]
async fn stream_eof_without_done_after_output_is_failed_not_complete() {
    // The backend streams text but the stream is exhausted with no terminal
    // `Done` (a truncated turn). The engine must not treat the partial text as
    // a clean completion: it resolves to `Failed` with the partial preserved.
    let backend = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![Ok(BackendEvent::TextDelta("half a thought".into()))]],
    ));
    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(backend);

    let result = agent.prompt_stream("go").await.unwrap();

    assert!(!result.is_complete());
    assert!(result.is_failed());
    assert_eq!(result.text(), "half a thought");
    match result {
        SessionResult::Failed { error, .. } => {
            assert!(
                error.contains("truncated") || error.contains("completion marker"),
                "unexpected error text: {error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test]
async fn stream_eof_without_done_and_no_output_is_terminal_error() {
    // An empty stream (no output, no `Done`) is a truncation with nothing to
    // preserve, so the run surfaces a terminal error rather than a completion.
    let backend = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![]],
    ));
    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(backend);

    let outcome = agent.prompt_stream("go").await;
    let err = outcome.expect_err("empty truncated stream must surface a terminal error");
    let text = err.to_string();
    assert!(
        text.contains("truncated") || text.contains("completion marker"),
        "unexpected error text: {text}"
    );
}

#[tokio::test]
async fn pre_output_context_overflow_via_stream_triggers_reactive_compaction() {
    // The lazy provider chain delivers a pre-output context overflow through
    // the stream, not as a `start` failure. The engine must apply the same
    // compact-and-retry recovery: overflow on the first turn, a clean answer
    // on the retry — not a failed run.
    let backend = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![
            vec![Err(BackendError::ContextOverflow("prompt too long".into()))],
            vec![
                Ok(BackendEvent::TextDelta("recovered answer".into())),
                Ok(BackendEvent::Done),
            ],
        ],
    ));
    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(backend);

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    agent.on_event(move |event| sink.lock().unwrap().push(event));

    let result = agent.prompt_stream("go").await.unwrap();
    assert!(result.is_complete());
    assert_eq!(result.text(), "recovered answer");
    assert!(
        event_names(&events.lock().unwrap()).contains(&"context_overflow_recovery"),
        "expected a reactive compaction recovery event"
    );
}

#[tokio::test]
async fn backend_selected_retags_backend_origin_event_source() {
    use hq_core::types::EventSource;

    // The backend announces a (fallback) identity, then streams text. The
    // text is backend-origin and must carry `EventSource::Backend(selected)`;
    // cost accounting stays session-generated.
    let backend = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![
            Ok(BackendEvent::BackendSelected("fallback-x".into())),
            Ok(BackendEvent::TextDelta("hi".into())),
            Ok(BackendEvent::Usage {
                input_tokens: 3,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }),
            Ok(BackendEvent::Done),
        ]],
    ));
    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(backend);

    let envelopes = Arc::new(Mutex::new(Vec::new()));
    let sink = envelopes.clone();
    agent.on_envelope(move |env| sink.lock().unwrap().push(env));

    let _ = agent.prompt_stream("go").await.unwrap();

    let seen = envelopes.lock().unwrap();
    // Streamed text is tagged with the selected backend identity.
    assert!(seen.iter().any(|e| matches!(
        (e.as_session_event(), &e.source),
        (Some(SessionEvent::TextDelta(t)), EventSource::Backend(name))
            if t == "hi" && name == "fallback-x"
    )));
    // Cost accounting is session-generated, not backend-origin.
    assert!(seen.iter().any(|e| matches!(
        (e.as_session_event(), &e.source),
        (Some(SessionEvent::CostUpdate { .. }), EventSource::Session)
    )));
}

/// A backend whose `start()` fails a preset number of times before succeeding.
/// Exercises legacy/single-backend retry (issue: `max_retries`/backoff/fallback).
struct FlakyStartBackend {
    fails_remaining: std::sync::Mutex<u32>,
    err: BackendError,
}

#[async_trait]
impl crate::backend::SessionBackend for FlakyStartBackend {
    fn name(&self) -> &str {
        "flaky"
    }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::full_api()
    }
    async fn start(
        &self,
        _request: &BackendRequest,
    ) -> std::result::Result<BackendEventStream, BackendError> {
        {
            let mut left = self.fails_remaining.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                return Err(self.err.clone());
            }
        }
        Ok(Box::pin(tokio_stream::iter(vec![
            Ok(BackendEvent::TextDelta("recovered".into())),
            Ok(BackendEvent::Done),
        ])))
    }
}

#[tokio::test]
async fn single_backend_retries_transient_start_errors_with_retry_attempt_events() {
    let backend = Arc::new(FlakyStartBackend {
        fails_remaining: std::sync::Mutex::new(2),
        err: BackendError::Transient("overloaded".into()),
    });
    let mut agent = session(idle_provider(), vec![]);
    // The default single backend does not own failover, so session retries
    // apply. Keep the delay tiny so the test is fast.
    agent.config.max_retries = 3;
    agent.config.retry_base_delay = std::time::Duration::from_millis(1);
    agent.set_backend(backend);

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    agent.on_event(move |e| sink.lock().unwrap().push(e));

    let result = agent.prompt_stream("go").await.unwrap();
    assert_eq!(result.text(), "recovered");

    let retries = events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, SessionEvent::RetryAttempt { .. }))
        .count();
    assert_eq!(
        retries, 2,
        "two transient start failures → two RetryAttempts"
    );
}

#[tokio::test]
async fn single_backend_switches_to_fallback_model_after_retries_exhausted() {
    // Primary model always fails at start; after `max_retries` the session
    // switches to `fallback_model`, whose request the backend serves.
    struct FallbackModelBackend {
        seen_models: std::sync::Mutex<Vec<String>>,
    }
    #[async_trait]
    impl crate::backend::SessionBackend for FallbackModelBackend {
        fn name(&self) -> &str {
            "fallback-model-backend"
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::full_api()
        }
        async fn start(
            &self,
            request: &BackendRequest,
        ) -> std::result::Result<BackendEventStream, BackendError> {
            self.seen_models.lock().unwrap().push(request.model.clone());
            if request.model == "fallback-model" {
                return Ok(Box::pin(tokio_stream::iter(vec![
                    Ok(BackendEvent::TextDelta("via fallback".into())),
                    Ok(BackendEvent::Done),
                ])));
            }
            Err(BackendError::Transient("primary down".into()))
        }
    }

    let backend = Arc::new(FallbackModelBackend {
        seen_models: std::sync::Mutex::new(Vec::new()),
    });
    let mut agent = session(idle_provider(), vec![]);
    agent.config.model = "primary-model".into();
    agent.config.max_retries = 1;
    agent.config.retry_base_delay = std::time::Duration::from_millis(1);
    agent.config.fallback_model = Some("fallback-model".into());
    agent.set_backend(backend.clone());

    let result = agent.prompt_stream("go").await.unwrap();
    assert_eq!(result.text(), "via fallback");

    let seen = backend.seen_models.lock().unwrap();
    // Primary attempted (initial + one retry), then the fallback model.
    assert!(seen.iter().any(|m| m == "primary-model"));
    assert!(seen.iter().any(|m| m == "fallback-model"));
}

/// Workstream D (deterministic tool-result pruning before compaction):
/// a batch dominated by one bloated tool output should shrink under
/// `compactor::DETERMINISTIC_SKIP_THRESHOLD` once pruned, so `compact()`
/// never reaches for the LLM summarizer at all.
#[tokio::test]
async fn compact_skips_llm_call_when_pruned_batch_is_small() {
    let mock = Arc::new(MockProvider::new(vec![], vec![]));
    let mut agent = session(mock.clone(), vec![]);

    for i in 0..5 {
        agent.push_message(ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: format!("turn {i}"),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        });
    }
    agent.push_message(ChatMessage {
        image_parts: Vec::new(),
        role: MessageRole::Tool,
        content: "x".repeat(40_000),
        tool_calls: Vec::new(),
        tool_call_id: Some("tc-1".to_string()),
        reasoning_content: None,
    });
    for i in 0..15 {
        agent.push_message(ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::Assistant,
            content: format!("reply {i}"),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        });
    }

    agent.compact().await;

    assert_eq!(
        mock.calls.load(Ordering::Relaxed),
        0,
        "deterministic pruning should have avoided any LLM call"
    );
    assert!(
        agent.messages[0]
            .content
            .contains("(deterministic, no LLM)")
    );
}

/// A batch that stays large even after pruning (many oversized tool
/// outputs, not just one) still falls through to the existing
/// LLM-summarization path unchanged.
#[tokio::test]
async fn compact_falls_back_to_llm_when_pruned_batch_still_large() {
    let mock = Arc::new(MockProvider::new(
        vec![BufferedReply::Complete("a real summary".to_string())],
        vec![],
    ));
    let mut agent = session(mock.clone(), vec![]);

    for i in 0..20 {
        agent.push_message(ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::Tool,
            content: "y".repeat(40_000),
            tool_calls: Vec::new(),
            tool_call_id: Some(format!("tc-{i}")),
            reasoning_content: None,
        });
    }
    for i in 0..5 {
        agent.push_message(ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::Assistant,
            content: format!("reply {i}"),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        });
    }

    agent.compact().await;

    assert_eq!(
        mock.calls.load(Ordering::Relaxed),
        1,
        "a batch still large after pruning should still use the LLM summarizer"
    );
    assert!(agent.messages[0].content.contains("a real summary"));
}

/// FR-018: the removed 40-call guardian ceiling must stay removed. A relay
/// session (100-turn cap) making 60 distinct tool calls runs every one.
#[tokio::test]
async fn sixty_tool_calls_under_the_relay_turn_cap_are_not_denied() {
    const CALLS: usize = 60;
    let mut replies: Vec<BufferedReply> = (0..CALLS)
        .map(|i| BufferedReply::ToolCall {
            id: format!("call-{i}"),
            name: "contract_tool".to_string(),
            arguments: serde_json::json!({ "step": i }),
        })
        .collect();
    replies.push(BufferedReply::Complete("all sixty done".to_string()));
    let provider = Arc::new(MockProvider::new(replies, vec![]));
    let registry = crate::governance::ToolGuardian::with_default_mode(
        vec![std::env::current_dir().unwrap()],
        SecurityProfile::Guarded,
    )
    .build_registry(
        vec![Box::new(ContractTool { cancel: None })],
        crate::governance::LiveUserTurn::unattended(),
    );
    let config = SessionConfig {
        max_retries: 0,
        max_duration_secs: None,
        ..SessionConfig::default()
    };
    let mut agent = AgentSession::new(provider.clone(), registry, config);

    let result = agent.prompt("do sixty steps").await.unwrap();

    assert!(result.is_complete(), "{}", result.text());
    assert_eq!(result.text(), "all sixty done");
    let requests = provider.requests.lock().unwrap();
    let tool_outputs = requests
        .last()
        .unwrap()
        .messages
        .iter()
        .filter(|m| m.role == MessageRole::Tool)
        .collect::<Vec<_>>();
    assert_eq!(tool_outputs.len(), CALLS);
    assert!(tool_outputs.iter().all(|m| m.content == "tool output"));
}

/// FR-056: Turn-count limits are removed completely. An AgentSession with
/// default config runs beyond 100 turns (the old relay/web cap) AND beyond 500
/// turns (the old default ceiling) without stopping on turn count.
#[tokio::test]
async fn unbounded_session_crosses_one_hundred_and_five_hundred_turn_ceilings() {
    const CALLS: usize = 520;
    let mut replies: Vec<BufferedReply> = (0..CALLS)
        .map(|i| BufferedReply::ToolCall {
            id: format!("call-{i}"),
            name: "contract_tool".to_string(),
            arguments: serde_json::json!({ "step": i }),
        })
        .collect();
    replies.push(BufferedReply::Complete(
        "all five hundred twenty done".to_string(),
    ));
    let provider = Arc::new(MockProvider::new(replies, vec![]));
    let registry = crate::governance::ToolGuardian::with_default_mode(
        vec![std::env::current_dir().unwrap()],
        SecurityProfile::Guarded,
    )
    .build_registry(
        vec![Box::new(ContractTool { cancel: None })],
        crate::governance::LiveUserTurn::unattended(),
    );
    let config = SessionConfig {
        max_retries: 0,
        max_duration_secs: None,
        ..SessionConfig::default()
    };
    let mut agent = AgentSession::new(provider.clone(), registry, config);

    let result = agent.prompt("do 520 steps").await.unwrap();

    assert!(result.is_complete(), "expected complete, got: {:?}", result);
    assert_eq!(result.text(), "all five hundred twenty done");
    let requests = provider.requests.lock().unwrap();
    let tool_outputs = requests
        .last()
        .unwrap()
        .messages
        .iter()
        .filter(|m| m.role == MessageRole::Tool)
        .collect::<Vec<_>>();
    assert_eq!(tool_outputs.len(), CALLS);
}

/// FR-056: Explicit cancellation still halts an unbounded session cleanly.
#[tokio::test]
async fn unbounded_session_still_cancels_cleanly() {
    let replies = vec![
        BufferedReply::ToolCall {
            id: "call-1".to_string(),
            name: "contract_tool".to_string(),
            arguments: serde_json::json!({ "step": 1 }),
        },
        BufferedReply::Complete("should not reach here".to_string()),
    ];
    let provider = Arc::new(MockProvider::new(replies, vec![]));
    let registry = crate::governance::ToolGuardian::with_default_mode(
        vec![std::env::current_dir().unwrap()],
        SecurityProfile::Guarded,
    )
    .build_registry(
        vec![Box::new(ContractTool { cancel: None })],
        crate::governance::LiveUserTurn::unattended(),
    );
    let config = SessionConfig {
        max_retries: 0,
        max_duration_secs: None,
        ..SessionConfig::default()
    };
    let mut agent = AgentSession::new(provider.clone(), registry, config);
    let cancel_flag = agent.cancel_handle();
    // Signal cancellation before the prompt finishes
    cancel_flag.store(true, Ordering::Relaxed);

    let result = agent.prompt("start").await.unwrap();
    assert!(
        matches!(result, SessionResult::Cancelled(_)),
        "expected Cancelled, got {:?}",
        result
    );
}

type Readings = Arc<Mutex<VecDeque<Option<f64>>>>;

/// A tracker whose reader pops scripted readings instead of calling Copilot.
fn scripted_tracker(readings: Vec<Option<f64>>) -> crate::session::credits::CreditTracker {
    let queue: Readings = Arc::new(Mutex::new(readings.into()));
    crate::session::credits::CreditTracker::with_reader(Arc::new(move || {
        let next = queue.lock().unwrap().pop_front().flatten();
        Box::pin(async move { next })
    }))
}

fn tool_then_answer() -> Arc<MockProvider> {
    Arc::new(MockProvider::new(
        vec![
            BufferedReply::ToolCall {
                id: "call-1".to_string(),
                name: "contract_tool".to_string(),
                arguments: serde_json::json!({"input": "value"}),
            },
            BufferedReply::Complete("done".to_string()),
        ],
        vec![],
    ))
}

async fn run_with_credits(backend_name: &str, readings: Vec<Option<f64>>) -> Vec<SessionEvent> {
    let provider = tool_then_answer();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut agent = session(
        provider.clone(),
        vec![Box::new(ContractTool { cancel: None })],
    );
    agent.set_backend(Arc::new(crate::backend::ApiBackend::new(
        backend_name.to_string(),
        provider,
    )));
    agent.step_credits = Some(scripted_tracker(readings));
    let sink = events.clone();
    agent.on_event(move |event| sink.lock().unwrap().push(event));
    agent.prompt("use the tool").await.unwrap();

    events.lock().unwrap().clone()
}

type StepCreditRow = (u32, Option<f64>, Option<f64>, Option<f64>);

fn step_credits(events: &[SessionEvent]) -> Vec<StepCreditRow> {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::StepCredits {
                turn,
                credits_used_before,
                credits_used_after,
                delta,
                approximate: true,
                ..
            } => Some((*turn, *credits_used_before, *credits_used_after, *delta)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_copilot_session_emits_step_credits_from_the_injected_readings() {
    // Baseline 100, end of step 1 at 103, end of step 2 at 103.5.
    let events = run_with_credits("copilot", vec![Some(100.0), Some(103.0), Some(103.5)]).await;
    let names = event_names(&events);
    let turn_end = names.iter().position(|n| *n == "turn_end").unwrap();
    assert_eq!(names[turn_end + 1], "step_credits", "{names:?}");
    assert_eq!(
        step_credits(&events),
        vec![
            (1, Some(100.0), Some(103.0), Some(3.0)),
            (1, Some(103.0), Some(103.5), Some(0.5)),
        ]
    );
}

#[tokio::test]
async fn a_failing_reader_gives_delta_none_and_the_step_still_completes() {
    let events = run_with_credits("copilot", vec![None, None, None]).await;
    let credits = step_credits(&events);
    assert_eq!(credits.len(), 2);
    assert!(credits.iter().all(|(_, _, _, delta)| delta.is_none()));
    assert!(event_names(&events).contains(&"text_done"));
}

#[tokio::test]
async fn a_non_copilot_backend_emits_no_step_credits() {
    let events = run_with_credits("session-contract-mock", vec![Some(1.0), Some(2.0)]).await;
    assert!(step_credits(&events).is_empty());
}

#[tokio::test]
async fn a_provider_billed_cost_is_what_the_session_budget_counts() {
    const BILLED_USD: f64 = 0.4321;
    let backend = Arc::new(ScriptedBackend::new(
        BackendCapabilities::full_api(),
        vec![vec![
            Ok(BackendEvent::Message("answer".into())),
            Ok(BackendEvent::Usage {
                input_tokens: 100,
                output_tokens: 20,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }),
            Ok(BackendEvent::Billing {
                cost_usd: Some(BILLED_USD),
                reasoning_tokens: 4,
            }),
            Ok(BackendEvent::Done),
        ]],
    ));
    let mut agent = session(idle_provider(), vec![]);
    agent.set_backend(backend);

    let _ = agent.prompt("go").await.unwrap();

    assert_eq!(agent.total_cost(), BILLED_USD);
}
