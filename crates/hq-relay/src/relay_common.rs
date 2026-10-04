//! Channel state and helpers shared by the Telegram and Discord relays.

use chrono::Local;
use hq_db::background_turns::{
    DEFAULT_WATCH_EXPIRY_HOURS, MAX_WATCH_EXPIRY_HOURS, MAX_WATCH_INTERVAL_MINS,
    MIN_WATCH_INTERVAL_MINS,
};
use hq_vault::VaultClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

// ─── Channel state (shared by Discord + Telegram relays) ─────

/// Per-channel conversation state, persisted across service restarts.
/// Old state files still carry `model_override`, `session_ids` and
/// `cc_initialized`; serde ignores them.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ChannelState {
    pub messages: Vec<hq_core::types::ChatMessage>,
    /// Handle to the cancel flag of the currently-running session, if any.
    /// Not persisted — cleared on relay restart.
    #[serde(skip)]
    pub active_cancel: Option<Arc<AtomicBool>>,
    /// Set while a turn is being dispatched for this chat and cleared when it
    /// finishes. Teloxide no longer serializes updates per chat (see
    /// `session.rs`'s dispatcher setup), so this is what keeps two concurrent
    /// messages for the same chat from both starting a turn against the same
    /// history. Not persisted — cleared on relay restart.
    #[serde(skip)]
    pub turn_in_flight: bool,
    /// Handle to the running HQ session's steer inbox, if any. A message that
    /// arrives while `turn_in_flight` is true gets written here instead of
    /// starting a second turn; `AgentSession` drains it between turns/tool
    /// calls and injects it as a `[STEERING]` user message. Not persisted,
    /// cleared on relay restart.
    #[serde(skip)]
    pub pending_steer: Option<Arc<std::sync::Mutex<Option<String>>>>,
    /// Explicit permission preset pin set by `/permission <name>` (Telegram)
    /// or `!permission <name>` (Discord). When Some, the `hq` harness's
    /// native session is built with that preset's `(SecurityProfile,
    /// PermissionMode)` bundle instead of the process default. Cleared by
    /// `/permission default`. `#[serde(default)]` so state persisted before
    /// this field existed still deserializes cleanly.
    #[serde(default)]
    pub pinned_permission_preset: Option<hq_core::types::PermissionPreset>,
    /// Always "hq". Only written, for one release, so a rollback to a binary
    /// that still requires `harness` keeps loading chats saved by this one.
    #[serde(rename = "harness", default = "legacy_harness", skip_deserializing)]
    legacy_harness: String,
}

fn legacy_harness() -> String {
    "hq".to_string()
}

impl ChannelState {
    pub fn new_default() -> Self {
        Self {
            messages: vec![],
            active_cancel: None,
            turn_in_flight: false,
            pending_steer: None,
            pinned_permission_preset: None,
            legacy_harness: legacy_harness(),
        }
    }

    /// The reply for a message that arrives while a turn is running, or `None`
    /// when the chat is free. A running turn with a steer inbox takes the text.
    pub fn busy_reply(&self, text: &str) -> Option<&'static str> {
        if !self.turn_in_flight {
            return None;
        }
        match &self.pending_steer {
            Some(inbox) => {
                queue_steer(inbox, text.to_string());
                Some(STEERING_REPLY)
            }
            None => Some(NO_STEER_INBOX_REPLY),
        }
    }

    /// Take the chat's single-flight slot, or return the busy reply.
    pub fn claim_turn(&mut self, text: &str) -> Result<(), &'static str> {
        if let Some(reply) = self.busy_reply(text) {
            return Err(reply);
        }
        self.turn_in_flight = true;
        // The steer hook registers through a spawned task, so a stale inbox from
        // the previous turn could otherwise swallow the next message.
        self.pending_steer = None;
        Ok(())
    }

    /// Upsert the system prompt, append the user turn, and keep history bounded
    /// to the system prompt plus the last `KEPT_HISTORY` messages.
    pub fn stage_turn(
        &mut self,
        system_prompt: &str,
        text: &str,
        images: Vec<hq_core::types::ImageAttachment>,
    ) {
        match self.messages.first_mut() {
            Some(first) if first.role == hq_core::types::MessageRole::System => {
                first.content = system_prompt.to_string()
            }
            // A history without a system slot (a /focus note, an old saved
            // chat) keeps every message; the prompt goes in front of it.
            _ => self.messages.insert(
                0,
                chat_message(hq_core::types::MessageRole::System, system_prompt, Vec::new()),
            ),
        }
        self.messages
            .push(chat_message(hq_core::types::MessageRole::User, text, images));
        if self.messages.len() > MAX_HISTORY {
            let tail = self.messages.split_off(self.messages.len() - KEPT_HISTORY);
            self.messages.truncate(1);
            self.messages.extend(tail);
        }
    }

    /// Persisted states whose file stem is `prefix` plus a chat key.
    pub fn restore_all<K: std::str::FromStr + Eq + std::hash::Hash>(
        vault_path: &Path,
        prefix: &str,
    ) -> HashMap<K, Self> {
        Self::load_all(vault_path)
            .into_iter()
            .filter_map(|(k, v)| Some((k.strip_prefix(prefix)?.parse().ok()?, v)))
            .collect()
    }

    /// Save channel state to disk for persistence across restarts.
    pub fn save(&self, vault_path: &Path, channel_key: &str) {
        let dir = vault_path.join("_gateway").join("channels");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{channel_key}.json"));
        // Cap messages to last 30 before saving (keep memory bounded)
        let mut capped = self.clone();
        if capped.messages.len() > 30 {
            let system = if !capped.messages.is_empty()
                && capped.messages[0].role == hq_core::types::MessageRole::System
            {
                Some(capped.messages[0].clone())
            } else {
                None
            };
            let recent: Vec<_> = capped.messages[capped.messages.len() - 20..].to_vec();
            capped.messages = match system {
                Some(s) => {
                    let mut v = vec![s];
                    v.extend(recent);
                    v
                }
                None => recent,
            };
        }
        if let Ok(json) = serde_json::to_string_pretty(&capped) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Records the outcome of a dispatched turn into `messages`: the
    /// assistant's reply on success, or a `[turn failed: ...]` marker on
    /// error. Without the error-path marker, `messages` (the exact history
    /// resubmitted to the LLM for this channel's continuity) would keep a
    /// dangling unanswered user turn — a later successful turn would
    /// resubmit `[..., user: "<the message that errored>", user: "<next
    /// message>"]` with no record a failure happened in between. Matches the
    /// same "derived history must honestly reflect what happened, including
    /// failures" principle `turn_reconciler::record_interrupt_thread_entry`
    /// already applies to `_threads/*.jsonl`.
    pub fn record_turn_outcome(&mut self, result: &anyhow::Result<String>) {
        let content = match result {
            Ok(text) => text.clone(),
            Err(e) => format!("[turn failed: {e}]"),
        };
        self.messages.push(hq_core::types::ChatMessage {
            image_parts: Vec::new(),
            role: hq_core::types::MessageRole::Assistant,
            content,
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        });
    }

    /// Swap the parking ack for a detached turn's final result, so
    /// the next turn replays the real answer. Replaced in place: appending a
    /// second assistant message would break role alternation.
    pub fn replace_parked_reply(&mut self, turn_id: &str, text: &str) {
        let marker = hq_agent::native_hq::parked_marker(turn_id);
        let parked = self.messages.iter_mut().rev().find(|m| {
            m.role == hq_core::types::MessageRole::Assistant && m.content.contains(&marker)
        });
        match parked {
            Some(m) => m.content = text.to_string(),
            None => tracing::debug!(turn_id, "no parked reply to replace; history was trimmed"),
        }
    }

    /// Load all persisted channel states from disk.
    pub fn load_all(vault_path: &Path) -> HashMap<String, Self> {
        let dir = vault_path.join("_gateway").join("channels");
        let mut result = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json")
                    && let Ok(data) = std::fs::read_to_string(&path)
                    && let Ok(state) = serde_json::from_str::<ChannelState>(&data)
                {
                    let key = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    if !key.is_empty() {
                        result.insert(key, state);
                    }
                }
            }
        }
        result
    }
}

/// History length that triggers a trim, and how many recent messages survive it.
const MAX_HISTORY: usize = 30;
const KEPT_HISTORY: usize = 20;
const QUOTE_CHARS: usize = 500;
const INSIGHT_LINE_CHARS: usize = 160;

fn chat_message(
    role: hq_core::types::MessageRole,
    content: &str,
    image_parts: Vec<hq_core::types::ImageAttachment>,
) -> hq_core::types::ChatMessage {
    hq_core::types::ChatMessage {
        image_parts,
        role,
        content: content.to_string(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    }
}

/// Mime type for an image file extension, JPEG when unrecognized.
pub(crate) fn image_mime(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" => "image/heic",
        "tiff" => "image/tiff",
        _ => "image/jpeg",
    }
}

/// Prefix a message with the one it replies to, so the model sees the thread.
pub(crate) fn quote_reply(who: &str, quoted: &str, body: &str) -> String {
    let quoted = hq_core::text::truncate_chars(quoted, QUOTE_CHARS);
    format!("[Replying to {who}: \"{quoted}\"]\n\n{body}")
}

const STEERING_REPLY: &str = "Steering, will apply at the next step.";
// A watch firing holds the slot without a steer inbox, so the message has nowhere to go.
const NO_STEER_INBOX_REPLY: &str = "Busy with a background task that can't take mid-run input, so your message wasn't queued. Resend in a moment, or /cancel to clear the slot.";

// ─── FR-017: current-turn images vs. prior-turn history ──────

/// Adds a mid-turn message to the steer inbox. Appends rather than replaces, so
/// several quick messages all reach the running turn.
pub(crate) fn queue_steer(inbox: &std::sync::Mutex<Option<String>>, text: String) {
    let mut slot = inbox.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(match slot.take() {
        Some(prev) => format!("{prev}\n\n{text}"),
        None => text,
    });
}

/// Splits a `ChannelState.messages`-derived `history` (system message
/// already filtered out by the caller) into prior turns plus the current
/// turn's image attachments.
///
/// Every relay dispatch path pushes the current user turn into
/// `ChannelState.messages` (for persistence and so it's visible immediately
/// to `/reset`, transcripts, etc.) BEFORE calling the harness — so by the
/// time `history` is read back here, its last entry IS the current turn.
/// The harness call also passes that same turn's text separately (as
/// `prompt`/`text`/`content`) to `AgentSession::prompt_with_images`/
/// `prompt_stream_with_images`, which pushes its own fresh user message.
/// Leaving the current turn in `history` too would push it TWICE — two
/// consecutive User messages with no assistant reply between them — and
/// only the `history` copy would carry the image, since the second
/// (actually triggering) copy is built fresh from `prompt`/`text`/`content`
/// alone. This showed up as `deepseek-flash` never actually receiving a
/// photo — the image sat on a message the model didn't act on immediately.
///
/// Pops the trailing User-role entry (if present) and returns its
/// `image_parts`; the popped entry's `content` is intentionally discarded,
/// since the caller already has that same text as a separate variable.
pub(crate) fn split_current_turn_images(
    mut history: Vec<hq_core::types::ChatMessage>,
) -> (
    Vec<hq_core::types::ChatMessage>,
    Vec<hq_core::types::ImageAttachment>,
) {
    let images = if history
        .last()
        .is_some_and(|m| m.role == hq_core::types::MessageRole::User)
    {
        history.pop().map(|m| m.image_parts).unwrap_or_default()
    } else {
        Vec::new()
    };
    (history, images)
}

// ─── Durable background-turn `resume` command ────────────────

/// Parse a `resume` command (also `/resume`, `!resume`). Returns `None` when
/// the message is not a resume command, `Some(None)` for the bare list form,
/// and `Some(Some(id))` for `resume <id>`.
pub(crate) fn parse_resume_command(lower: &str) -> Option<Option<String>> {
    let lower = lower.trim();
    for prefix in ["resume", "/resume", "!resume"] {
        if lower == prefix {
            return Some(None);
        }
        if let Some(rest) = lower.strip_prefix(prefix) {
            // Only match when the prefix is followed by whitespace (so
            // "resumex" does not hijack the message).
            if rest.starts_with(char::is_whitespace) {
                let id = rest.trim();
                if !id.is_empty() {
                    return Some(Some(id.to_string()));
                }
            }
        }
    }
    None
}

/// Char-safe excerpt of a stored prompt/result for resume listings.
pub(crate) fn resume_excerpt(text: &str, max_chars: usize) -> String {
    hq_core::text::truncate_chars_with(text, max_chars, "…")
}

// ─── Recurring `watch` command ───────────────────────────────

/// Usage reply for a malformed `/watch` invocation.
pub(crate) const WATCH_USAGE: &str =
    "Usage: `/watch <minutes, 5-1440> [for <hours>h, default 720h/30d] <prompt>`";
/// Usage reply for a malformed `/unwatch` invocation.
pub(crate) const UNWATCH_USAGE: &str = "Usage: `/unwatch <id>`";

/// Reply token a watch firing emits once its condition is confirmed.
pub(crate) const WATCH_DONE_MARKER: &str = "WATCH_DONE";

/// Appended to every watch firing's instructions so a satisfied watch stops itself.
pub(crate) const WATCH_DONE_INSTRUCTION: &str = "When the watched condition is confirmed and no further checks are needed, include the literal token WATCH_DONE in your reply. The watch then delivers that reply once and stops.";

/// Split a firing's reply into the text to deliver and whether it declared the watch done.
pub(crate) fn take_watch_done(text: &str) -> (String, bool) {
    if !text.contains(WATCH_DONE_MARKER) {
        return (text.to_string(), false);
    }
    let mut cleaned = text.to_string();
    for wrapped in ["`WATCH_DONE`", "**WATCH_DONE**", WATCH_DONE_MARKER] {
        cleaned = cleaned.replace(wrapped, "");
    }
    (cleaned.trim().to_string(), true)
}

/// Strip the done marker from a firing's reply; when present, close the watch
/// row (not the firing's own turn row) so it never fires again.
pub(crate) fn settle_watch_firing(
    db: Option<&hq_db::Database>,
    watch_id: &str,
    text: &str,
) -> (String, bool) {
    let (text, done) = take_watch_done(text);
    if done && let Some(db) = db {
        let now = chrono::Utc::now().timestamp();
        if let Err(e) =
            db.with_conn(|c| hq_db::background_turns::mark_completed(c, watch_id, &text, now))
        {
            tracing::warn!(%e, watch_id, "watch: closing a satisfied watch failed");
        }
    }
    (text, done)
}

/// Parsed arguments of a `/watch` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchArgs {
    /// Firing interval in minutes, clamped to `MIN_WATCH_INTERVAL_MINS`..=1440.
    pub interval_mins: i64,
    /// Expiry in hours: `for <N>h` if given, else `DEFAULT_WATCH_EXPIRY_HOURS`.
    /// Always `Some` — there is no "run forever" option by omission anymore.
    pub expiry_hours: Option<i64>,
    /// The prompt to dispatch on each firing (original casing).
    pub prompt: String,
}

/// Split off the first whitespace-delimited token, returning (token, rest)
/// with `rest` trimmed of leading whitespace.
fn split_token(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

/// Parse a `watch` command (also `/watch`, `!watch`). Returns `None` when the
/// message is not a watch command, `Some(None)` for a malformed invocation
/// (caller should reply with [`WATCH_USAGE`]), and `Some(Some(args))` on
/// success. `lower` is the lowercased message text; `text` carries the
/// original casing used for the prompt.
pub(crate) fn parse_watch_command(lower: &str, text: &str) -> Option<Option<WatchArgs>> {
    let lower = lower.trim();
    let text = text.trim();
    for prefix in ["watch", "/watch", "!watch"] {
        if lower == prefix {
            return Some(None);
        }
        if let Some(rest) = lower.strip_prefix(prefix) {
            // Only match when the prefix is followed by whitespace (so
            // "watchdog" does not hijack the message). The prefix is ASCII,
            // so its byte length is the same in `text` and `lower`.
            if !rest.starts_with(char::is_whitespace) {
                continue;
            }
            let orig_rest = &text[prefix.len()..];

            let (mins_tok, r1) = split_token(orig_rest);
            let Ok(mins) = mins_tok.parse::<i64>() else {
                return Some(None);
            };
            let interval_mins = mins.clamp(MIN_WATCH_INTERVAL_MINS, MAX_WATCH_INTERVAL_MINS);

            // Optional `for <N>h` expiry clause; omitted means
            // DEFAULT_WATCH_EXPIRY_HOURS, not forever (see its doc comment).
            let (expiry_hours, prompt_src) = {
                let (maybe_for, r2) = split_token(r1);
                if maybe_for.eq_ignore_ascii_case("for") {
                    let (hours_tok, r3) = split_token(r2);
                    let hours_lower = hours_tok.to_lowercase();
                    let digits = hours_lower.strip_suffix('h').unwrap_or(&hours_lower);
                    let Ok(hours) = digits.parse::<i64>() else {
                        return Some(None);
                    };
                    if !(1..=MAX_WATCH_EXPIRY_HOURS).contains(&hours) {
                        return Some(None);
                    }
                    (Some(hours), r3)
                } else {
                    (Some(DEFAULT_WATCH_EXPIRY_HOURS), r1)
                }
            };

            let prompt = prompt_src.trim();
            if prompt.is_empty() {
                return Some(None);
            }
            return Some(Some(WatchArgs {
                interval_mins,
                expiry_hours,
                prompt: prompt.to_string(),
            }));
        }
    }
    None
}

/// Parse an `unwatch` command (also `/unwatch`, `!unwatch`). Returns `None`
/// when the message is not an unwatch command, `Some(None)` for the bare
/// form (caller should reply with [`UNWATCH_USAGE`]), and `Some(Some(id))`
/// for `unwatch <id>`.
pub(crate) fn parse_unwatch_command(lower: &str) -> Option<Option<String>> {
    let lower = lower.trim();
    for prefix in ["unwatch", "/unwatch", "!unwatch"] {
        if lower == prefix {
            return Some(None);
        }
        if let Some(rest) = lower.strip_prefix(prefix)
            && rest.starts_with(char::is_whitespace) {
                let id = rest.trim();
                if !id.is_empty() {
                    return Some(Some(id.to_string()));
                }
            }
    }
    None
}

// ─── System prompt loader ────────────────────────────────────

/// Load the HQ soul from the vault. Delegates to the canonical loader in `hq-vault`.
pub(crate) fn load_system_prompt(vault: &VaultClient) -> String {
    hq_vault::system::load_soul(vault.vault_path())
}

/// Stable content leads so DeepSeek's prefix KV cache gets a long hit.
///
/// This no longer enumerates tools. It used to carry a hand-maintained list
/// of sixteen, which drifted from the real registry. `SessionBuilder`'s
/// `build_harness_block` now injects the real tool notes and machine profile.
pub(crate) fn load_system_prompt_with_env(vault: &VaultClient) -> String {
    let soul = load_system_prompt(vault);
    let vault_path = vault.vault_path();

    // Must agree with `resolve_session_model` (chain primary → relay.model →
    // default_model) — this used to skip the `backends` chain entirely and
    // read only `relay.model`/`default_model`, so a configured chain's
    // primary model (e.g. Copilot's `gemini-3.8-flash`) never reached this
    // string: the agent asserted its stale `default_model` identity even
    // while actually streaming from the chain's real primary.
    let config = hq_core::config::HqConfig::load().unwrap_or_default();
    let model = hq_core::config::resolve_session_model(&config);

    let vault_path_str = vault_path.display().to_string();

    let now = Local::now();
    let date_str = now.format("%A, %Y-%m-%d %H:%M %:z").to_string();

    // ── Vault memory: core context + recent insights ─────────────
    let memory_block = load_memory_summary(vault_path);

    let runtime_block = format!(
        r#"## Live Context — {date_str}

**You are HQ**, running as `{model}` on this machine's Agent-HQ relay. You are NOT a generic LLM. Do not say you cannot identify your model. Per-message caller identity (owner vs guest) is appended by the Telegram handler when applicable.

- **Vault**: `{vault_path_str}`

Your tools are native function calls — invoke them directly, never with an
`ACTION:` prefix and never as a bash command written into a markdown block.
The Environment and Tool Usage Notes sections below list what you have and
how to use the ones with gotchas.

{memory_block}
---

"#
    );

    // SOUL leads: stable identity prefix maximises DeepSeek's prefix KV cache hit rate.
    // Live runtime block (date, memory) trails so volatility doesn't bust the cache.
    format!("{soul}\n\n---\n\n{runtime_block}")
}

/// Extract the Core Context summary + top 3 recent insights from MEMORY.md.
fn load_memory_summary(vault_path: &std::path::Path) -> String {
    let path = vault_path.join("_system/MEMORY.md");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return String::new();
    };

    let mut out = String::new();

    // Pull the "Recent Insights" section (first 3 bullet items)
    let insights: Vec<&str> = content
        .lines()
        .skip_while(|l| !l.contains("Recent Insights"))
        .skip(1)
        .filter(|l| l.trim_start().starts_with("- "))
        .take(3)
        .collect();

    if !insights.is_empty() {
        out.push_str("**Vault Insights** (recent):\n");
        for line in &insights {
            let truncated = hq_core::text::truncate_chars(line, INSIGHT_LINE_CHARS);
            out.push_str(&format!("{truncated}\n"));
        }
    }

    out
}

// ─── Relay status tracking ──────────────────────────────────

/// Allowlist of valid platform identifiers. Prevents path traversal when
/// platform is used as a filename component.
fn valid_platform(platform: &str) -> bool {
    matches!(platform, "discord" | "telegram")
}

/// Write explicit connection status for a relay bridge to `_system/relay-status/{platform}.json`.
/// Each bridge calls this on connect, disconnect, and error so the web API reads real state
/// rather than inferring it from stale file artifacts.
pub(crate) fn write_relay_status(
    vault_path: &std::path::Path,
    platform: &str,
    status: &str,
    detail: Option<&str>,
    error_message: Option<&str>,
) {
    if !valid_platform(platform) {
        tracing::warn!(
            "write_relay_status: rejected unknown platform {:?}",
            platform
        );
        return;
    }
    let dir = vault_path.join("_system").join("relay-status");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{platform}.json"));
    let payload = serde_json::json!({
        "status": status,
        "last_seen": chrono::Utc::now().to_rfc3339(),
        "detail": detail,
        "error_message": error_message,
    });
    if let Ok(json) = serde_json::to_string_pretty(&payload) {
        let _ = std::fs::write(&path, json);
    }
}

// ─── Message chunking ────────────────────────────────────────

pub fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }
        // Cut on a char boundary: slicing mid-character (an em-dash, an emoji) panics.
        // Always take at least one char, or a limit below its width would loop forever.
        let limit = remaining.floor_char_boundary(max_len).max(remaining.ceil_char_boundary(1));
        let search = &remaining[..limit];
        let split_at = search
            .rfind("\n\n")
            .or_else(|| search.rfind('\n'))
            .or_else(|| search.rfind(' '))
            .filter(|&i| i > 0)
            .unwrap_or(limit);
        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start();
    }
    chunks
}

/// Sends one chat message on a surface. Implementations spawn the send and
/// log failures themselves, so callers never block on delivery.
pub(crate) type ChatSender = Arc<dyn Fn(String) + Send + Sync>;

/// Cut `text` to `max` characters, ending with an ellipsis when cut.
pub(crate) fn truncate_chars(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let kept = hq_core::text::truncate_chars(&text, max.saturating_sub(1));
    format!("{kept}…")
}

/// Insert a turn's registry row. `Some(id)` only when the row now exists, so
/// callers never quote an id to the user that nothing can look up.
pub(crate) fn register_turn(
    db: Option<&hq_db::Database>,
    turn_id: &str,
    platform: &str,
    chat_id: &str,
    identity: Option<&str>,
    prompt: &str,
) -> Option<String> {
    let db = db?;
    let now = chrono::Utc::now().timestamp();
    match db.with_conn(|c| {
        hq_db::background_turns::insert(
            c, turn_id, platform, chat_id, None, identity, prompt, now, None,
        )
    }) {
        Ok(()) => Some(turn_id.to_string()),
        Err(e) => {
            tracing::warn!(%e, platform, "background_turns insert failed, turn runs untracked");
            None
        }
    }
}

/// Resolve a user-typed turn ref (short or full id), or the reply saying why not.
pub(crate) fn lookup_turn(
    db: &hq_db::Database,
    turn_ref: &str,
) -> Result<hq_db::background_turns::BackgroundTurnRow, String> {
    // Acks show the ref in backticks, so a copied ref often keeps them.
    let turn_ref = turn_ref.trim_matches('`');
    match db.with_conn(|c| hq_db::background_turns::get_by_prefix(c, turn_ref)) {
        Ok(Some(row)) => Ok(row),
        Ok(None) => Err(format!("No background turn `{turn_ref}` found.")),
        Err(e) => {
            tracing::warn!(%e, turn_ref, "background turn lookup failed");
            Err(format!("Can't resolve `{turn_ref}`: {e}"))
        }
    }
}

/// Close a `background_turns` row as completed or failed. Best-effort.
pub(crate) fn close_turn_row(db: Option<&hq_db::Database>, turn_id: &str, text: &str, success: bool) {
    let Some(db) = db else { return };
    let now = chrono::Utc::now().timestamp();
    let update = if success {
        db.with_conn(|c| hq_db::background_turns::mark_completed(c, turn_id, text, now))
    } else {
        db.with_conn(|c| hq_db::background_turns::mark_failed(c, turn_id, text, now))
    };
    if let Err(e) = update {
        tracing::warn!(%e, turn_id, "background_turns close-out failed");
    }
}

/// Close a detached turn's registry row and deliver its result.
pub(crate) fn finish_detached(
    db: Option<&hq_db::Database>,
    send: &ChatSender,
    outcome: &hq_agent::native_hq::DetachedTurnOutcome,
) {
    close_turn_row(db, &outcome.turn_id, &outcome.text, outcome.success);
    let msg = match (outcome.text.trim().is_empty(), outcome.success) {
        (false, _) => outcome.text.clone(),
        (true, true) => "(turn completed with no output)".to_string(),
        (true, false) => "(turn failed with no error output)".to_string(),
    };
    send(msg);
}

/// Progress delivery for a relay turn: `report_progress` notes (note: Some)
/// update the row's blocked state, land in the thread file and go out as a
/// message; supervisor heartbeats (note: None) go out as a "still running"
/// notice. The supervisor interval is the only throttle.
pub(crate) fn progress_sink(
    db: Option<Arc<hq_db::Database>>,
    vault_path: std::path::PathBuf,
    identity: hq_core::identity::RequestIdentity,
    max_chars: usize,
    send: ChatSender,
) -> hq_agent::native_hq::ProgressSink {
    Arc::new(move |event: hq_agent::native_hq::ProgressEvent| {
        if event.note.is_none() {
            send(format!(
                "Turn `{}` still running (elapsed {}m).",
                hq_db::background_turns::short_ref(&event.turn_id),
                event.elapsed_secs / 60
            ));
            return;
        }
        // The reconciler reads the blocked state back after this process is gone.
        if let Some(db) = &db {
            let (turn_id, blocked_on, detail) = (
                event.turn_id.clone(),
                event.blocked_on.clone(),
                event.note.clone(),
            );
            let result = db.with_conn(move |c| match &blocked_on {
                Some(on) => {
                    hq_db::background_turns::set_blocked(c, &turn_id, on, detail.as_deref())
                }
                None => hq_db::background_turns::clear_blocked(c, &turn_id),
            });
            if let Err(e) = result {
                tracing::warn!(%e, "progress blocked-state update failed");
            }
        }
        let entry = hq_agent::native_hq::render_progress_note(&event.turn_id, &event);
        if let Err(e) =
            hq_agent::threads::append_thread_entry(&vault_path, &identity, "assistant", &entry)
        {
            tracing::warn!(%e, "progress thread append failed");
        }
        send(truncate_chars(entry, max_chars));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_state_still_carries_harness_for_rollback() {
        let json = serde_json::to_value(ChannelState::new_default()).unwrap();
        assert_eq!(json["harness"], "hq");
        let back: ChannelState = serde_json::from_value(json).unwrap();
        assert!(back.messages.is_empty());
    }

    fn msg(role: hq_core::types::MessageRole, content: &str) -> hq_core::types::ChatMessage {
        hq_core::types::ChatMessage {
            role,
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
            image_parts: vec![],
        }
    }

    #[test]
    fn split_current_turn_images_pops_the_trailing_user_message_and_returns_its_images() {
        let img = hq_core::types::ImageAttachment {
            path: std::path::PathBuf::from("/tmp/x.jpg"),
            mime_type: "image/jpeg".to_string(),
        };
        let mut current = msg(hq_core::types::MessageRole::User, "describe this");
        current.image_parts = vec![img.clone()];
        let history = vec![
            msg(hq_core::types::MessageRole::User, "earlier turn"),
            msg(hq_core::types::MessageRole::Assistant, "earlier reply"),
            current,
        ];

        let (remaining, images) = split_current_turn_images(history);

        assert_eq!(remaining.len(), 2, "current turn must be popped off");
        assert_eq!(remaining[0].content, "earlier turn");
        assert_eq!(remaining[1].content, "earlier reply");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].path, img.path);
    }

    #[test]
    fn split_current_turn_images_on_empty_history_returns_nothing() {
        let (remaining, images) = split_current_turn_images(vec![]);
        assert!(remaining.is_empty());
        assert!(images.is_empty());
    }

    #[test]
    fn split_current_turn_images_leaves_a_trailing_assistant_message_untouched() {
        // Defensive: only pop when the trailing entry is actually a User
        // turn, so an unexpected history shape doesn't silently drop the
        // last assistant reply.
        let history = vec![
            msg(hq_core::types::MessageRole::User, "turn"),
            msg(hq_core::types::MessageRole::Assistant, "reply"),
        ];
        let (remaining, images) = split_current_turn_images(history);
        assert_eq!(remaining.len(), 2);
        assert!(images.is_empty());
    }

    /// Workstream G (event-sourced history audit) regression test: a failed
    /// turn must not leave `messages` with a dangling unanswered user turn,
    /// or the next successful turn would resubmit two consecutive user
    /// messages to the LLM with no record a failure happened in between.
    #[test]
    fn replace_parked_reply_swaps_the_ack_in_place() {
        let mut state = ChannelState::new_default();
        state.record_turn_outcome(&Ok(hq_agent::native_hq::parked_ack(Some(
            "t9-0000-long-uuid",
        ))));
        let before = state.messages.len();
        state.replace_parked_reply("t9-0000-long-uuid", "Done: 3 files migrated.");
        assert_eq!(state.messages.len(), before);
        assert_eq!(
            state.messages.last().unwrap().content,
            "Done: 3 files migrated."
        );
        state.record_turn_outcome(&Ok(hq_agent::native_hq::parked_ack(None)));
        state.replace_parked_reply("", "Untracked result.");
        assert_eq!(state.messages.last().unwrap().content, "Untracked result.");
    }

    #[test]
    fn ack_quotes_an_id_only_when_the_registry_row_exists() {
        let db = hq_db::Database::open_memory().unwrap();
        let tracked = register_turn(Some(&db), "abcdef123456", "telegram", "c1", None, "p");
        assert_eq!(tracked.as_deref(), Some("abcdef123456"));
        assert!(lookup_turn(&db, "`abcdef12`").is_ok());
        assert!(hq_agent::native_hq::parked_ack(tracked.as_deref()).contains("`abcdef12`"));

        let untracked = register_turn(None, "fedcba654321", "telegram", "c1", None, "p");
        assert!(untracked.is_none());
        assert!(!hq_agent::native_hq::parked_ack(untracked.as_deref()).contains('`'));
    }

    #[test]
    fn watch_done_marker_is_stripped_and_closes_only_the_watch_row() {
        assert_eq!(
            take_watch_done("still pending"),
            ("still pending".to_string(), false)
        );
        assert_eq!(
            take_watch_done("PR merged. `WATCH_DONE`"),
            ("PR merged.".to_string(), true)
        );

        let db = hq_db::Database::open_memory().unwrap();
        db.with_conn(|c| {
            hq_db::background_turns::insert_watch(
                c, "w-1", "telegram", "1", None, None, "p", 0, 300, None,
            )
        })
        .unwrap();
        let (text, done) = settle_watch_firing(Some(&db), "w-1", "Deploy is green.\nWATCH_DONE");
        assert!(done);
        assert_eq!(text, "Deploy is green.");
        let row = db
            .with_conn(|c| hq_db::background_turns::get(c, "w-1"))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, hq_db::background_turns::STATUS_COMPLETED);
        assert_eq!(row.result_text.as_deref(), Some("Deploy is green."));

        let (_, done) = settle_watch_firing(Some(&db), "w-1", "no marker");
        assert!(!done);
    }

    #[test]
    fn truncate_chars_cuts_on_characters_not_bytes() {
        assert_eq!(truncate_chars("héllo".to_string(), 10), "héllo");
        assert_eq!(truncate_chars("héllo wörld".to_string(), 5), "héll…");
    }

    #[test]
    fn progress_sink_sends_notes_and_heartbeats() {
        let vault = tempfile::TempDir::new().unwrap();
        let sent = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let send: ChatSender = {
            let sent = sent.clone();
            Arc::new(move |m| sent.lock().unwrap().push(m))
        };
        let sink = progress_sink(
            None,
            vault.path().to_path_buf(),
            hq_core::identity::RequestIdentity::from_discord(7),
            1900,
            send,
        );
        let event = |note: Option<&str>| hq_agent::native_hq::ProgressEvent {
            turn_id: "t1".to_string(),
            elapsed_secs: 180,
            note: note.map(str::to_string),
            blocked_on: None,
            resumed_with_assumption: None,
        };
        sink(event(Some("halfway")));
        sink(event(None));
        let sent = sent.lock().unwrap();
        assert_eq!(sent[0], "Turn `t1`: halfway");
        assert_eq!(sent[1], "Turn `t1` still running (elapsed 3m).");
    }

    #[test]
    fn record_turn_outcome_appends_assistant_reply_on_success() {
        let mut state = ChannelState::new_default();
        state.messages.push(hq_core::types::ChatMessage {
            image_parts: Vec::new(),
            role: hq_core::types::MessageRole::User,
            content: "hello".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        });

        state.record_turn_outcome(&Ok("hi there".to_string()));

        assert_eq!(state.messages.len(), 2);
        assert_eq!(
            state.messages[1].role,
            hq_core::types::MessageRole::Assistant
        );
        assert_eq!(state.messages[1].content, "hi there");
    }

    #[test]
    fn record_turn_outcome_appends_failure_marker_instead_of_leaving_a_dangling_user_turn() {
        let mut state = ChannelState::new_default();
        state.messages.push(hq_core::types::ChatMessage {
            image_parts: Vec::new(),
            role: hq_core::types::MessageRole::User,
            content: "run the deploy".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        });

        state.record_turn_outcome(&Err(anyhow::anyhow!("harness timed out")));

        assert_eq!(state.messages.len(), 2);
        assert_eq!(
            state.messages[1].role,
            hq_core::types::MessageRole::Assistant
        );
        assert!(state.messages[1].content.contains("turn failed"));
        assert!(state.messages[1].content.contains("harness timed out"));

        // A subsequent successful turn's user message no longer lands right
        // after an unanswered one — the log stays a coherent alternation.
        state.messages.push(hq_core::types::ChatMessage {
            image_parts: Vec::new(),
            role: hq_core::types::MessageRole::User,
            content: "try again".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        });
        state.record_turn_outcome(&Ok("done".to_string()));
        assert_eq!(state.messages.len(), 4);
        let roles: Vec<_> = state.messages.iter().map(|m| m.role.clone()).collect();
        assert_eq!(
            roles,
            vec![
                hq_core::types::MessageRole::User,
                hq_core::types::MessageRole::Assistant,
                hq_core::types::MessageRole::User,
                hq_core::types::MessageRole::Assistant,
            ]
        );
    }

    #[test]
    fn watch_parse_plain_form() {
        let text = "/watch 15 check whether the agy review is done";
        let lower = text.to_lowercase();
        let args = parse_watch_command(&lower, text).unwrap().unwrap();
        assert_eq!(args.interval_mins, 15);
        // Omitting `for <N>h` no longer means "forever" — it means
        // DEFAULT_WATCH_EXPIRY_HOURS (see that const's doc comment for why).
        assert_eq!(args.expiry_hours, Some(DEFAULT_WATCH_EXPIRY_HOURS));
        assert_eq!(args.prompt, "check whether the agy review is done");
    }

    #[test]
    fn watch_parse_for_hours_form() {
        let text = "/watch 30 for 4h keep an eye on the build";
        let lower = text.to_lowercase();
        let args = parse_watch_command(&lower, text).unwrap().unwrap();
        assert_eq!(args.interval_mins, 30);
        assert_eq!(args.expiry_hours, Some(4));
        assert_eq!(args.prompt, "keep an eye on the build");
    }

    #[test]
    fn watch_parse_interval_is_clamped() {
        let text = "/watch 0 ping me";
        let lower = text.to_lowercase();
        let args = parse_watch_command(&lower, text).unwrap().unwrap();
        assert_eq!(args.interval_mins, MIN_WATCH_INTERVAL_MINS);

        let text = "/watch 99999 ping me";
        let lower = text.to_lowercase();
        let args = parse_watch_command(&lower, text).unwrap().unwrap();
        assert_eq!(args.interval_mins, 1440);
    }

    #[test]
    fn watch_parse_prompt_keeps_original_case() {
        let text = "/Watch 5 Check The PR Status";
        let lower = text.to_lowercase();
        let args = parse_watch_command(&lower, text).unwrap().unwrap();
        assert_eq!(args.prompt, "Check The PR Status");
    }

    #[test]
    fn watch_parse_missing_prompt_is_usage_error() {
        let lower = "/watch 15";
        assert_eq!(parse_watch_command(lower, lower), Some(None));
        let lower = "/watch";
        assert_eq!(parse_watch_command(lower, lower), Some(None));
        let lower = "/watch 15 for 2h";
        assert_eq!(parse_watch_command(lower, lower), Some(None));
    }

    #[test]
    fn watch_parse_bad_interval_is_usage_error() {
        let lower = "/watch soon check the thing";
        assert_eq!(parse_watch_command(lower, lower), Some(None));
        let lower = "/watch 15 for xh check the thing";
        assert_eq!(parse_watch_command(lower, lower), Some(None));
    }

    #[test]
    fn watch_parse_ignores_non_watch_messages() {
        assert_eq!(
            parse_watch_command("watchdog status", "watchdog status"),
            None
        );
        assert_eq!(parse_watch_command("hello there", "hello there"), None);
    }

    #[test]
    fn unwatch_parse_forms() {
        assert_eq!(
            parse_unwatch_command("/unwatch abc-123"),
            Some(Some("abc-123".to_string()))
        );
        assert_eq!(
            parse_unwatch_command("unwatch abc-123"),
            Some(Some("abc-123".to_string()))
        );
        assert_eq!(parse_unwatch_command("/unwatch"), Some(None));
        assert_eq!(parse_unwatch_command("/unwatchx foo"), None);
        assert_eq!(parse_unwatch_command("hello"), None);
    }

    #[test]
    fn a_busy_chat_steers_when_it_can_and_says_so_when_it_cannot() {
        let mut state = ChannelState::new_default();
        assert_eq!(state.busy_reply("x"), None);
        assert_eq!(state.claim_turn("first"), Ok(()));
        assert_eq!(state.claim_turn("second"), Err(NO_STEER_INBOX_REPLY));
        let inbox = Arc::new(std::sync::Mutex::new(None));
        state.pending_steer = Some(inbox.clone());
        assert_eq!(state.claim_turn("go left"), Err(STEERING_REPLY));
        assert_eq!(inbox.lock().unwrap().as_deref(), Some("go left"));
    }

    #[test]
    fn state_files_written_before_the_harness_field_went_still_load() {
        let old = r#"{"messages":[],"harness":"cursor","model_override":"x","pinned_permission_preset":null}"#;
        assert!(serde_json::from_str::<ChannelState>(old).is_ok());
    }

    #[test]
    fn staging_refreshes_the_system_prompt_and_trims_to_the_recent_tail() {
        let mut state = ChannelState::new_default();
        for i in 0..MAX_HISTORY {
            state.stage_turn(&format!("sys {i}"), &format!("msg {i}"), Vec::new());
        }
        assert_eq!(state.messages.len(), 1 + KEPT_HISTORY);
        assert_eq!(state.messages[0].content, "sys 29");
        assert_eq!(state.messages.last().unwrap().content, "msg 29");
    }

    #[test]
    fn staging_never_overwrites_a_leading_user_message() {
        let mut state = ChannelState::new_default();
        state.messages.push(chat_message(
            hq_core::types::MessageRole::User,
            "[focus] switching topic to: tax",
            Vec::new(),
        ));
        state.stage_turn("prompt", "next", Vec::new());
        let contents: Vec<&str> = state.messages.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(contents, ["prompt", "[focus] switching topic to: tax", "next"]);
        assert_eq!(state.messages[0].role, hq_core::types::MessageRole::System);
    }

    #[test]
    fn steers_queue_instead_of_overwriting() {
        let inbox = std::sync::Mutex::new(None);
        queue_steer(&inbox, "first".into());
        queue_steer(&inbox, "second".into());
        assert_eq!(inbox.lock().unwrap().as_deref(), Some("first\n\nsecond"));
    }
}
