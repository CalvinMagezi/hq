//! CLI-harness-backed [`SessionBackend`] adapters.
//!
//! CLI harnesses are agentic subprocesses (they run their own tools internally
//! and return a final answer). From this contract's perspective they are
//! *buffered* and *limited*: they cannot stream tokens, cannot drive our
//! tool-calling loop, and report no token usage. [`CopilotCliBackend`] advertises
//! exactly that via [`BackendCapabilities::buffered_cli`] — no pretending.
//!
//! The event shape a buffered CLI produces is **lifecycle + one final message**:
//! a [`BackendEvent::Progress`] note, then a single [`BackendEvent::Message`]
//! carrying the full answer, then [`BackendEvent::Done`]. It never fabricates
//! token deltas. Because `Progress` is not committed output, the provider chain
//! can still fail over if the subprocess fails before producing a `Message`.
//!
//! Discovery and process execution reuse the existing primitives
//! ([`hq_core::machine::which_binary`] and
//! [`hq_tools::run_external_cli_harness_strict`]) so nothing here couples to web state.

use std::path::PathBuf;

use async_trait::async_trait;
use hq_core::config::GitHubCopilotConfig;
use hq_core::types::{ChatMessage, MessageRole};

use super::{
    BackendCapabilities, BackendError, BackendEvent, BackendEventStream, BackendRequest,
    SessionBackend,
};

/// A [`SessionBackend`] that dispatches to the GitHub Copilot CLI (`gh copilot`),
/// buffered.
pub struct CopilotCliBackend {
    label: String,
    /// Resolved `gh` binary. `None` means unavailable (chain will fail over).
    binary: Option<PathBuf>,
    model: Option<String>,
    cwd: Option<PathBuf>,
    timeout_secs: u64,
    allow_all_tools: bool,
}

impl CopilotCliBackend {
    /// Build from [`GitHubCopilotConfig`], resolving the `gh` binary via harness
    /// discovery. Returns a backend even when `gh` is absent — [`start`](Self::start)
    /// then reports [`BackendError::Unavailable`] so the chain can fail over.
    pub fn from_config(config: &GitHubCopilotConfig) -> Self {
        let binary = discover_github_copilot_binary();
        Self {
            label: "github-copilot".to_string(),
            binary,
            model: config.model.clone(),
            cwd: config.cwd.clone().map(PathBuf::from),
            timeout_secs: config.timeout_secs,
            allow_all_tools: config.allow_all_tools,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_binary(binary: PathBuf) -> Self {
        Self {
            label: "github-copilot".to_string(),
            binary: Some(binary),
            model: None,
            cwd: None,
            timeout_secs: 300,
            allow_all_tools: false,
        }
    }

    /// Override the label used in diagnostics.
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// The `gh copilot` argument vector for the configured options.
    ///
    /// Mirrors the proven headless invocation
    /// (`copilot -- -s --no-ask-user [--allow-all-tools] [--model M] -p`).
    fn command_args(&self) -> Vec<String> {
        copilot_cli_args(self.allow_all_tools, self.model.as_deref())
    }
}

/// Resolve the GitHub Copilot (`gh`) binary on PATH.
fn discover_github_copilot_binary() -> Option<PathBuf> {
    hq_core::machine::which_binary("gh")
}

/// Build the `gh copilot` headless argument vector.
///
/// The prompt itself is appended by the runner as the final argument (after `-p`).
pub(crate) fn copilot_cli_args(allow_all_tools: bool, model: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "copilot".to_string(),
        "--".to_string(),
        "-s".to_string(),
        "--no-ask-user".to_string(),
    ];
    if allow_all_tools {
        args.push("--allow-all-tools".to_string());
    }
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        args.push("--model".to_string());
        args.push(model.to_string());
    }
    args.push("-p".to_string());
    args
}

/// Flatten a message list into a single prompt string for a one-shot CLI.
///
/// A lone user turn passes through unchanged; multi-turn conversations render
/// as a labeled transcript (system text included as context).
pub(crate) fn render_prompt(messages: &[ChatMessage]) -> String {
    if let [only] = messages
        && only.role == MessageRole::User
    {
        return only.content.clone();
    }

    let mut out = String::new();
    for m in messages {
        if m.content.trim().is_empty() && m.tool_calls.is_empty() {
            continue;
        }
        let label = match m.role {
            MessageRole::System => "System",
            MessageRole::User => "User",
            MessageRole::Assistant => "Assistant",
            MessageRole::Tool => "Tool",
        };
        out.push_str(label);
        out.push_str(": ");
        out.push_str(&m.content);
        out.push_str("\n\n");
    }
    out.trim_end().to_string()
}

/// Inputs threaded through the buffered-stream state machine.
struct RunInputs {
    binary: PathBuf,
    args: Vec<String>,
    prompt: String,
    cwd: Option<PathBuf>,
    timeout_secs: u64,
}

/// State machine for the buffered CLI event stream.
enum BufferedState {
    /// Emit the lifecycle note, then run.
    Progress(String, RunInputs),
    /// Run the subprocess, emit the final message (or an error).
    Run(RunInputs),
    /// Emit the terminal marker.
    Done,
    /// Stream exhausted.
    End,
}

/// Build the normalized buffered event stream for a CLI harness run.
///
/// Yields `Progress` → (`Message` | `Err`) → `Done`. The subprocess runs lazily
/// on the second poll, so the `Progress` note is delivered *before* the work —
/// which lets [`ProviderChain`](super::ProviderChain) fail over cleanly if the
/// run fails before any message is produced.
pub(crate) fn buffered_cli_stream(
    note: String,
    binary: PathBuf,
    args: Vec<String>,
    prompt: String,
    cwd: Option<PathBuf>,
    timeout_secs: u64,
) -> BackendEventStream {
    let run = RunInputs {
        binary,
        args,
        prompt,
        cwd,
        timeout_secs,
    };
    let stream = futures::stream::unfold(BufferedState::Progress(note, run), |state| async move {
        match state {
            BufferedState::Progress(note, run) => {
                Some((Ok(BackendEvent::Progress(note)), BufferedState::Run(run)))
            }
            BufferedState::Run(run) => {
                let RunInputs {
                    binary,
                    args,
                    prompt,
                    cwd,
                    timeout_secs,
                } = run;
                let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let result = hq_tools::run_external_cli_harness_strict(
                    &binary,
                    &arg_refs,
                    &prompt,
                    cwd.as_deref(),
                    timeout_secs,
                )
                .await;
                let item = match result {
                    Ok(text) => Ok(BackendEvent::Message(text)),
                    Err(e) => Err(BackendError::from_anyhow(e)),
                };
                let next = if item.is_ok() {
                    BufferedState::Done
                } else {
                    BufferedState::End
                };
                Some((item, next))
            }
            BufferedState::Done => Some((Ok(BackendEvent::Done), BufferedState::End)),
            BufferedState::End => None,
        }
    });
    Box::pin(stream)
}

#[async_trait]
impl SessionBackend for CopilotCliBackend {
    fn name(&self) -> &str {
        &self.label
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::buffered_cli()
    }

    async fn start(&self, request: &BackendRequest) -> Result<BackendEventStream, BackendError> {
        let Some(binary) = self.binary.clone() else {
            return Err(BackendError::Unavailable(
                "gh (github-copilot) not found in PATH".to_string(),
            ));
        };

        let prompt = render_prompt(&request.messages);
        let note = format!("dispatching to {} (buffered CLI)", self.label);
        Ok(buffered_cli_stream(
            note,
            binary,
            self.command_args(),
            prompt,
            self.cwd.clone(),
            self.timeout_secs,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_stream::StreamExt as _;

    fn msg(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            image_parts: Vec::new(),
            role,
            content: content.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
        }
    }

    #[test]
    fn capabilities_are_honestly_limited() {
        let caps = CopilotCliBackend::with_binary(PathBuf::from("/bin/echo")).capabilities();
        assert!(!caps.streaming);
        assert!(!caps.tools);
        assert!(!caps.reasoning);
        assert!(!caps.usage_accounting);
        assert!(caps.system_prompt);
    }

    #[test]
    fn command_args_gate_allow_all_and_model() {
        assert_eq!(
            copilot_cli_args(false, None),
            vec!["copilot", "--", "-s", "--no-ask-user", "-p"]
        );
        assert_eq!(
            copilot_cli_args(true, Some("claude-sonnet-4.6")),
            vec![
                "copilot",
                "--",
                "-s",
                "--no-ask-user",
                "--allow-all-tools",
                "--model",
                "claude-sonnet-4.6",
                "-p"
            ]
        );
        // Blank model is ignored.
        assert_eq!(
            copilot_cli_args(false, Some("  ")),
            vec!["copilot", "--", "-s", "--no-ask-user", "-p"]
        );
    }

    #[test]
    fn render_prompt_passes_lone_user_turn_through() {
        assert_eq!(
            render_prompt(&[msg(MessageRole::User, "just do it")]),
            "just do it"
        );
    }

    #[test]
    fn render_prompt_builds_labeled_transcript() {
        let rendered = render_prompt(&[
            msg(MessageRole::System, "be terse"),
            msg(MessageRole::User, "hello"),
            msg(MessageRole::Assistant, "hi"),
        ]);
        assert!(rendered.contains("System: be terse"));
        assert!(rendered.contains("User: hello"));
        assert!(rendered.contains("Assistant: hi"));
    }

    #[tokio::test]
    async fn unavailable_binary_yields_failoverable_error() {
        let backend = CopilotCliBackend {
            label: "github-copilot".to_string(),
            binary: None,
            model: None,
            cwd: None,
            timeout_secs: 30,
            allow_all_tools: false,
        };
        let err = match backend
            .start(&BackendRequest {
                messages: vec![msg(MessageRole::User, "hi")],
                ..Default::default()
            })
            .await
        {
            Ok(_) => panic!("expected an unavailable error"),
            Err(e) => e,
        };
        assert!(matches!(err, BackendError::Unavailable(_)));
        assert!(err.is_failoverable());
    }

    #[tokio::test]
    async fn buffered_stream_emits_progress_message_done() {
        // Deterministic subprocess: `/bin/echo COPILOT_OK` prints the prompt back.
        let stream = buffered_cli_stream(
            "dispatching".to_string(),
            PathBuf::from("/bin/echo"),
            Vec::new(),
            "COPILOT_OK".to_string(),
            None,
            30,
        );
        let events: Vec<_> = stream.collect().await;
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                Ok(BackendEvent::Progress(_)) => "progress",
                Ok(BackendEvent::Message(_)) => "message",
                Ok(BackendEvent::Done) => "done",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["progress", "message", "done"]);
        let message = events.iter().find_map(|e| match e {
            Ok(BackendEvent::Message(m)) => Some(m.clone()),
            _ => None,
        });
        assert_eq!(message.as_deref(), Some("COPILOT_OK"));
        // Exactly one output event (the final Message) — no fake token deltas.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Ok(ev) if ev.is_output()))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn nonzero_copilot_exit_is_classified_for_failover() {
        let stream = buffered_cli_stream(
            "dispatching".to_string(),
            PathBuf::from("/bin/sh"),
            vec![
                "-c".to_string(),
                "echo 'rate limit exceeded' >&2; exit 1".to_string(),
            ],
            "ignored".to_string(),
            None,
            30,
        );
        let events: Vec<_> = stream.collect().await;

        assert!(matches!(
            events.first(),
            Some(Ok(BackendEvent::Progress(_)))
        ));
        assert!(matches!(
            events.get(1),
            Some(Err(BackendError::Transient(message))) if message.contains("rate limit")
        ));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Ok(BackendEvent::Message(_))))
        );
    }
}
