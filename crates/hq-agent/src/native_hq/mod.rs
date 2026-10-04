//! Shared native-hq dispatch: the one place that builds and runs an `AgentSession`
//! for the hq harness.
//!
//! The Telegram and Discord relays both route through `run_native_hq`, so the
//! harness configuration — history injection, timeout, and the success/quality
//! signal — can never drift between the two surfaces. Surface-specific concerns
//! (which instructions, which model, live event streaming, cancel wiring) are
//! passed in via `NativeHqHooks`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::Result;
use hq_core::config::HqConfig;
use hq_core::types::{ChatMessage, MessageRole, SessionEvent};

use crate::builder::SessionBuilder;
use crate::session::SessionConfig;

mod ingest;
mod progress;

pub use ingest::wire_post_turn_ingestion;
use ingest::{
    estimate_tokens, record_thread_entry, record_thread_turn, result_text, thread_identity,
};
use progress::supervise_detached;
pub use progress::{
    DetachedTurnOutcome, DetachedTurnSink, ProgressEvent, ProgressSink, parked_ack, parked_marker,
    render_progress_note,
};

/// Quality fed to the router on a clean completion. Soft limits (max turns / budget)
/// and hard failures score lower so the self-learning signal reflects how it ended.
const QUALITY_COMPLETE: f64 = 0.7;
const QUALITY_SOFT_LIMIT: f64 = 0.4;
const QUALITY_FAILED: f64 = 0.2;

/// Shared slot a surface writes a mid-turn redirect message into.
pub type SteerInbox = Arc<std::sync::Mutex<Option<String>>>;

/// Surface-specific wiring for a native-hq run. All optional — the proxy passes
/// `Default`, the relay supplies history, an event sink, a cancel hook, and a timeout.
#[derive(Default)]
pub struct NativeHqHooks {
    /// Prior conversation turns to inject before the prompt (relay). System messages skipped.
    pub history: Vec<ChatMessage>,
    /// Live event subscriber (relay ActivityFeed). Registered before the prompt runs.
    pub on_event: Option<Box<dyn Fn(SessionEvent) + Send + Sync + 'static>>,
    /// Called once with the session's cancel flag after build, so a surface can wire
    /// its own "cancel this run" control (relay `!cancel`).
    pub on_cancel: Option<Box<dyn FnOnce(Arc<AtomicBool>) + Send>>,
    /// Called once with the session's steer inbox after build, so a surface can
    /// wire "redirect this run" — a message that arrives mid-turn (relay).
    pub on_steer: Option<Box<dyn FnOnce(SteerInbox) + Send>>,
    /// Wall-clock timeout for the prompt. None = no extra timeout.
    pub timeout: Option<Duration>,
    /// Resolved request identity. When set, the session merges cross-interface
    /// thread context and the run appends its turns to the interface's
    /// `_threads/*.jsonl` file.
    pub identity: Option<hq_core::identity::RequestIdentity>,
    /// When set (with `timeout`), the timeout becomes an ack window instead of a
    /// kill switch: the prompt keeps running in a background task and the final
    /// result is delivered here. When `None`, the legacy timeout-drop behavior
    /// is preserved exactly.
    pub on_detached: Option<DetachedTurnSink>,
    /// `background_turns` row id registered by the caller, echoed back in the
    /// [`DetachedTurnOutcome`] so the sink can mark the row completed.
    pub turn_id: Option<String>,
    /// Async child-completion delivery for non-blocking `spawn_subagents`
    /// plans. Wired into the session's `ChildExecContext` (with `turn_id` as
    /// the parent turn) so each finished child is pushed back to the surface
    /// and recorded in the chat thread.
    pub on_child_completion: Option<crate::agents::CompletionSink>,
    /// Progress callback for a detached turn: heartbeat ticks from the
    /// detached supervisor plus volunteered notes from `report_progress`.
    /// Only ticks after detach (heartbeats are pointless before the ack
    /// message); the synchronous path never fires it.
    pub on_progress: Option<ProgressSink>,
    /// Heartbeat cadence for the detached supervisor, in seconds. Requires
    /// `on_progress`; `None` (or 0) disables heartbeats.
    pub progress_interval_secs: Option<u64>,
    /// Named permission preset to apply instead of the process default,
    /// pinned per-chat via `/permission` (Telegram) / `!permission`
    /// (Discord). `None` leaves `SessionBuilder`'s own default untouched.
    pub permission_preset: Option<hq_core::types::PermissionPreset>,
    /// Images attached to the CURRENT turn (FR-017), forwarded to a
    /// vision-capable model. Callers must NOT also include the current
    /// turn's own message in `history` — `history` is prior turns only;
    /// this prompt call pushes the current one. Including both would
    /// duplicate the turn as two consecutive User messages, with only the
    /// `history` copy carrying the image (the actual triggering message,
    /// pushed here, would silently lose it).
    pub image_parts: Vec<hq_core::types::ImageAttachment>,
    /// A self-contained conversation (a web chat): no turns from other
    /// interfaces in the prompt, and none of its own written to `_threads/`.
    /// `history` is its only context.
    pub isolated: bool,
    /// Do not record this exchange in long-term memory after the turn. For a
    /// turn whose asker is not the owner typing (an MCP `hq_ask`).
    pub skip_memory_ingestion: bool,
    /// Name prefixes of tools this turn must not see at all.
    pub deny_tool_prefixes: Vec<String>,
}

/// Outcome of a native-hq run, carrying the explicit success + quality signal so
/// both surfaces feed the self-learning loop identically.
pub struct NativeHqResult {
    pub text: String,
    pub success: bool,
    pub quality: f64,
    pub latency_ms: u64,
    pub output_tokens: u32,
    /// True when the turn detached past its ack window: `text` is the parking
    /// ack and the real result arrives later via the `on_detached` sink.
    pub detached: bool,
}

/// Cheap local guard against a `TaskType::Vault` misclassification of an actual
/// coding task (e.g. "Add a note explaining the recursion in fibonacci.rs" —
/// `task_classifier::classify_task` matches the bare word "note" before it ever
/// checks code signals). Mirrors the kind of signal `task_classifier.rs`'s own
/// `CodeEditing` branch checks, without depending on that module's internals —
/// deliberately not calling into `task_classifier` so this stays a narrow,
/// local check scoped to the Weak-profile decision below, not a change to the
/// shared classifier.
fn prompt_has_code_signal(prompt: &str) -> bool {
    let p = prompt.to_lowercase();
    if p.contains(".rs")
        || p.contains(".ts")
        || p.contains(".py")
        || p.contains(".js")
        || p.contains("```")
        || p.contains("fn ")
        || p.contains("impl ")
        || p.contains("def ")
        || p.contains("function ")
    {
        return true;
    }
    // Whole-word fault vocabulary: "bug"/"error"/"crash"/"exception" are specific
    // to software-fault contexts in a way generic verbs ("fix", "edit", "broken")
    // are not — those verbs are common in ordinary, genuinely vault-only prompts
    // ("remember to fix the note about mom's birthday"), so they're deliberately
    // excluded here to avoid re-inflating the token cost this guard exists to cut.
    // Word-set membership (not substring) mirrors `task_classifier::has_any` so
    // "debugging" doesn't false-positive on "bug".
    let words: std::collections::HashSet<&str> = p.split_whitespace().collect();
    words.contains("bug")
        || words.contains("error")
        || words.contains("crash")
        || words.contains("exception")
}

/// Apply the standard hq harness configuration to a builder: vault-scoped tool
/// tiering plus the base instructions. Exposed so callers that must build the
/// session themselves still share the gating.
pub fn configure_native_hq_builder(
    builder: SessionBuilder,
    _config: &HqConfig,
    prompt: &str,
    base_instructions: String,
) -> SessionBuilder {
    let mut builder = builder;
    // Vault-centric requests ("remember X", "save this note", "search my vault")
    // never need the ~70-tool coding/dev/calendar/cursor catalog that `SessionBuilder`
    // registers by default — every one of those tool schemas is serialized into the
    // model request on every turn regardless of task type. `SessionProfile::Weak`
    // narrows the session to the lightweight vault/dev shortcut tools (already tagged
    // `ToolPolicy::Weak` for exactly this purpose) instead.
    //
    // `prompt_has_code_signal` guards against a Weak session stripping
    // `bash`/`edit`/`read`/`write` entirely (a hard failure, not a degraded one, if
    // wrong): `classify_task` checks `TaskType::Vault` triggers (bare "note",
    // "notebook", "remember") before `TaskType::CodeEditing` triggers, so a real
    // coding prompt like "Add a note explaining the recursion in fibonacci.rs"
    // misclassifies as Vault. That's a pre-existing, shared-classifier quirk
    // (out of scope to fix here); this is a local, cheap secondary check that
    // skips the downgrade when the prompt still carries an obvious code signal
    // despite the Vault classification.
    if hq_tools::task_classifier::classify_task(prompt)
        == hq_tools::task_classifier::TaskType::Vault
        && !prompt_has_code_signal(prompt)
    {
        builder = builder.session_profile(crate::builder::SessionProfile::Weak);
    }
    builder.harness_instructions(base_instructions)
}

/// Build the session builder for a native-hq turn from the surface's hooks.
/// Takes the child-completion sink out of `hooks`; everything else stays.
fn native_builder(
    config: &HqConfig,
    prompt: &str,
    base_instructions: String,
    cwd: PathBuf,
    session_config: SessionConfig,
    hooks: &mut NativeHqHooks,
) -> SessionBuilder {
    let mut base_builder = SessionBuilder::from_config(config)
        .working_dir(cwd)
        .session_config(session_config);
    if let Some(ref identity) = hooks.identity {
        base_builder = base_builder.with_identity(identity.clone());
        if !hooks.history.is_empty() {
            base_builder = base_builder.exclude_own_interface_thread();
        }
    }
    if hooks.isolated {
        base_builder = base_builder.no_thread_continuity();
    }
    if !hooks.deny_tool_prefixes.is_empty() {
        base_builder = base_builder.deny_tool_prefixes(hooks.deny_tool_prefixes.clone());
    }
    if let Some(preset) = hooks.permission_preset {
        base_builder = base_builder.permission_preset(preset);
    }
    let builder = configure_native_hq_builder(base_builder, config, prompt, base_instructions);

    // Wire async child-completion delivery into the session's sub-agent tools:
    // the surface-supplied sink (chat message + registry attach) plus a thread
    // record so the parent session's history sees child outcomes on follow-ups.
    let builder = if let Some(sink) = hooks.on_child_completion.take() {
        let vault_path = config.vault_path.clone();
        let identity = thread_identity(hooks);
        let wrapped: crate::agents::CompletionSink = std::sync::Arc::new(move |event| {
            if let Some(id) = &identity {
                let status = if event.success { "finished" } else { "failed" };
                let entry = format!(
                    "Sub-agent {} ({}) {status}: {}",
                    event.task_id, event.role, event.summary
                );
                if let Err(e) =
                    crate::threads::append_thread_entry(&vault_path, id, "assistant", &entry)
                {
                    tracing::warn!(%e, "native_hq: child completion thread append failed");
                }
            }
            sink(event);
        });
        builder.child_completion(hooks.turn_id.clone(), wrapped)
    } else {
        builder
    };
    let builder = builder.child_turn(hooks.turn_id.clone());
    // Volunteered `report_progress` notes go to the same sink as heartbeats.
    match &hooks.on_progress {
        Some(sink) => builder.child_progress(hooks.turn_id.clone(), sink.clone()),
        None => builder,
    }
}

/// Build and run a native-hq `AgentSession`. The single shared path for the hq harness.
pub async fn run_native_hq(
    config: &HqConfig,
    prompt: &str,
    base_instructions: String,
    cwd: PathBuf,
    session_config: SessionConfig,
    hooks: NativeHqHooks,
) -> Result<NativeHqResult> {
    let mut hooks = hooks;
    let builder = native_builder(
        config,
        prompt,
        base_instructions,
        cwd,
        session_config,
        &mut hooks,
    );
    let mut session = builder.build().await?;
    let harness_label = hooks
        .identity
        .as_ref()
        .map(|i| i.source.label())
        .unwrap_or("cli");
    if !hooks.skip_memory_ingestion {
        wire_post_turn_ingestion(
            &mut session,
            &config.vault_path,
            &config.db_path(),
            harness_label,
        );
    }
    let thread_identity = thread_identity(&hooks);
    // FR-017: the current turn's images, forwarded separately from
    // `history` — see `NativeHqHooks::image_parts`'s doc comment for why
    // `history` must be prior turns only, not the current one.
    let image_parts = hooks.image_parts;

    for msg in hooks.history {
        if msg.role != MessageRole::System {
            session.push_message(msg);
        }
    }
    if let Some(cb) = hooks.on_cancel {
        cb(session.cancel_handle());
    }
    if let Some(cb) = hooks.on_steer {
        cb(session.steer_handle());
    }
    if let Some(sink) = hooks.on_event {
        session.on_event(sink);
    }

    let start = Instant::now();
    // Route through the unified streaming engine so HTTP/WebSocket/relay
    // subscribers receive real token deltas from streaming API backends (and a
    // buffered CLI backend still surfaces progress + one final text event). The
    // engine collects the run into the same `SessionResult` either way.
    let (on_detached, turn_id, timeout) = (hooks.on_detached, hooks.turn_id, hooks.timeout);
    let (on_progress, progress_interval_secs) = (hooks.on_progress, hooks.progress_interval_secs);
    let outcome = match timeout {
        Some(t) if on_detached.is_some() => {
            // Detach mode: the timeout is an ack window, not a kill switch. The
            // prompt future must own the session so it can be moved into a
            // background task when the window elapses; Box::pin keeps it
            // borrow-free and 'static.
            let sink = on_detached.expect("checked by match guard");
            let prompt_owned = prompt.to_string();
            let image_parts = image_parts.clone();
            let mut fut = Box::pin(async move {
                session
                    .prompt_stream_with_images(&prompt_owned, image_parts)
                    .await
            });
            tokio::select! {
                inner = &mut fut => inner,
                _ = tokio::time::sleep(t) => {
                    // Ack window elapsed: park the turn, spawn the prompt to
                    // completion, and reply with the parking ack immediately.
                    let text = parked_ack(turn_id.as_deref());
                    record_thread_turn(config, &thread_identity, prompt, &text);
                    let bg_config = config.clone();
                    let bg_identity = thread_identity.clone();
                    tokio::spawn(async move {
                        // Heartbeat ticker lives only here, in the detached
                        // supervisor: the pre-detach path never ticks.
                        let outcome = supervise_detached(
                            fut,
                            on_progress,
                            progress_interval_secs,
                            turn_id.clone().unwrap_or_default(),
                            start,
                        )
                        .await;
                        let latency_ms = start.elapsed().as_millis() as u64;
                        // Same success signal as the synchronous path: a clean
                        // `Complete` is success, soft limits and hard failures
                        // are not.
                        let (text, success, output_tokens) = match &outcome {
                            Ok(result) => (
                                result_text(result),
                                result.is_complete(),
                                estimate_tokens(result.text()),
                            ),
                            Err(e) => (format!("(HQ error: {e})"), false, 0),
                        };
                        // The user half was recorded with the parking ack.
                        record_thread_entry(&bg_config, &bg_identity, "assistant", &text);
                        sink(DetachedTurnOutcome {
                            turn_id: turn_id.unwrap_or_default(),
                            text,
                            success,
                            latency_ms,
                            output_tokens,
                        });
                    });
                    return Ok(NativeHqResult {
                        text,
                        success: true,
                        quality: QUALITY_COMPLETE,
                        latency_ms: start.elapsed().as_millis() as u64,
                        output_tokens: 0,
                        detached: true,
                    });
                }
            }
        }
        Some(t) => match tokio::time::timeout(
            t,
            session.prompt_stream_with_images(prompt, image_parts.clone()),
        )
        .await
        {
            Ok(inner) => inner,
            Err(_) => {
                let text = "Request timed out. Try breaking it into smaller steps.".to_string();
                record_thread_turn(config, &thread_identity, prompt, &text);
                return Ok(NativeHqResult {
                    text,
                    success: false,
                    quality: QUALITY_FAILED,
                    latency_ms: start.elapsed().as_millis() as u64,
                    output_tokens: 0,
                    detached: false,
                });
            }
        },
        None => session.prompt_stream_with_images(prompt, image_parts).await,
    };
    let latency_ms = start.elapsed().as_millis() as u64;

    match outcome {
        Ok(result) => {
            let success = result.is_complete();
            let quality = if success {
                QUALITY_COMPLETE
            } else if result.is_failed() {
                // A post-output backend error is a hard failure (truncated,
                // errored output), not a soft limit like max-turns/budget.
                QUALITY_FAILED
            } else {
                QUALITY_SOFT_LIMIT
            };
            let text = result_text(&result);
            record_thread_turn(config, &thread_identity, prompt, &text);
            Ok(NativeHqResult {
                output_tokens: estimate_tokens(&text),
                text,
                success,
                quality,
                latency_ms,
                detached: false,
            })
        }
        Err(e) => {
            let text = format!("(HQ error: {e})");
            record_thread_turn(config, &thread_identity, prompt, &text);
            Ok(NativeHqResult {
                text,
                success: false,
                quality: QUALITY_FAILED,
                latency_ms,
                output_tokens: 0,
                detached: false,
            })
        }
    }
}

#[cfg(test)]
mod tests;
