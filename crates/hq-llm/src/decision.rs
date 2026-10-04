//! Structured-decision ("System 1") client. Typed yes/no and choice questions
//! come back as calibrated probabilities instead of generated text, which makes
//! it a cheap gate to run before an expensive generative call.
//!
//! Definition: [`DecisionProvider`]. Provider: [`HttpDecisionProvider`], one
//! struct for every route because OpenRouter and TypeSafe share a wire format.
//! Consumers hold a [`Decisions`] handle and never see a URL or a model id.

use crate::http::SHARED_HTTP_CLIENT;
use crate::provider::LlmError;
use async_trait::async_trait;
use hq_core::config::{DecisionMode, DecisionRoute, DecisionsConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// Jev's context window is 32k tokens; this keeps `state` well inside it.
const MAX_STATE_CHARS: usize = 48_000;
const MAX_ERROR_BODY_CHARS: usize = 200;
const SHADOW_CONCURRENCY: usize = 4;
const SHADOW_LOG_DIR: &str = "_system/decision-shadow";

/// Reserved label [`DecisionRequest::choice_or_abstain`] adds to every option
/// list it builds. Offering it redistributes probability mass across the whole
/// option set, so a site using it calibrates thresholds against that set, not
/// just against the model version.
pub const UNCLEAR: &str = "unclear";

#[derive(Debug, Clone, PartialEq)]
pub enum QuestionKind {
    Noul,
    /// (option, description) pairs; an empty description is sent as null.
    Choice(Vec<(String, String)>),
}

#[derive(Debug, Clone)]
pub struct Question {
    pub kind: QuestionKind,
    pub instructions: String,
}

#[derive(Debug, Clone, Default)]
pub struct DecisionRequest {
    pub state: String,
    pub questions: BTreeMap<String, Question>,
}

impl DecisionRequest {
    pub fn new(state: &str) -> Self {
        Self {
            state: state.chars().take(MAX_STATE_CHARS).collect(),
            questions: BTreeMap::new(),
        }
    }

    pub fn noul(mut self, key: &str, instructions: &str) -> Self {
        self.questions.insert(
            key.to_string(),
            Question {
                kind: QuestionKind::Noul,
                instructions: instructions.to_string(),
            },
        );
        self
    }

    pub fn choice(mut self, key: &str, instructions: &str, options: &[(&str, &str)]) -> Self {
        let options = options
            .iter()
            .map(|(label, desc)| (label.to_string(), desc.to_string()))
            .collect();
        self.questions.insert(
            key.to_string(),
            Question {
                kind: QuestionKind::Choice(options),
                instructions: instructions.to_string(),
            },
        );
        self
    }

    /// Like [`choice`](Self::choice), but always offers [`UNCLEAR`] alongside
    /// `options`, mirroring jev-browser's model-selectable `REVIEW` escape hatch.
    /// Without it, a model facing a state that fits none of the options must
    /// still pick one, so a genuine "I can't tell" is indistinguishable from a
    /// confident pick that happens to score low. `abstain_hint` tells the model
    /// when to reach for it instead of guessing. Read the result with
    /// [`DecisionResponse::choice_or_abstain`], and record the outcome even when
    /// it abstains: that log line is the entire point of asking this way.
    pub fn choice_or_abstain(
        self,
        key: &str,
        instructions: &str,
        options: &[(&str, &str)],
        abstain_hint: &str,
    ) -> Self {
        let mut all = options.to_vec();
        all.push((UNCLEAR, abstain_hint));
        self.choice(key, instructions, &all)
    }

    fn to_body(&self, model: &str) -> Value {
        let questions: Map<String, Value> = self
            .questions
            .iter()
            .map(|(key, q)| (key.clone(), question_json(q)))
            .collect();
        json!({ "model": model, "state": self.state, "questions": questions })
    }
}

fn question_json(q: &Question) -> Value {
    match &q.kind {
        QuestionKind::Noul => json!({ "type": "noul", "instructions": q.instructions }),
        QuestionKind::Choice(options) => {
            let criteria: Map<String, Value> = options
                .iter()
                .map(|(label, desc)| {
                    let desc = if desc.is_empty() { Value::Null } else { json!(desc) };
                    (label.clone(), desc)
                })
                .collect();
            json!({ "type": "choice", "instructions": q.instructions, "criteria": criteria })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    /// Calibrated probability in 0..=1 that the statement is true. Carries no confidence.
    Noul { noul: f64 },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
}

/// Result of [`DecisionResponse::choice_or_abstain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceOrAbstain<'a> {
    Picked(&'a str),
    /// The model chose [`UNCLEAR`] rather than guess among the offered options.
    Unclear,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    /// Only present on gateways that bill per call (OpenRouter).
    #[serde(default)]
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResponse {
    /// The build that answered, e.g. `typesafe/jev-1.13-20260917`. Differs from the
    /// requested id, so compare runs on this value.
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error(transparent)]
    Http(#[from] LlmError),
    #[error("undecodable decision response: {0}")]
    Decode(String),
    #[error("invalid decision response: {0}")]
    Invalid(String),
}

impl DecisionResponse {
    pub fn noul(&self, key: &str) -> Result<f64, DecisionError> {
        match self.answers.get(key) {
            Some(Answer::Noul { noul }) => Ok(*noul),
            _ => Err(DecisionError::Invalid(format!("no yes/no answer for `{key}`"))),
        }
    }

    pub fn choice(&self, key: &str) -> Result<&str, DecisionError> {
        match self.answers.get(key) {
            Some(Answer::Choice { choice, .. }) => Ok(choice),
            _ => Err(DecisionError::Invalid(format!("no choice answer for `{key}`"))),
        }
    }

    /// Reads the answer to a question built with
    /// [`DecisionRequest::choice_or_abstain`], separating a confident pick from
    /// the model choosing [`UNCLEAR`]. An `Err` here still means the same as for
    /// [`choice`](Self::choice) (provider failure, timeout, unusable answer);
    /// `Ok(ChoiceOrAbstain::Unclear)` means the call succeeded and the model
    /// declined. Record both outcomes before falling back to the incumbent
    /// behavior, or the distinction this method exists for is lost.
    pub fn choice_or_abstain(&self, key: &str) -> Result<ChoiceOrAbstain<'_>, DecisionError> {
        match self.choice(key)? {
            UNCLEAR => Ok(ChoiceOrAbstain::Unclear),
            picked => Ok(ChoiceOrAbstain::Picked(picked)),
        }
    }

    /// Every asked question must be answered with the right type and a known label.
    fn validate(&self, req: &DecisionRequest) -> Result<(), DecisionError> {
        for (key, question) in &req.questions {
            match (&question.kind, self.answers.get(key)) {
                (QuestionKind::Noul, Some(Answer::Noul { noul })) if (0.0..=1.0).contains(noul) => {}
                (QuestionKind::Choice(options), Some(Answer::Choice { choice, .. }))
                    if options.iter().any(|(label, _)| label == choice) => {}
                (_, None) => {
                    return Err(DecisionError::Invalid(format!("missing answer for `{key}`")));
                }
                _ => {
                    return Err(DecisionError::Invalid(format!("unusable answer for `{key}`")));
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
pub trait DecisionProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError>;
}

pub struct HttpDecisionProvider {
    route: DecisionRoute,
    api_key: String,
    timeout: Duration,
}

impl HttpDecisionProvider {
    pub fn new(route: DecisionRoute, api_key: String, timeout: Duration) -> Self {
        Self {
            route,
            api_key,
            timeout,
        }
    }
}

#[async_trait]
impl DecisionProvider for HttpDecisionProvider {
    fn name(&self) -> &str {
        &self.route.endpoint
    }

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        let response = SHARED_HTTP_CLIENT
            .post(&self.route.endpoint)
            .bearer_auth(&self.api_key)
            .timeout(self.timeout)
            .json(&request.to_body(&self.route.model))
            .send()
            .await
            .map_err(|e| LlmError::from_request_error(&e))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| LlmError::from_request_error(&e))?;
        if !status.is_success() {
            // Truncate on a char boundary: `from_http` slices bytes and would panic mid-codepoint.
            let body: String = text.chars().take(MAX_ERROR_BODY_CHARS).collect();
            return Err(LlmError::from_http(status.as_u16(), &body).into());
        }
        let parsed: DecisionResponse =
            serde_json::from_str(&text).map_err(|e| DecisionError::Decode(e.to_string()))?;
        parsed.validate(request)?;
        Ok(parsed)
    }
}

/// Routes are equivalent alternatives, so any failure moves on to the next one.
// ponytail: no per-route health or cooldown, so a dead first route costs a failed call each time.
pub struct DecisionChain(Vec<Arc<dyn DecisionProvider>>);

#[async_trait]
impl DecisionProvider for DecisionChain {
    fn name(&self) -> &str {
        "chain"
    }

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        let mut last = DecisionError::Invalid("no decision routes configured".to_string());
        for provider in &self.0 {
            match provider.decide(request).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    tracing::warn!(route = provider.name(), error = %e, "decision route failed");
                    last = e;
                }
            }
        }
        Err(last)
    }
}

/// What consumers hold: a provider plus per-site policy and the shadow log.
pub struct Decisions {
    provider: Arc<dyn DecisionProvider>,
    config: DecisionsConfig,
    log_dir: PathBuf,
    shadow_slots: Arc<Semaphore>,
}

impl Decisions {
    pub fn new(
        provider: Arc<dyn DecisionProvider>,
        config: DecisionsConfig,
        vault_path: &Path,
    ) -> Self {
        Self {
            provider,
            config,
            log_dir: vault_path.join(SHADOW_LOG_DIR),
            shadow_slots: Arc::new(Semaphore::new(SHADOW_CONCURRENCY)),
        }
    }

    pub fn mode(&self, site: &str) -> DecisionMode {
        self.config.site_mode(site)
    }

    pub fn threshold(&self, site: &str, builtin: f64) -> f64 {
        self.config.site_threshold(site, builtin)
    }

    /// One overall deadline for the whole chain, so an inline caller never waits longer.
    pub async fn ask(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        let deadline = Duration::from_millis(self.config.timeout_ms);
        match tokio::time::timeout(deadline, self.provider.decide(request)).await {
            Ok(result) => result,
            Err(_) => Err(LlmError::Network("decision request timed out".to_string()).into()),
        }
    }

    /// Runs the question in the background and logs it beside the incumbent's
    /// decision. Never blocks the caller; at most `SHADOW_CONCURRENCY` calls are in
    /// flight and the rest wait for a slot, so a burst is delayed rather than lost.
    pub fn shadow(self: &Arc<Self>, site: &'static str, request: DecisionRequest, incumbent: Value) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let Ok(_permit) = this.shadow_slots.clone().acquire_owned().await else {
                return;
            };
            let started = Instant::now();
            let outcome = this.ask(&request).await;
            let mut entry = json!({
                "incumbent": incumbent,
                "latency_ms": started.elapsed().as_millis() as u64,
            });
            match outcome {
                Ok(r) => {
                    entry["model"] = json!(r.model);
                    entry["cost"] = json!(r.usage.cost);
                    entry["answers"] = json!(r.answers);
                }
                Err(e) => entry["error"] = json!(e.to_string()),
            }
            this.record(site, entry);
        });
    }

    /// Appends one JSON line to the day's decision log. Logging never fails the caller.
    pub fn record(&self, site: &str, mut entry: Value) {
        entry["ts"] = json!(chrono::Utc::now().to_rfc3339());
        entry["site"] = json!(site);
        let path = self
            .log_dir
            .join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d")));
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(&self.log_dir)?;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            writeln!(file, "{entry}")
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, path = %path.display(), "decision log write failed");
        }
    }
}

/// `None` when decisions are disabled or no route has a usable credential.
pub fn build(config: &DecisionsConfig, vault_path: &Path) -> Option<Arc<Decisions>> {
    if !config.enabled {
        return None;
    }
    let timeout = Duration::from_millis(config.timeout_ms);
    let providers: Vec<Arc<dyn DecisionProvider>> = config
        .routes
        .iter()
        .filter_map(|route| {
            let key = std::env::var(&route.credential_env)
                .ok()
                .filter(|k| !k.trim().is_empty());
            if key.is_none() {
                tracing::warn!(env = %route.credential_env, "decision route skipped: credential env var is unset");
            }
            key.map(|k| {
                Arc::new(HttpDecisionProvider::new(route.clone(), k, timeout))
                    as Arc<dyn DecisionProvider>
            })
        })
        .collect();
    if providers.is_empty() {
        tracing::warn!("decisions are enabled but no route has a usable credential");
        return None;
    }
    let chain = Arc::new(DecisionChain(providers));
    Some(Arc::new(Decisions::new(chain, config.clone(), vault_path)))
}

static GLOBAL: OnceLock<Option<Arc<Decisions>>> = OnceLock::new();

/// Call once at daemon startup; later calls are ignored.
pub fn init(config: &DecisionsConfig, vault_path: &Path) {
    let _ = GLOBAL.set(build(config, vault_path));
}

pub fn get() -> Option<Arc<Decisions>> {
    GLOBAL.get().and_then(Clone::clone)
}

/// Test double for consumer tests in other crates: answers every request with
/// the canned answers, or fails when `fail` is set.
#[derive(Default)]
pub struct FakeDecisionProvider {
    pub answers: BTreeMap<String, Answer>,
    pub fail: bool,
}

#[async_trait]
impl DecisionProvider for FakeDecisionProvider {
    fn name(&self) -> &str {
        "fake"
    }

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        if self.fail {
            return Err(DecisionError::Decode("fake failure".to_string()));
        }
        let response = DecisionResponse {
            model: "fake".to_string(),
            answers: self.answers.clone(),
            usage: Usage::default(),
        };
        response.validate(request)?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Captured from OpenRouter on 2026-09-18 with model `typesafe/jev-1.13`.
    const LIVE_RESPONSE: &str = r#"{"model":"typesafe/jev-1.13-20260917","answers":{"should_interrupt":{"type":"noul","noul":0.13},"urgency":{"type":"choice","choice":"routine","probabilities":{"critical":0,"routine":1,"actionable":0},"confidence":1}},"usage":{"input_tokens":372,"output_tokens":58,"cost":0.000015624},"id":"gen-dec-1789755789-Ypwiuky2BMcuwIftgofE","provider":"TypeSafe"}"#;

    fn request() -> DecisionRequest {
        DecisionRequest::new("Weekly newsletter")
            .noul("should_interrupt", "Interrupt the owner now?")
            .choice(
                "urgency",
                "How urgent?",
                &[("routine", "newsletters"), ("actionable", ""), ("critical", "outage")],
            )
    }

    fn provider_for(server: &MockServer, timeout_ms: u64) -> HttpDecisionProvider {
        let route = DecisionRoute {
            endpoint: format!("{}/api/alpha/decisions", server.uri()),
            model: "typesafe/jev-1.13".to_string(),
            credential_env: "UNUSED".to_string(),
        };
        HttpDecisionProvider::new(route, "test-key".to_string(), Duration::from_millis(timeout_ms))
    }

    #[test]
    fn parses_the_captured_live_response() {
        let parsed: DecisionResponse = serde_json::from_str(LIVE_RESPONSE).unwrap();
        parsed.validate(&request()).unwrap();
        assert_eq!(parsed.model, "typesafe/jev-1.13-20260917");
        assert_eq!(parsed.noul("should_interrupt").unwrap(), 0.13);
        assert_eq!(parsed.choice("urgency").unwrap(), "routine");
        assert_eq!(parsed.usage.cost, Some(0.000015624));
    }

    #[test]
    fn validate_rejects_missing_wrong_type_and_unknown_label() {
        let mut parsed: DecisionResponse = serde_json::from_str(LIVE_RESPONSE).unwrap();
        let req = request();

        let mut unknown_label = parsed.clone();
        unknown_label.answers.insert(
            "urgency".into(),
            Answer::Choice {
                choice: "nope".into(),
                probabilities: BTreeMap::new(),
                confidence: 1.0,
            },
        );
        assert!(unknown_label.validate(&req).is_err());

        let mut wrong_type = parsed.clone();
        wrong_type
            .answers
            .insert("urgency".into(), Answer::Noul { noul: 0.5 });
        assert!(wrong_type.validate(&req).is_err());

        parsed.answers.remove("should_interrupt");
        assert!(parsed.validate(&req).is_err());
    }

    #[test]
    fn request_body_matches_the_wire_format() {
        let body = request().to_body("typesafe/jev-1.13");
        assert_eq!(body["model"], "typesafe/jev-1.13");
        assert_eq!(body["questions"]["should_interrupt"]["type"], "noul");
        assert_eq!(body["questions"]["urgency"]["criteria"]["routine"], "newsletters");
        assert!(body["questions"]["urgency"]["criteria"]["actionable"].is_null());
    }

    #[test]
    fn choice_or_abstain_adds_the_unclear_option_to_the_wire_body() {
        let req = DecisionRequest::new("state").choice_or_abstain(
            "route",
            "Which harness fits?",
            &[("fast", "cheap and quick"), ("careful", "slow and thorough")],
            "Neither the task nor the state gives a clear signal.",
        );
        let body = req.to_body("typesafe/jev-1.13");
        assert_eq!(
            body["questions"]["route"]["criteria"]["unclear"],
            "Neither the task nor the state gives a clear signal."
        );
        assert_eq!(body["questions"]["route"]["criteria"]["fast"], "cheap and quick");
    }

    #[test]
    fn choice_or_abstain_distinguishes_a_pick_from_unclear() {
        let req = DecisionRequest::new("state").choice_or_abstain(
            "route",
            "Which harness fits?",
            &[("fast", "cheap and quick")],
            "no signal",
        );

        let mut picked = choice_response(&req, "route", "fast");
        assert_eq!(picked.choice_or_abstain("route").unwrap(), ChoiceOrAbstain::Picked("fast"));

        picked.answers.insert(
            "route".into(),
            Answer::Choice {
                choice: UNCLEAR.into(),
                probabilities: BTreeMap::new(),
                confidence: 0.4,
            },
        );
        assert_eq!(picked.choice_or_abstain("route").unwrap(), ChoiceOrAbstain::Unclear);
    }

    /// Builds a minimal valid response answering only `key` with `choice`, for
    /// tests that don't need the full `LIVE_RESPONSE` fixture's question set.
    fn choice_response(req: &DecisionRequest, key: &str, choice: &str) -> DecisionResponse {
        let mut answers = BTreeMap::new();
        answers.insert(
            key.to_string(),
            Answer::Choice {
                choice: choice.to_string(),
                probabilities: BTreeMap::new(),
                confidence: 1.0,
            },
        );
        let response = DecisionResponse { model: "fake".to_string(), answers, usage: Usage::default() };
        response.validate(req).unwrap();
        response
    }

    #[test]
    fn state_is_capped_on_a_char_boundary() {
        let req = DecisionRequest::new(&"é".repeat(MAX_STATE_CHARS + 10));
        assert_eq!(req.state.chars().count(), MAX_STATE_CHARS);
    }

    #[tokio::test]
    async fn http_provider_sends_auth_and_decodes_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/alpha/decisions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(LIVE_RESPONSE))
            .mount(&server)
            .await;
        let response = provider_for(&server, 2000).decide(&request()).await.unwrap();
        assert_eq!(response.choice("urgency").unwrap(), "routine");
    }

    #[tokio::test]
    async fn http_provider_maps_failures_to_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"error":{"message":"Model ~typesafe/jev-1.13 does not exist","code":400}}"#,
            ))
            .mount(&server)
            .await;
        let err = provider_for(&server, 2000).decide(&request()).await.unwrap_err();
        assert!(matches!(err, DecisionError::Http(_)));

        let bad = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&bad)
            .await;
        let err = provider_for(&bad, 2000).decide(&request()).await.unwrap_err();
        assert!(matches!(err, DecisionError::Decode(_)));

        let slow = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(LIVE_RESPONSE)
                    .set_delay(Duration::from_millis(600)),
            )
            .mount(&slow)
            .await;
        let err = provider_for(&slow, 100).decide(&request()).await.unwrap_err();
        assert!(matches!(err, DecisionError::Http(LlmError::Network(_))));
    }

    #[tokio::test]
    async fn chain_falls_over_to_the_next_route() {
        let dead = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&dead)
            .await;
        let live = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(LIVE_RESPONSE))
            .mount(&live)
            .await;
        let chain = DecisionChain(vec![
            Arc::new(provider_for(&dead, 2000)),
            Arc::new(provider_for(&live, 2000)),
        ]);
        assert!(chain.decide(&request()).await.is_ok());
    }

    fn fake_decisions(fake: FakeDecisionProvider, dir: &Path, timeout_ms: u64) -> Arc<Decisions> {
        let config = DecisionsConfig {
            timeout_ms,
            ..DecisionsConfig::default()
        };
        Arc::new(Decisions::new(Arc::new(fake), config, dir))
    }

    #[tokio::test]
    async fn shadow_records_incumbent_and_answers_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeDecisionProvider {
            answers: BTreeMap::from([("should_interrupt".to_string(), Answer::Noul { noul: 0.2 })]),
            fail: false,
        };
        let decisions = fake_decisions(fake, dir.path(), 1000);
        let req = DecisionRequest::new("x").noul("should_interrupt", "?");
        decisions.shadow("test_site", req, json!({"decision": "urgent"}));

        let log_dir = dir.path().join(SHADOW_LOG_DIR);
        let mut lines = String::new();
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if let Some(entry) = std::fs::read_dir(&log_dir).ok().and_then(|mut d| d.next()) {
                lines = std::fs::read_to_string(entry.unwrap().path()).unwrap_or_default();
                if !lines.is_empty() {
                    break;
                }
            }
        }
        let entry: Value = serde_json::from_str(lines.lines().next().unwrap()).unwrap();
        assert_eq!(entry["site"], "test_site");
        assert_eq!(entry["incumbent"]["decision"], "urgent");
        assert_eq!(entry["answers"]["should_interrupt"]["noul"], 0.2);
    }

    #[tokio::test]
    async fn ask_surfaces_provider_errors_for_fail_open_callers() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeDecisionProvider {
            fail: true,
            ..Default::default()
        };
        let decisions = fake_decisions(fake, dir.path(), 1000);
        assert!(decisions.ask(&DecisionRequest::new("x")).await.is_err());
    }

    #[test]
    fn build_is_none_when_disabled_or_uncredentialed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(build(&DecisionsConfig::default(), dir.path()).is_none());
        let mut config = DecisionsConfig {
            enabled: true,
            ..DecisionsConfig::default()
        };
        config.routes[0].credential_env = "HQ_TEST_DECISION_KEY_DEFINITELY_UNSET".to_string();
        assert!(build(&config, dir.path()).is_none());
    }
}
