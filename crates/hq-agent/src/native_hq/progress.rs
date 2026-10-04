//! Progress events, parking acks, and the detached-turn heartbeat supervisor.

use std::time::{Duration, Instant};

/// Completion callback for a turn that detached past its ack window. Invoked
/// once from the background task when the parked `prompt_stream` resolves,
/// carrying the final text and the same success signal the synchronous path
/// would have produced.
pub type DetachedTurnSink = std::sync::Arc<dyn Fn(DetachedTurnOutcome) + Send + Sync>;

/// Final outcome of a detached (backgrounded) turn, delivered via
/// [`DetachedTurnSink`]. Field types mirror [`NativeHqResult`].
pub struct DetachedTurnOutcome {
    /// `background_turns` row id registered by the caller ("" if none).
    pub turn_id: String,
    pub text: String,
    pub success: bool,
    pub latency_ms: u64,
    pub output_tokens: u32,
}

/// Progress callback for a detached (backgrounded) turn. Fired by the
/// heartbeat ticker in the detached supervisor (`note: None`) and by the
/// `report_progress` agent tool when the model volunteers a substantive
/// update (`note: Some(_)`).
pub type ProgressSink = std::sync::Arc<dyn Fn(ProgressEvent) + Send + Sync>;

/// One progress signal from a running turn. `note: None` is a supervisor
/// heartbeat tick ("still alive"); `note: Some(_)` is volunteered by the
/// agent via the `report_progress` tool.
#[derive(Debug, Clone)]
pub struct ProgressEvent {
    /// `background_turns` row id ("" if none registered).
    pub turn_id: String,
    /// Wall-clock seconds since the turn started.
    pub elapsed_secs: u64,
    pub note: Option<String>,
    /// Set when the agent is reporting that it has stopped to wait on someone
    /// (a human or another agent), named here — e.g. "the operator".
    /// `None` for ordinary progress and for heartbeat ticks.
    pub blocked_on: Option<String>,
    /// Set when the agent was blocked past a reasonable wait and chose to
    /// proceed on a stated assumption instead of waiting indefinitely. The
    /// assumption is recorded here so it can be checked and corrected after
    /// the fact rather than silently trusted.
    pub resumed_with_assumption: Option<String>,
}

/// Render a note-bearing progress event as the one-line message a chat
/// surface should show. Never called for a bare heartbeat tick (`note: None`)
/// — each delivery site handles that case on its own, since only they know
/// their own "still running" phrasing. Centralized so the blocked/resumed
/// framing can't drift between Discord's and Telegram's near-identical
/// delivery closures.
pub fn render_progress_note(turn_id: &str, event: &ProgressEvent) -> String {
    let label = if turn_id.is_empty() {
        "Progress".to_string()
    } else {
        format!("Turn `{}`", hq_db::background_turns::short_ref(turn_id))
    };
    let note = event.note.as_deref().unwrap_or("");
    if let Some(assumption) = &event.resumed_with_assumption {
        return format!("{label}: proceeding on assumption ({assumption}) — {note}");
    }
    if let Some(on) = &event.blocked_on {
        let detail = if note.is_empty() {
            "no detail given"
        } else {
            note
        };
        return format!("{label}: blocked, waiting on {on} — {detail}");
    }
    format!("{label}: {note}")
}

/// The part of a parking ack that `parked_ack` and the relays' history rewrite
/// agree on. An empty id means the turn has no registry row.
pub fn parked_marker(turn_id: &str) -> String {
    if turn_id.is_empty() {
        "Running in the background untracked".to_string()
    } else {
        format!(
            "Parked as turn `{}`",
            hq_db::background_turns::short_ref(turn_id)
        )
    }
}

/// The ack sent when a turn outlives the ack window. `turn_id` is `Some` only
/// when a registry row exists, so the user is never handed an id that
/// `resume` or `background_turn_status` cannot find.
pub fn parked_ack(turn_id: Option<&str>) -> String {
    let marker = parked_marker(turn_id.unwrap_or_default());
    format!("This needs longer than a chat turn. {marker}, I'll report back when it's done.")
}

/// Detached-supervisor heartbeat loop: awaits the prompt future to completion,
/// firing `ProgressEvent { note: None }` on each interval tick while it runs.
/// Extracted from the spawned supervisor task so the ticker logic is unit-
/// testable with a fake prompt future. With no sink or no interval (or a
/// zero interval), this is a plain await — the ticker never exists on the
/// pre-detach path, only inside the post-detach supervisor.
pub(super) async fn supervise_detached<F>(
    mut fut: F,
    on_progress: Option<ProgressSink>,
    progress_interval_secs: Option<u64>,
    turn_id: String,
    start: Instant,
) -> F::Output
where
    F: std::future::Future + Unpin,
{
    let (Some(sink), Some(secs)) = (on_progress, progress_interval_secs) else {
        return fut.await;
    };
    if secs == 0 {
        return fut.await;
    }
    let mut interval = tokio::time::interval(Duration::from_secs(secs));
    // The first `interval` tick resolves immediately; consume it so the first
    // heartbeat lands one full period after detach, not at detach time.
    interval.tick().await;
    loop {
        tokio::select! {
            out = &mut fut => return out,
            _ = interval.tick() => {
                sink(ProgressEvent {
                    turn_id: turn_id.clone(),
                    elapsed_secs: start.elapsed().as_secs(),
                    note: None,
                    blocked_on: None,
                    resumed_with_assumption: None,
                });
            }
        }
    }
}
