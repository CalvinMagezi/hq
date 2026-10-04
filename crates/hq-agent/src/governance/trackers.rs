use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Maximum consecutive denials before suggesting a permanent rule.
/// Distilled from claude-code's `DENIAL_LIMITS.maxConsecutive`.
const MAX_CONSECUTIVE_DENIALS: u32 = 3;

/// Maximum total denials (across any tools) before suggesting bypass mode.
/// Distilled from claude-code's `DENIAL_LIMITS.maxTotal`.
const MAX_TOTAL_DENIALS: u32 = 20;

/// Per-tool denial counts, tracked for the lifetime of a session.
#[derive(Debug, Default, Clone)]
struct ToolDenialState {
    /// Denials in a row for this specific tool (reset on success).
    consecutive: u32,
    /// All-time denials for this tool this session.
    total: u32,
}

/// Session-scoped denial tracker, shared across all `GovernedTool` instances.
///
/// Distilled from claude-code's `denialTracking.ts`. Records denials and
/// successes per tool. When thresholds are exceeded, logs a suggestion to
/// create a permanent allow-rule so the user isn't prompted again.
#[derive(Debug, Clone, Default)]
pub struct DenialTracker {
    inner: Arc<Mutex<DenialTrackerInner>>,
}

#[derive(Debug, Default)]
struct DenialTrackerInner {
    by_tool: HashMap<String, ToolDenialState>,
    session_total: u32,
}

impl DenialTracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a denial for `tool_name`. Returns `true` if the denial thresholds
    /// are now exceeded and a permanent rule should be suggested.
    pub fn record_denial(&self, tool_name: &str) -> bool {
        let mut g = self.inner.lock().expect("DenialTracker lock poisoned");
        let state = g.by_tool.entry(tool_name.to_string()).or_default();
        state.consecutive += 1;
        state.total += 1;
        let consecutive = state.consecutive;
        g.session_total += 1;
        consecutive >= MAX_CONSECUTIVE_DENIALS || g.session_total >= MAX_TOTAL_DENIALS
    }

    /// Record a successful tool call for `tool_name`. Resets consecutive count.
    pub fn record_success(&self, tool_name: &str) {
        let mut g = self.inner.lock().expect("DenialTracker lock poisoned");
        if let Some(state) = g.by_tool.get_mut(tool_name) {
            state.consecutive = 0;
        }
    }

    /// Returns `true` if thresholds are already exceeded (checked without recording).
    pub fn is_saturated(&self) -> bool {
        let g = self.inner.lock().expect("DenialTracker lock poisoned");
        g.session_total >= MAX_TOTAL_DENIALS
    }

    /// Session-wide total denials across all tools.
    pub fn session_total(&self) -> u32 {
        let g = self.inner.lock().expect("DenialTracker lock poisoned");
        g.session_total
    }
}

// ─── RepeatCallTracker ──────────────────────────────────────────

/// Ascending consecutive-call counts at which the repeat guard advises the
/// model, once each per run of identical calls. Distilled from dsh's
/// `repeat-tool-reminder` guard package.
const DEFAULT_REPEAT_THRESHOLDS: &[u32] = &[3, 5, 8];

/// Tools exempt from the repeat guard: legitimately called with the same or
/// near-identical arguments many times in a row without being unproductive.
const DEFAULT_REPEAT_EXCLUDED_TOOLS: &[&str] = &["todo_write"];

/// Character cap on the canonicalized-arguments preview shown in the
/// detailed (non-first) advisory message.
const REPEAT_ARGS_PREVIEW_CHARS: usize = 200;

/// Session-scoped tracker for consecutive identical-argument tool calls.
///
/// Distilled from dsh's `repeat-tool-reminder` guard: it never blocks a
/// call, it only ever adds an advisory `context_modifier` so the model sees
/// a nudge on its *next* turn. Keys on `(tool_name, canonicalized_args)` —
/// a run resets the moment either the tool or its (canonicalized) arguments
/// change.
#[derive(Debug, Clone)]
pub struct RepeatCallTracker {
    inner: Arc<Mutex<RepeatCallTrackerInner>>,
    thresholds: &'static [u32],
    excluded_tools: &'static [&'static str],
}

impl Default for RepeatCallTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
struct RepeatCallTrackerInner {
    last_key: Option<(String, String)>,
    consecutive: u32,
    /// Thresholds already fired for the current consecutive run, so each one
    /// advises exactly once rather than on every call past the crossing.
    fired_thresholds: HashSet<u32>,
}

impl RepeatCallTracker {
    /// Create a tracker using the default thresholds `[3, 5, 8]` and default
    /// exclude list (`todo_write`).
    pub fn new() -> Self {
        Self::with_config(DEFAULT_REPEAT_THRESHOLDS, DEFAULT_REPEAT_EXCLUDED_TOOLS)
    }

    /// Create a tracker with explicit thresholds/exclude list.
    pub fn with_config(
        thresholds: &'static [u32],
        excluded_tools: &'static [&'static str],
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RepeatCallTrackerInner::default())),
            thresholds,
            excluded_tools,
        }
    }

    /// Record a call to `tool_name` with `args`. Returns `Some(advisory)` the
    /// first time a consecutive-identical-call threshold is crossed; `None`
    /// otherwise (including every call before/after a threshold, and every
    /// call to an excluded tool).
    pub fn record_call(&self, tool_name: &str, args: &Value) -> Option<String> {
        if self.excluded_tools.contains(&tool_name) {
            return None;
        }
        let canonical = canonicalize_args(args);
        let key = (tool_name.to_string(), canonical.clone());

        let mut g = self.inner.lock().expect("RepeatCallTracker lock poisoned");
        if g.last_key.as_ref() == Some(&key) {
            g.consecutive += 1;
        } else {
            g.last_key = Some(key);
            g.consecutive = 1;
            g.fired_thresholds.clear();
        }
        let consecutive = g.consecutive;

        for &threshold in self.thresholds {
            if consecutive == threshold && g.fired_thresholds.insert(threshold) {
                return Some(if threshold == self.thresholds[0] {
                    format!(
                        "Note: '{tool_name}' has been called with the same arguments \
                         {consecutive} times in a row. Consider whether repeating \
                         this exact call is necessary."
                    )
                } else {
                    let preview = hq_core::text::truncate_chars_with(
                        &canonical,
                        REPEAT_ARGS_PREVIEW_CHARS,
                        "...",
                    );
                    format!(
                        "Note: '{tool_name}' has now been called with identical \
                         arguments {consecutive} times in a row (args: {preview}). \
                         Repeating the same call this many times is unlikely to be \
                         productive — reconsider the approach."
                    )
                });
            }
        }
        None
    }
}

/// Recursively sort object keys so two argument payloads that are equal but
/// differ only in field order canonicalize to the same string.
fn canonicalize_args(args: &Value) -> String {
    fn sort_value(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let sorted: std::collections::BTreeMap<String, Value> = map
                    .iter()
                    .map(|(k, val)| (k.clone(), sort_value(val)))
                    .collect();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(arr) => Value::Array(arr.iter().map(sort_value).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&sort_value(args)).unwrap_or_default()
}
