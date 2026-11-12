//! Follows one session's turns from the messages an agent sends. It is the structured counterpart
//! of reading a terminal screen: the state, whether a turn is done, the dialog it waits at, the
//! files it edited and how it ended all come from protocol facts, never from guessing at text.

use crate::dialog::{Choice, Dialog, outcome, parse_dialog};
use crate::update::{ToolKind, Update, parse_update};
use crate::wire::{Incoming, METHOD_NOT_FOUND, RpcError, notification, reply_error, reply_ok};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const PERMISSION_METHOD: &str = "session/request_permission";
const UPDATE_METHOD: &str = "session/update";
const AUTH_CODE: i64 = -32000;
/// Lines of transcript kept; older ones are dropped and counted.
const TRANSCRIPT_MAX_LINES: usize = 5000;
const BILLING_WORDS: [&str; 5] = ["subscription", "quota", "billing", "rate limit", "credit"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    Cancelled,
    MaxTokens,
    Refusal,
    Other(String),
}

/// Why a prompt failed, coarse enough to choose between asking for a login, trying another lane
/// and reporting a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Auth,
    Billing,
    Other,
}

pub fn failure_kind(e: &RpcError) -> FailureKind {
    let msg = e.message.to_lowercase();
    if e.code == AUTH_CODE || msg.contains("authentication") {
        FailureKind::Auth
    } else if BILLING_WORDS.iter().any(|w| msg.contains(w)) {
        FailureKind::Billing
    } else {
        FailureKind::Other
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub total: Option<u64>,
    pub cached_read: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnEnd {
    Stopped {
        reason: StopReason,
        usage: TokenUsage,
    },
    Failed {
        kind: FailureKind,
        error: RpcError,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    /// The number an answer must name; a new dialog always gets a new one.
    pub seq: u64,
    pub request_id: Value,
    pub dialog: Dialog,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnswerError {
    #[error("no dialog is open")]
    NoDialog,
    #[error("the dialog changed; read it again")]
    Stale,
    #[error("the dialog has no option '{0}'")]
    UnknownOption(String),
}

#[derive(Debug, Default)]
struct Edit {
    paths: Vec<String>,
    failed: bool,
}

#[derive(Debug)]
pub struct Tracker {
    state: AgentState,
    state_seq: u64,
    prompt_id: Option<u64>,
    pending: Option<Pending>,
    tools: BTreeMap<String, ToolKind>,
    edits: BTreeMap<String, Edit>,
    text: String,
    context: (Option<u64>, Option<u64>),
    end: Option<TurnEnd>,
    cwd: Option<String>,
    lines: VecDeque<String>,
    dropped: u64,
    partial: String,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            state: AgentState::Idle,
            state_seq: 0,
            prompt_id: None,
            pending: None,
            tools: BTreeMap::new(),
            edits: BTreeMap::new(),
            text: String::new(),
            context: (None, None),
            end: None,
            cwd: None,
            lines: VecDeque::new(),
            dropped: 0,
            partial: String::new(),
        }
    }
}

impl Tracker {
    pub fn state(&self) -> AgentState {
        self.state
    }

    /// Goes up whenever the state or the open dialog changes, like the host's `state_seq`.
    pub fn state_seq(&self) -> u64 {
        self.state_seq
    }

    /// A turn finished and nothing has been asked of the agent since.
    pub fn done(&self) -> bool {
        self.state == AgentState::Idle && self.end.is_some()
    }

    pub fn pending(&self) -> Option<&Pending> {
        self.pending.as_ref()
    }

    pub fn end(&self) -> Option<&TurnEnd> {
        self.end.as_ref()
    }

    /// What the agent wrote in the current or last turn.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Context window use as `(used, size)`, when the agent reports it.
    pub fn context_use(&self) -> (Option<u64>, Option<u64>) {
        (self.context.1, self.context.0)
    }

    /// The session as lines, oldest first: what the agent said, the tools it started, the dialogs
    /// it opened and how each turn ended. Older lines are dropped past a cap; `dropped` counts them.
    pub fn lines(&self) -> &VecDeque<String> {
        &self.lines
    }

    /// The unfinished last line of the agent's text.
    pub fn partial(&self) -> &str {
        &self.partial
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Records a prompt that was sent, so the transcript reads as a conversation.
    pub fn record_prompt(&mut self, text: &str) {
        let first = text.lines().next().unwrap_or("");
        self.note(format!("> {first}"));
    }

    /// The folder the session runs in; relative paths an agent reports (Hermes sends
    /// `hello.txt`) are read against it.
    pub fn set_cwd(&mut self, cwd: &str) {
        self.cwd = Some(cwd.trim_end_matches('/').to_owned());
    }

    /// Files the current or last turn edited, leaving out edits the agent reported as failed. This
    /// is the agent's own account: evidence for a task should be checked against git.
    pub fn touched(&self) -> BTreeSet<String> {
        let ok = self.edits.values().filter(|e| !e.failed);
        ok.flat_map(|e| e.paths.iter().map(|p| self.absolute(p)))
            .collect()
    }

    fn absolute(&self, path: &str) -> String {
        match &self.cwd {
            Some(cwd) if !path.starts_with('/') => {
                format!("{cwd}/{}", path.trim_start_matches("./"))
            }
            _ => path.to_owned(),
        }
    }

    pub fn start_prompt(&mut self, request_id: u64) {
        self.prompt_id = Some(request_id);
        self.pending = None;
        self.edits.clear();
        self.text.clear();
        self.end = None;
        self.set_state(AgentState::Working);
    }

    /// Takes one message from the agent. Returns the lines to write back to it, if any.
    pub fn feed(&mut self, msg: &Incoming) -> Vec<String> {
        match msg {
            Incoming::Notification { method, params } if method == UPDATE_METHOD => {
                if let Some(update) = parse_update(params) {
                    self.apply(update);
                }
                Vec::new()
            }
            Incoming::Request { id, method, params } if method == PERMISSION_METHOD => {
                self.open_dialog(id, params)
            }
            Incoming::Request { id, method, .. } => {
                let why = format!("{method} is not supported by this client");
                vec![reply_error(id, METHOD_NOT_FOUND, &why)]
            }
            Incoming::Response { id, result } if Some(*id) == self.prompt_id => {
                self.finish(result);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Answers the open dialog `seq`. Returns the line to write to the agent.
    pub fn answer(&mut self, seq: u64, choice: &Choice) -> Result<String, AnswerError> {
        let pending = self.pending.as_ref().ok_or(AnswerError::NoDialog)?;
        if pending.seq != seq {
            return Err(AnswerError::Stale);
        }
        if let Choice::Option(id) = choice
            && !pending.dialog.options.iter().any(|o| &o.id == id)
        {
            return Err(AnswerError::UnknownOption(id.clone()));
        }
        let line = reply_ok(&pending.request_id, outcome(choice));
        self.pending = None;
        self.set_state(AgentState::Working);
        Ok(line)
    }

    /// Lines that stop the turn: the open dialog is answered "cancelled" first, as the protocol
    /// asks, then the cancel notification goes out. The turn ends when the agent's reply arrives.
    pub fn cancel(&mut self, session_id: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(p) = self.pending.take() {
            lines.push(reply_ok(&p.request_id, outcome(&Choice::Cancelled)));
            self.set_state(AgentState::Working);
        }
        lines.push(notification(
            "session/cancel",
            json!({"sessionId": session_id}),
        ));
        lines
    }

    fn apply(&mut self, update: Update) {
        match update {
            Update::AgentText(t) => {
                self.say(&t);
                self.text.push_str(&t);
            }
            Update::ToolStart(call) => {
                if !call.title.is_empty() {
                    self.note(format!("tool: {}", call.title));
                }
                self.tools.insert(call.id.clone(), call.kind);
                if call.kind == ToolKind::Edit {
                    self.edits.entry(call.id).or_default().paths = call.paths;
                }
            }
            Update::ToolProgress { id, status, paths } => self.progress(id, status, paths),
            Update::Usage { size, used } => self.context = (size.or(self.context.0), used),
            Update::Thought(_) | Update::Other(_) => {}
        }
    }

    fn progress(&mut self, id: String, status: Option<String>, paths: Vec<String>) {
        if self.tools.get(&id) != Some(&ToolKind::Edit) {
            return;
        }
        let edit = self.edits.entry(id).or_default();
        for p in paths {
            if !edit.paths.contains(&p) {
                edit.paths.push(p);
            }
        }
        edit.failed = status.as_deref() == Some("failed");
    }

    fn open_dialog(&mut self, id: &Value, params: &Value) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(old) = self.pending.take() {
            lines.push(reply_ok(&old.request_id, outcome(&Choice::Cancelled)));
        }
        let title = params.pointer("/toolCall/title").and_then(Value::as_str);
        self.note(format!("ask: {}", title.unwrap_or("permission")));
        let seq = self.bump();
        self.pending = Some(Pending {
            seq,
            request_id: id.clone(),
            dialog: parse_dialog(params),
        });
        self.state = AgentState::Blocked;
        lines
    }

    fn finish(&mut self, result: &Result<Value, RpcError>) {
        self.prompt_id = None;
        self.pending = None;
        self.note(match result {
            Ok(v) => format!("end: {}", stop_label(&stop_reason(v))),
            Err(e) => format!("failed: {}", e.message),
        });
        self.end = Some(match result {
            Ok(v) => TurnEnd::Stopped {
                reason: stop_reason(v),
                usage: token_usage(v),
            },
            Err(e) => TurnEnd::Failed {
                kind: failure_kind(e),
                error: e.clone(),
            },
        });
        self.set_state(AgentState::Idle);
    }

    /// Adds agent text, finishing a line at each newline.
    fn say(&mut self, text: &str) {
        let mut pieces = text.split('\n');
        if let Some(first) = pieces.next() {
            self.partial.push_str(first);
        }
        for piece in pieces {
            let done = std::mem::take(&mut self.partial);
            self.push_line(done);
            self.partial.push_str(piece);
        }
    }

    /// A marker line; whatever the agent was in the middle of saying is finished first.
    fn note(&mut self, line: String) {
        if !self.partial.is_empty() {
            let done = std::mem::take(&mut self.partial);
            self.push_line(done);
        }
        self.push_line(line);
    }

    fn push_line(&mut self, line: String) {
        if self.lines.len() == TRANSCRIPT_MAX_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    fn set_state(&mut self, state: AgentState) {
        self.state = state;
        self.bump();
    }

    fn bump(&mut self) -> u64 {
        self.state_seq += 1;
        self.state_seq
    }
}

fn stop_label(reason: &StopReason) -> String {
    match reason {
        StopReason::EndTurn => "turn done".into(),
        StopReason::Cancelled => "cancelled".into(),
        StopReason::MaxTokens => "out of tokens".into(),
        StopReason::Refusal => "refused".into(),
        StopReason::Other(s) => s.clone(),
    }
}

fn stop_reason(result: &Value) -> StopReason {
    match result.get("stopReason").and_then(Value::as_str) {
        Some("end_turn") => StopReason::EndTurn,
        Some("cancelled") => StopReason::Cancelled,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("refusal") => StopReason::Refusal,
        Some(other) => StopReason::Other(other.to_owned()),
        None => StopReason::Other(String::new()),
    }
}

fn token_usage(result: &Value) -> TokenUsage {
    let n = |key: &str| {
        result
            .pointer(&format!("/usage/{key}"))
            .and_then(Value::as_u64)
    };
    TokenUsage {
        input: n("inputTokens"),
        output: n("outputTokens"),
        total: n("totalTokens"),
        cached_read: n("cachedReadTokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::parse_line;

    fn feed(t: &mut Tracker, line: &str) -> Vec<String> {
        t.feed(&parse_line(line).expect("test message parses"))
    }

    #[test]
    fn a_turn_goes_working_then_idle_and_done() {
        let mut t = Tracker::default();
        assert!(!t.done());
        t.start_prompt(3);
        assert_eq!(t.state(), AgentState::Working);
        feed(
            &mut t,
            r#"{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}}}"#,
        );
        feed(
            &mut t,
            r#"{"id":3,"result":{"stopReason":"end_turn","usage":{"totalTokens":9}}}"#,
        );
        assert!(t.done());
        assert_eq!(t.text(), "hi");
        assert!(
            matches!(t.end(), Some(TurnEnd::Stopped { reason: StopReason::EndTurn, usage }) if usage.total == Some(9))
        );
    }

    #[test]
    fn a_dialog_blocks_and_a_stale_answer_is_refused() {
        let mut t = Tracker::default();
        t.start_prompt(3);
        feed(
            &mut t,
            r#"{"id":"p1","method":"session/request_permission","params":{"options":[{"optionId":"yes","kind":"allow_once","name":"Yes"}],"toolCall":{"title":"x","kind":"edit"}}}"#,
        );
        assert_eq!(t.state(), AgentState::Blocked);
        let seq = t.pending().expect("dialog").seq;
        assert_eq!(
            t.answer(seq + 1, &Choice::Option("yes".into())),
            Err(AnswerError::Stale)
        );
        assert_eq!(
            t.answer(seq, &Choice::Option("nope".into())),
            Err(AnswerError::UnknownOption("nope".into()))
        );
        let line = t
            .answer(seq, &Choice::Option("yes".into()))
            .expect("answer");
        assert!(line.contains(r#""optionId":"yes""#) && line.contains(r#""id":"p1""#));
        assert_eq!(t.state(), AgentState::Working);
        assert_eq!(
            t.answer(seq, &Choice::Cancelled),
            Err(AnswerError::NoDialog)
        );
    }

    #[test]
    fn a_failed_edit_is_not_counted_as_touched() {
        let mut t = Tracker::default();
        t.start_prompt(1);
        let start = |id: &str, path: &str| {
            format!(
                r#"{{"method":"session/update","params":{{"update":{{"sessionUpdate":"tool_call","toolCallId":"{id}","kind":"edit","locations":[{{"path":"{path}"}}]}}}}}}"#
            )
        };
        feed(&mut t, &start("a", "/w/ok"));
        feed(&mut t, &start("b", "/w/bad"));
        feed(
            &mut t,
            r#"{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"b","status":"failed"}}}"#,
        );
        assert_eq!(t.touched().into_iter().collect::<Vec<_>>(), vec!["/w/ok"]);
    }

    #[test]
    fn the_transcript_reads_as_lines_with_markers() {
        let mut t = Tracker::default();
        t.record_prompt("write it\nplease");
        t.start_prompt(1);
        let chunk = |text: &str| {
            format!(
                r#"{{"method":"session/update","params":{{"update":{{"sessionUpdate":"agent_message_chunk","content":{{"text":"{text}"}}}}}}}}"#
            )
        };
        feed(&mut t, &chunk("one\\ntw"));
        feed(&mut t, &chunk("o"));
        assert_eq!(t.partial(), "two");
        feed(&mut t, r#"{"id":1,"result":{"stopReason":"end_turn"}}"#);
        let lines: Vec<&str> = t.lines().iter().map(String::as_str).collect();
        assert_eq!(lines, vec!["> write it", "one", "two", "end: turn done"]);
        assert_eq!(t.partial(), "");
    }

    #[test]
    fn relative_paths_are_read_against_the_session_folder() {
        let mut t = Tracker::default();
        t.set_cwd("/work/");
        t.start_prompt(1);
        feed(
            &mut t,
            r#"{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":"a","kind":"edit","locations":[{"path":"./src/a.rs"}]}}}"#,
        );
        assert_eq!(
            t.touched().into_iter().collect::<Vec<_>>(),
            vec!["/work/src/a.rs"]
        );
    }

    #[test]
    fn cancel_answers_the_dialog_before_notifying() {
        let mut t = Tracker::default();
        t.start_prompt(1);
        feed(
            &mut t,
            r#"{"id":7,"method":"session/request_permission","params":{"options":[]}}"#,
        );
        let lines = t.cancel("s1");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("cancelled") && lines[0].contains(r#""id":7"#));
        assert!(lines[1].contains("session/cancel"));
    }

    #[test]
    fn unknown_agent_requests_get_method_not_found() {
        let mut t = Tracker::default();
        let out = feed(
            &mut t,
            r#"{"id":4,"method":"fs/read_text_file","params":{}}"#,
        );
        assert!(out[0].contains("-32601"));
    }

    #[test]
    fn failures_are_classified() {
        let e = |code, m: &str| RpcError {
            code,
            message: m.into(),
        };
        assert_eq!(
            failure_kind(&e(-32000, "Authentication required")),
            FailureKind::Auth
        );
        assert_eq!(
            failure_kind(&e(-32603, "Your current subscription does not have access")),
            FailureKind::Billing
        );
        assert_eq!(failure_kind(&e(-32603, "boom")), FailureKind::Other);
    }
}
