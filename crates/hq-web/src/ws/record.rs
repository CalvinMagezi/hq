//! What a web chat reply did, kept while it streams: its text, reasoning and
//! tool calls. Saved as the reply's `meta` so a reload shows the same thing,
//! and saved exactly once, by whichever of finish or Stop gets there first.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use hq_core::types::SessionEvent;
use serde_json::{Map, Value, json};

/// Caps on what one reply stores, so a 50-step turn stays small on a phone.
const MAX_STEPS: usize = 200;
const MAX_STEP_ARGS_CHARS: usize = 2_000;
const MAX_STEP_RESULT_CHARS: usize = 1_000;
const MAX_REASONING_CHARS: usize = 20_000;
/// Longest tool result sent live to the browser; the full result stays in the session.
pub(super) const LIVE_RESULT_CHARS: usize = 4_000;
const REDACTED: &str = "[REDACTED]";
/// Argument keys whose values are credentials whatever they look like.
const SECRET_KEY_HINTS: &[&str] = &[
    "token", "secret", "password", "passwd", "apikey", "api_key", "authorization", "cookie", "credential",
    "private_key",
];

pub(super) type SharedRecord = Arc<Mutex<TurnRecord>>;

#[derive(Default)]
pub(super) struct TurnRecord {
    content: String,
    reasoning: String,
    steps: Vec<StepRecord>,
    /// Approximate Copilot credits per agent step, in order.
    credits: Vec<Value>,
    /// How many tool steps earlier credit events already covered.
    credited_steps: usize,
    /// Set by the one save that happens; a second caller finds it and backs off.
    saved: bool,
    /// For a reply the session driver started: which session and why.
    driver: Option<Value>,
}

struct StepRecord {
    id: String,
    name: String,
    args: String,
    result: Option<String>,
    started: Instant,
    duration_ms: Option<u64>,
}

/// A reply ready to be written: its text and, when it did more than talk, its meta.
pub(super) struct ReplyToSave {
    pub content: String,
    pub meta: Option<Value>,
}

impl TurnRecord {
    pub(super) fn with_driver(driver: Value) -> Self {
        Self {
            driver: Some(driver),
            ..Self::default()
        }
    }

    /// The `parent_turn_id` a sub-agent follow-up turn carries, so children it
    /// starts inherit the follow-up depth.
    pub(super) fn followup_turn_id(&self) -> Option<String> {
        self.driver
            .as_ref()?
            .get("turn_id")?
            .as_str()
            .map(str::to_string)
    }

    /// Whether the session driver started this reply, not the user.
    /// Whether the session driver (not a sub-agent follow-up) started this reply.
    pub(super) fn is_session_driver_turn(&self) -> bool {
        self.driver.as_ref().is_some_and(|d| d.get("session_id").is_some())
    }

    pub(super) fn is_driver_turn(&self) -> bool {
        self.driver.is_some()
    }

    pub(super) fn observe(&mut self, event: &SessionEvent) {
        match event {
            SessionEvent::TextDelta(s) => self.content.push_str(s),
            SessionEvent::Reasoning(s) if self.reasoning.chars().count() < MAX_REASONING_CHARS => {
                self.reasoning.push_str(s);
            }
            SessionEvent::ToolStart { tool_name, tool_call_id, arguments } if self.steps.len() < MAX_STEPS => {
                self.steps.push(StepRecord {
                    id: tool_call_id.clone(),
                    name: tool_name.clone(),
                    args: args_preview(arguments, MAX_STEP_ARGS_CHARS),
                    result: None,
                    started: Instant::now(),
                    duration_ms: None,
                });
            }
            SessionEvent::StepCredits { .. } if self.credits.len() < MAX_STEPS => self.record_credits(event),
            SessionEvent::ToolEnd { tool_call_id, result, .. } => {
                if let Some(step) = self.steps.iter_mut().find(|s| &s.id == tool_call_id) {
                    step.result = Some(result_preview(result, MAX_STEP_RESULT_CHARS));
                    step.duration_ms = Some(step.started.elapsed().as_millis() as u64);
                }
            }
            _ => {}
        }
    }

    /// Keeps a step's credits with the first tool call it started, so a reload shows the badge in place.
    fn record_credits(&mut self, event: &SessionEvent) {
        let SessionEvent::StepCredits { turn, delta, input_tokens, output_tokens, model, .. } = event else {
            return;
        };
        let mut entry = json!({
            "turn": turn, "delta": delta, "input_tokens": input_tokens, "output_tokens": output_tokens, "model": model,
        });
        if let Some(first) = self.steps.get(self.credited_steps) {
            entry["tool_call_id"] = Value::String(first.id.clone());
        }
        self.credited_steps = self.steps.len();
        self.credits.push(entry);
    }

    /// Takes the reply for saving, once. `final_text` replaces the streamed text
    /// when the run returned its own (it can hold text that never streamed).
    /// None when it was already taken or there is nothing to keep.
    pub(super) fn take(&mut self, final_text: Option<&str>, stopped: bool) -> Option<ReplyToSave> {
        if self.saved {
            return None;
        }
        self.saved = true;
        let content = final_text.filter(|t| !t.is_empty()).unwrap_or(&self.content).to_string();
        // A stop flag alone is not worth a message: stopped before anything happened saves nothing.
        let did_work = !self.steps.is_empty() || !self.reasoning.trim().is_empty();
        if content.trim().is_empty() && !did_work {
            return None;
        }
        Some(ReplyToSave { content, meta: self.meta(stopped) })
    }

    fn meta(&self, stopped: bool) -> Option<Value> {
        let mut meta = Map::new();
        let reasoning: String = self.reasoning.chars().take(MAX_REASONING_CHARS).collect();
        if !reasoning.trim().is_empty() {
            meta.insert("reasoning".into(), Value::String(reasoning));
        }
        if !self.steps.is_empty() {
            let steps = self.steps.iter().map(StepRecord::to_json).collect();
            meta.insert("tool_steps".into(), Value::Array(steps));
        }
        if !self.credits.is_empty() {
            meta.insert("step_credits".into(), Value::Array(self.credits.clone()));
        }
        if stopped {
            meta.insert("stopped".into(), Value::Bool(true));
        }
        if let Some(driver) = &self.driver {
            meta.insert("driver".into(), driver.clone());
        }
        (!meta.is_empty()).then_some(Value::Object(meta))
    }
}

impl StepRecord {
    fn to_json(&self) -> Value {
        let mut v = json!({ "id": self.id, "name": self.name, "args": self.args });
        if let Some(result) = &self.result {
            v["result"] = Value::String(result.clone());
        }
        if let Some(ms) = self.duration_ms {
            v["duration_ms"] = Value::from(ms);
        }
        v
    }
}

/// Tool arguments as JSON text a person may see: credential-named values
/// blanked, known secret shapes redacted, cut to `cap` characters.
pub(super) fn args_preview(arguments: &Value, cap: usize) -> String {
    let text = scrub(arguments).to_string();
    cap_chars(&hq_core::redact::redact_secrets(&text), cap)
}

/// A tool result a person may see: known secret shapes redacted, cut to `cap` characters.
pub(super) fn result_preview(result: &str, cap: usize) -> String {
    hq_core::redact::redact_secrets(&cap_chars(result, cap))
}

fn cap_chars(text: &str, cap: usize) -> String {
    text.chars().take(cap).collect()
}

fn names_a_secret(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SECRET_KEY_HINTS.iter().any(|hint| name.contains(hint))
}

/// Settings tools pass `{"key": "openrouter_api_key", "value": "..."}`: the
/// secret sits under `value`, and only its sibling says so.
const SETTING_NAME_FIELDS: &[&str] = &["key", "name", "field", "setting"];
const SETTING_VALUE_FIELD: &str = "value";

fn scrub(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let secret_setting = SETTING_NAME_FIELDS
                .iter()
                .any(|f| map.get(*f).and_then(Value::as_str).is_some_and(names_a_secret));
            Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let secret = names_a_secret(k) || (secret_setting && k == SETTING_VALUE_FIELD);
                        (k.clone(), if secret { Value::String(REDACTED.into()) } else { scrub(v) })
                    })
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(scrub).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(id: &str, args: Value) -> SessionEvent {
        SessionEvent::ToolStart { tool_name: "bash".into(), tool_call_id: id.into(), arguments: args }
    }

    fn end(id: &str, result: &str) -> SessionEvent {
        SessionEvent::ToolEnd { tool_name: "bash".into(), tool_call_id: id.into(), result: result.into() }
    }

    #[test]
    fn only_the_session_driver_is_a_session_driver_turn() {
        let session = TurnRecord::with_driver(json!({"session_id": "hs-1", "mode": "drive"}));
        let followup = TurnRecord::with_driver(json!({"mode": "followup", "turn_id": "t"}));
        assert!(session.is_driver_turn() && session.is_session_driver_turn());
        assert!(followup.is_driver_turn() && !followup.is_session_driver_turn());
        assert!(!TurnRecord::default().is_session_driver_turn());
    }

    #[test]
    fn credential_arguments_never_reach_the_page_or_the_store() {
        let args = json!({
            "command": "curl -H 'Authorization: Bearer abcdefghijklmnop1234' https://x", // gitleaks:allow
            "config": {"api_key": "plain-looking", "API_TOKEN": "x", "name": "ok"},
            "list": [{"password": "hunter2"}],
        });
        let shown = args_preview(&args, 10_000);
        assert!(!shown.contains("plain-looking") && !shown.contains("hunter2"), "{shown}");
        assert!(!shown.contains("abcdefghijklmnop1234"), "{shown}");
        assert!(shown.contains("\"name\":\"ok\""), "{shown}");

        let result = result_preview("key sk-abcdefghijklmnopqrstuv done", 1_000); // gitleaks:allow
        assert!(!result.contains("sk-abcdefghijklmnopqrstuv"), "{result}"); // gitleaks:allow
    }

    #[test]
    fn real_provider_keys_and_config_lines_are_redacted() {
        let openrouter = "sk-or-v1-0123456789abcdef0123456789abcdef0123456789abcdef"; // gitleaks:allow
        let anthropic = "sk-ant-api03-AbCdEf0123456789_AbCdEf0123456789-xyz"; // gitleaks:allow
        let args = json!({
            "command": format!("OPENROUTER_API_KEY={openrouter} hq doctor"), // gitleaks:allow
            "settings": {"key": "openrouter_api_key", "value": "plainvalue123"},
            "note": format!("key is {anthropic}"),
        });
        let shown = args_preview(&args, 10_000);
        for secret in [openrouter, anthropic, "plainvalue123"] {
            assert!(!shown.contains(secret), "{secret} leaked in {shown}");
        }

        let file = format!("openrouter_api_key: {openrouter}\nGITHUB_TOKEN=\"ghx_notatypicalshape99\"\nmodel: sonnet"); // gitleaks:allow
        let result = result_preview(&file, 10_000);
        assert!(!result.contains(openrouter) && !result.contains("ghx_notatypicalshape99"), "{result}");
        assert!(result.contains("model: sonnet"), "{result}");
    }

    #[test]
    fn a_reply_with_tools_keeps_capped_steps_and_reasoning() {
        let mut r = TurnRecord::default();
        r.observe(&SessionEvent::Reasoning("thinking".into()));
        r.observe(&start("c1", json!({"command": "x".repeat(5_000)})));
        r.observe(&end("c1", &"y".repeat(5_000)));

        let saved = r.take(Some(""), false).expect("tools-only reply is kept");
        assert_eq!(saved.content, "");
        let meta = saved.meta.unwrap();
        assert_eq!(meta["reasoning"], "thinking");
        let step = &meta["tool_steps"][0];
        assert_eq!(step["name"], "bash");
        assert_eq!(step["args"].as_str().unwrap().chars().count(), MAX_STEP_ARGS_CHARS);
        assert_eq!(step["result"].as_str().unwrap().chars().count(), MAX_STEP_RESULT_CHARS);
        assert!(step["duration_ms"].is_u64());
        assert!(meta.get("stopped").is_none());
    }

    #[test]
    fn only_the_first_taker_saves() {
        let mut r = TurnRecord::default();
        r.observe(&SessionEvent::TextDelta("partial".into()));
        let stopped = r.take(None, true).unwrap();
        assert_eq!(stopped.content, "partial");
        assert_eq!(stopped.meta.unwrap()["stopped"], true);
        assert!(r.take(Some("full answer"), false).is_none());
    }

    #[test]
    fn the_run_text_wins_over_what_streamed_and_empty_turns_save_nothing() {
        let mut r = TurnRecord::default();
        r.observe(&SessionEvent::TextDelta("stream".into()));
        let saved = r.take(Some("final"), false).unwrap();
        assert_eq!(saved.content, "final");
        assert!(saved.meta.is_none());

        assert!(TurnRecord::default().take(Some(""), false).is_none());
        assert!(TurnRecord::default().take(None, true).is_none(), "a stop before any output saves nothing");
    }

    fn credits(turn: u32, delta: Option<f64>) -> SessionEvent {
        SessionEvent::StepCredits {
            turn,
            credits_used_before: None,
            credits_used_after: None,
            delta,
            input_tokens: 10,
            output_tokens: 5,
            model: "m".into(),
            approximate: true,
        }
    }

    #[test]
    fn step_credits_are_saved_against_the_first_tool_call_of_their_step() {
        let mut r = TurnRecord::default();
        r.observe(&start("a", json!({})));
        r.observe(&start("b", json!({})));
        r.observe(&credits(1, Some(3.0)));
        r.observe(&start("c", json!({})));
        r.observe(&credits(2, None));
        r.observe(&credits(3, Some(1.5)));
        let meta = r.take(Some("done"), false).unwrap().meta.unwrap();
        let saved = meta["step_credits"].as_array().unwrap();
        assert_eq!(saved.len(), 3);
        assert_eq!((saved[0]["tool_call_id"].as_str(), saved[0]["delta"].as_f64()), (Some("a"), Some(3.0)));
        assert_eq!((saved[1]["tool_call_id"].as_str(), saved[1]["delta"].is_null()), (Some("c"), true));
        assert!(saved[2].get("tool_call_id").is_none(), "the answering step used no tool");
    }

    #[test]
    fn a_runaway_turn_stores_a_bounded_number_of_steps() {
        let mut r = TurnRecord::default();
        for i in 0..MAX_STEPS + 20 {
            r.observe(&start(&format!("c{i}"), json!({})));
        }
        let meta = r.take(None, false).unwrap().meta.unwrap();
        assert_eq!(meta["tool_steps"].as_array().unwrap().len(), MAX_STEPS);
    }
}
