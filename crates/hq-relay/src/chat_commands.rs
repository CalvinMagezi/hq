//! Command bodies shared by the Telegram and Discord bridges. Each bridge parses
//! the command and sends the reply; these functions decide what the reply is.

use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;
use std::sync::Arc;

use hq_core::types::{ChatMessage, MessageRole, PermissionPreset};
use hq_db::Database;
use hq_db::background_turns::{self, BackgroundTurnRow};
use tokio::sync::Mutex as TokioMutex;

use crate::relay_common::{
    ChannelState, UNWATCH_USAGE, WATCH_USAGE, WatchArgs, lookup_turn, parse_resume_command,
    parse_unwatch_command, parse_watch_command, resume_excerpt,
};
use crate::session_info::{self, ModelResolution};

const FOCUS_NOTE_PREFIX: &str = "[focus] switching topic to: ";

const REGISTRY_UNAVAILABLE: &str =
    "Background turn registry unavailable right now — try again shortly.";

/// What a bridge does after a command: reply and stop, or reply and run a prompt.
pub(crate) enum CommandReply {
    Done(String),
    Redispatch { ack: String, prompt: String },
}

/// Where a command was typed, for checking that a turn belongs here.
struct ChatRef<'a> {
    platform: &'a str,
    chat_id: &'a str,
}

impl ChatRef<'_> {
    fn owns(&self, row: &BackgroundTurnRow) -> bool {
        row.platform == self.platform && row.chat_id == self.chat_id
    }
}

enum RegistryCommand {
    Resume(Option<String>),
    Watch(WatchArgs),
    Unwatch(String),
}

/// Run `resume`, `watch` or `unwatch` when `lower` is one; `None` otherwise.
/// `open_db` runs only for a matched command, so plain messages pay nothing.
pub(crate) fn registry_command(
    lower: &str,
    text: &str,
    platform: &str,
    chat_key: impl Display,
    identity: Option<&str>,
    open_db: impl FnOnce() -> Option<Arc<Database>>,
) -> Option<CommandReply> {
    let usage = |text: &str| Some(CommandReply::Done(text.to_string()));
    let cmd = if let Some(turn_ref) = parse_resume_command(lower) {
        RegistryCommand::Resume(turn_ref)
    } else if let Some(args) = parse_watch_command(lower, text) {
        let Some(args) = args else { return usage(WATCH_USAGE) };
        RegistryCommand::Watch(args)
    } else {
        let turn_ref = parse_unwatch_command(lower)?;
        let Some(turn_ref) = turn_ref else { return usage(UNWATCH_USAGE) };
        RegistryCommand::Unwatch(turn_ref)
    };
    let Some(db) = open_db() else {
        return usage(REGISTRY_UNAVAILABLE);
    };
    let chat_id = chat_key.to_string();
    let chat = ChatRef { platform, chat_id: &chat_id };
    Some(match cmd {
        RegistryCommand::Resume(turn_ref) => resume(&db, &chat, turn_ref),
        RegistryCommand::Watch(args) => watch(&db, &chat, identity, args),
        RegistryCommand::Unwatch(turn_ref) => CommandReply::Done(unwatch(&db, &chat, &turn_ref)),
    })
}

/// `resume` lists interrupted turns; `resume <id>` reruns one.
fn resume(db: &Database, chat: &ChatRef<'_>, turn_ref: Option<String>) -> CommandReply {
    let Some(turn_ref) = turn_ref else {
        return CommandReply::Done(list_interrupted(db, chat));
    };
    let row = match lookup_turn(db, &turn_ref) {
        Ok(row) => row,
        Err(reply) => return CommandReply::Done(reply),
    };
    let short = background_turns::short_ref(&row.id);
    if !chat.owns(&row) {
        return CommandReply::Done(format!(
            "Turn `{short}` belongs to a different chat or platform — resume it where it started."
        ));
    }
    match row.status.as_str() {
        background_turns::STATUS_RUNNING => {
            CommandReply::Done(format!("Turn `{short}` is still running."))
        }
        background_turns::STATUS_COMPLETED | background_turns::STATUS_FAILED => {
            let snippet = row
                .result_text
                .as_deref()
                .map(|t| resume_excerpt(t, 200))
                .unwrap_or_else(|| "(no result recorded)".to_string());
            CommandReply::Done(format!("Turn `{short}` already {}: {snippet}", row.status))
        }
        // 'interrupted' or anything unrecognized: bring it back and rerun it.
        _ => match db.with_conn(|c| background_turns::mark_running(c, &row.id)) {
            Ok(()) => CommandReply::Redispatch {
                ack: format!("Resuming turn `{short}`…"),
                prompt: row.prompt,
            },
            Err(e) => {
                tracing::warn!(%e, turn = %row.id, "resume: mark_running failed");
                CommandReply::Done(REGISTRY_UNAVAILABLE.to_string())
            }
        },
    }
}

fn list_interrupted(db: &Database, chat: &ChatRef<'_>) -> String {
    let rows = db.with_conn(|c| {
        background_turns::list_recent_interrupted(c, chat.platform, chat.chat_id, 5)
    });
    match rows {
        Ok(rows) if rows.is_empty() => "No interrupted background turns to resume here.".to_string(),
        Ok(rows) => {
            let mut lines = vec!["Interrupted background turns:".to_string()];
            for row in &rows {
                lines.push(format!(
                    "• `{}` — {}",
                    background_turns::short_ref(&row.id),
                    resume_excerpt(&row.prompt, 80)
                ));
            }
            lines.push("Say `resume <id>` to rerun one.".to_string());
            lines.join("\n")
        }
        Err(e) => {
            tracing::warn!(%e, "resume: registry list failed");
            REGISTRY_UNAVAILABLE.to_string()
        }
    }
}

/// `/watch`: record a recurring watch row, then fire its prompt once right away.
fn watch(db: &Database, chat: &ChatRef<'_>, identity: Option<&str>, args: WatchArgs) -> CommandReply {
    let turn_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let watch_until = args.expiry_hours.map(|h| now + h * 3600);
    if let Err(e) = db.with_conn(|c| {
        background_turns::insert_watch(
            c,
            &turn_id,
            chat.platform,
            chat.chat_id,
            None,
            identity,
            &args.prompt,
            now,
            args.interval_mins * 60,
            watch_until,
        )
    }) {
        tracing::warn!(%e, platform = chat.platform, "watch: insert_watch failed");
        return CommandReply::Done(REGISTRY_UNAVAILABLE.to_string());
    }
    let short = background_turns::short_ref(&turn_id);
    let expiry_note = match args.expiry_hours {
        Some(h) if h % 24 == 0 => format!(", expires in {}d", h / 24),
        Some(h) => format!(", expires in {h}h"),
        None => String::new(),
    };
    CommandReply::Redispatch {
        ack: format!(
            "Watching every {}m as turn `{short}`{expiry_note}: '{}'. /unwatch `{short}` to stop.",
            args.interval_mins,
            resume_excerpt(&args.prompt, 60),
        ),
        prompt: args.prompt,
    }
}

/// `/unwatch <id>`: cancel a running watch in this chat.
fn unwatch(db: &Database, chat: &ChatRef<'_>, turn_ref: &str) -> String {
    let row = match lookup_turn(db, turn_ref) {
        Ok(row) => row,
        Err(reply) => return reply,
    };
    let short = background_turns::short_ref(&row.id);
    if !chat.owns(&row) {
        return format!(
            "Turn `{short}` belongs to a different chat or platform — stop it where it started."
        );
    }
    if row.kind != background_turns::KIND_WATCH || row.status != background_turns::STATUS_RUNNING {
        return format!("Turn `{short}` is {}.", row.status);
    }
    let now = chrono::Utc::now().timestamp();
    match db.with_conn(|c| background_turns::mark_cancelled(c, &row.id, now)) {
        Ok(()) => format!("Stopped watch `{short}`."),
        Err(e) => {
            tracing::warn!(%e, turn = %row.id, "unwatch: mark_cancelled failed");
            REGISTRY_UNAVAILABLE.to_string()
        }
    }
}

/// `model` lists the backend chain; `model <name>` switches its primary.
/// `cmd` is how the user types the command here, for the hints in the reply.
pub(crate) fn model(name: &str, cmd: &str) -> String {
    let config = hq_core::config::HqConfig::load().unwrap_or_default();
    if name.is_empty() {
        if !config.backends.is_configured() {
            return format!(
                "No explicit backend chain configured. Current model: `{}`.",
                session_info::resolve_session_model(&config)
            );
        }
        let listing =
            session_info::render_chain_listing(&config, "**Available models** (backend chain):");
        return format!("{listing}\n\n`{cmd} <name>` to switch (backend name or model id).");
    }
    match session_info::resolve_model_arg(&config.backends, name) {
        ModelResolution::Backend(backend) if backend == config.backends.primary => {
            format!("`{backend}` is already the primary backend.")
        }
        ModelResolution::Backend(backend) => match session_info::set_primary_backend(&backend) {
            Ok((model, _)) => {
                let model_line = model.map(|m| format!(" (`{m}`)")).unwrap_or_default();
                format!(
                    "Model set: primary backend switched to `{backend}`{model_line}. Applies globally, next message on. `{cmd}` shows the new chain."
                )
            }
            Err(e) => format!("Failed to switch model: {e}"),
        },
        ModelResolution::NoMatch(raw) => {
            format!("No configured backend matches `{raw}`. `{cmd}` lists the chain.")
        }
    }
}

/// Clear the chat and return the reset banner.
pub(crate) async fn reset<K: Eq + Hash>(threads: &TokioMutex<HashMap<K, ChannelState>>, key: &K) -> String {
    threads.lock().await.remove(key);
    session_info::reset_banner_from_config().await
}

/// Signal the running turn to stop and always free the busy gate: a detached
/// turn has no live cancel handle and a watch firing never wires one, so
/// without the unconditional clear the chat would stay wedged until a restart.
pub(crate) async fn cancel<K: Eq + Hash>(threads: &TokioMutex<HashMap<K, ChannelState>>, key: K) -> &'static str {
    let (flag, was_stuck) = {
        let mut t = threads.lock().await;
        let state = t.entry(key).or_insert_with(ChannelState::new_default);
        let was_stuck = state.turn_in_flight;
        state.turn_in_flight = false;
        state.pending_steer = None;
        (state.active_cancel.take(), was_stuck)
    };
    match flag {
        Some(f) => {
            f.store(true, std::sync::atomic::Ordering::Relaxed);
            "Cancel signal sent. Stopping now, including any running command."
        }
        None if was_stuck => {
            "No live handle for the current task (already detached), but cleared the busy flag, so send your message again."
        }
        None => "No active task to cancel.",
    }
}

/// The topic after `/focus` or `!focus`: `None` when `text_lower` is not a
/// focus command, `Some(None)` for the bare form.
pub(crate) fn focus_topic<'a>(text_lower: &str, text: &'a str) -> Option<Option<&'a str>> {
    let is_focus = ["/focus", "!focus"].iter().any(|cmd| {
        text_lower
            .strip_prefix(cmd)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    });
    if !is_focus {
        return None;
    }
    Some(permission_arg(text))
}

/// Clear the chat's history but keep its model and permission pin, and seed
/// the new topic when one is given. Returns the reply.
pub(crate) async fn focus<K: Eq + Hash>(
    threads: &TokioMutex<HashMap<K, ChannelState>>,
    key: K,
    topic: Option<&str>,
) -> String {
    let mut t = threads.lock().await;
    let state = t.entry(key).or_insert_with(ChannelState::new_default);
    let system = state
        .messages
        .first()
        .filter(|m| m.role == MessageRole::System)
        .cloned();
    state.messages.clear();
    let Some(topic) = topic else {
        return "Focus pivoted. History cleared. Send your next message.".to_string();
    };
    state.messages.extend(system);
    state.messages.push(text_message(
        MessageRole::User,
        &format!("{FOCUS_NOTE_PREFIX}{topic}"),
    ));
    format!("Focus pivoted. History cleared. New topic: {topic}")
}

fn text_message(role: MessageRole, content: &str) -> ChatMessage {
    ChatMessage {
        image_parts: Vec::new(),
        role,
        content: content.to_string(),
        tool_calls: vec![],
        tool_call_id: None,
        reasoning_content: None,
    }
}

/// True for `/permission` or `!permission`, with or without an argument.
pub(crate) fn is_permission_command(text_lower: &str) -> bool {
    ["/permission", "!permission"].iter().any(|cmd| {
        text_lower
            .strip_prefix(cmd)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    })
}

/// The argument after the permission command, trimmed; `None` for the bare form.
pub(crate) fn permission_arg(text: &str) -> Option<&str> {
    let (_, rest) = text.trim().split_once(char::is_whitespace)?;
    Some(rest.trim()).filter(|r| !r.is_empty())
}

fn preset_names() -> String {
    let names: Vec<&str> = PermissionPreset::all().iter().map(|p| p.name()).collect();
    names.join(", ")
}

/// Report, pin or clear this chat's permission preset, which the next native
/// session is built with. `cmd` is how the user types the command here.
pub(crate) async fn permission<K: Eq + Hash>(
    threads: &TokioMutex<HashMap<K, ChannelState>>,
    key: K,
    arg: Option<&str>,
    cmd: &str,
) -> String {
    let mut t = threads.lock().await;
    let state = t.entry(key).or_insert_with(ChannelState::new_default);
    let Some(arg) = arg.map(str::to_lowercase) else {
        return match state.pinned_permission_preset {
            Some(p) => format!(
                "Pinned permission preset: `{}` ({}). Send `{cmd} default` to clear it.",
                p.name(),
                p.description()
            ),
            None => format!(
                "No permission preset pinned, so the process default applies. Send `{cmd} <name>` to pin one. Available: {}",
                preset_names()
            ),
        };
    };
    if matches!(arg.as_str(), "default" | "clear" | "auto") {
        state.pinned_permission_preset = None;
        return "Permission preset pin cleared. The process default applies again.".to_string();
    }
    let Some(preset) = PermissionPreset::from_name(&arg) else {
        return format!("Unknown permission preset `{arg}`. Available: {}", preset_names());
    };
    state.pinned_permission_preset = Some(preset);
    format!(
        "Pinned permission preset to `{}` ({}) for this chat.",
        preset.name(),
        preset.description()
    )
}

/// Model and message count for a status reply.
pub(crate) struct StatusFields {
    pub model: String,
    pub messages: usize,
}

/// Read under the lock, then load config after releasing it.
pub(crate) async fn status_fields<K: Eq + Hash>(
    threads: &TokioMutex<HashMap<K, ChannelState>>,
    key: &K,
) -> StatusFields {
    let messages = threads.lock().await.get(key).map_or(0, |s| s.messages.len());
    let config = hq_core::config::HqConfig::load().unwrap_or_default();
    StatusFields {
        model: session_info::resolve_session_model(&config),
        messages,
    }
}

impl StatusFields {
    pub fn text(&self) -> String {
        format!(
            "Status\nModel: {}\nMessages: {}",
            self.model, self.messages
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: ChatRef<'static> = ChatRef { platform: "telegram", chat_id: "1" };

    fn seed_interrupted(db: &Database, id: &str, chat_id: &str) {
        db.with_conn(|c| {
            background_turns::insert(c, id, "telegram", chat_id, None, None, "do it", 1, None)?;
            background_turns::mark_interrupted(c, id, 2)
        })
        .unwrap();
    }

    #[test]
    fn resume_reruns_an_interrupted_turn_in_its_own_chat() {
        let db = Database::open_memory().unwrap();
        seed_interrupted(&db, "abcdef12-0000", "1");
        match resume(&db, &CHAT, Some("abcdef12".into())) {
            CommandReply::Redispatch { ack, prompt } => {
                assert_eq!(ack, "Resuming turn `abcdef12`…");
                assert_eq!(prompt, "do it");
            }
            CommandReply::Done(text) => panic!("expected a rerun, got {text}"),
        }
    }

    #[test]
    fn resume_refuses_a_turn_from_another_chat() {
        let db = Database::open_memory().unwrap();
        seed_interrupted(&db, "abcdef12-0000", "2");
        let CommandReply::Done(text) = resume(&db, &CHAT, Some("abcdef12".into())) else {
            panic!("expected a refusal");
        };
        assert!(text.contains("different chat"));
    }

    #[test]
    fn plain_messages_never_open_the_registry() {
        let reply = registry_command("hello", "hello", "telegram", 1, None, || panic!("opened"));
        assert!(reply.is_none());
    }

    #[test]
    fn permission_command_and_arg_parse() {
        assert!(is_permission_command("/permission read-only"));
        assert!(is_permission_command("!permission"));
        assert!(!is_permission_command("/permissionx"));
        assert!(!is_permission_command("hello permission"));
        assert_eq!(permission_arg("/permission  danger-full-access  "), Some("danger-full-access"));
        assert_eq!(permission_arg("/permission "), None);
    }

    #[tokio::test]
    async fn permission_pins_clears_and_rejects() {
        let threads = TokioMutex::new(HashMap::<u64, ChannelState>::new());
        let name = PermissionPreset::all()[0].name();
        assert!(permission(&threads, 1, Some(name), "!permission").await.starts_with("Pinned"));
        assert!(threads.lock().await[&1].pinned_permission_preset.is_some());
        assert!(permission(&threads, 1, Some("DEFAULT"), "!permission").await.contains("cleared"));
        assert!(permission(&threads, 1, Some("bogus"), "!permission").await.starts_with("Unknown"));
        assert!(permission(&threads, 1, None, "!permission").await.contains("No permission preset"));
    }

    #[tokio::test]
    async fn cancel_frees_a_stuck_slot() {
        let threads = TokioMutex::new(HashMap::<u64, ChannelState>::new());
        threads.lock().await.entry(1).or_insert_with(ChannelState::new_default).turn_in_flight = true;
        assert!(cancel(&threads, 1).await.contains("cleared the busy flag"));
        assert!(!threads.lock().await[&1].turn_in_flight);
        assert_eq!(cancel(&threads, 1).await, "No active task to cancel.");
    }

    #[test]
    fn watch_then_unwatch_round_trips() {
        let db = Database::open_memory().unwrap();
        let args = WatchArgs { interval_mins: 30, expiry_hours: Some(48), prompt: "check".into() };
        let CommandReply::Redispatch { ack, prompt } = watch(&db, &CHAT, None, args) else {
            panic!("expected the first firing");
        };
        assert_eq!(prompt, "check");
        assert!(ack.contains("every 30m") && ack.contains("expires in 2d"));
        let short = ack.split('`').nth(1).unwrap();
        assert_eq!(unwatch(&db, &CHAT, short), format!("Stopped watch `{short}`."));
    }

    #[test]
    fn focus_topic_parses() {
        assert_eq!(focus_topic("/focus", "/focus"), Some(None));
        assert_eq!(focus_topic("!focus tax", "!focus Tax  "), Some(Some("Tax")));
        assert_eq!(focus_topic("/focused", "/focused"), None);
    }

    #[tokio::test]
    async fn focus_note_survives_the_next_turn() {
        let threads = TokioMutex::new(HashMap::<u64, ChannelState>::new());
        threads
            .lock()
            .await
            .entry(1)
            .or_insert_with(ChannelState::new_default)
            .stage_turn("old prompt", "old topic", Vec::new());
        let reply = focus(&threads, 1, Some("tax returns")).await;
        assert!(reply.ends_with("New topic: tax returns"));

        let mut t = threads.lock().await;
        let state = t.get_mut(&1).unwrap();
        state.stage_turn("new prompt", "where do we start?", Vec::new());
        let contents: Vec<&str> = state.messages.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(
            contents,
            ["new prompt", "[focus] switching topic to: tax returns", "where do we start?"]
        );
    }

    #[tokio::test]
    async fn focus_on_a_fresh_chat_keeps_the_note_too() {
        let threads = TokioMutex::new(HashMap::<u64, ChannelState>::new());
        focus(&threads, 1, Some("travel")).await;
        let mut t = threads.lock().await;
        let state = t.get_mut(&1).unwrap();
        state.stage_turn("prompt", "hi", Vec::new());
        assert_eq!(state.messages[0].role, MessageRole::System);
        assert_eq!(state.messages[1].content, "[focus] switching topic to: travel");
    }

    #[tokio::test]
    async fn bare_focus_clears_history() {
        let threads = TokioMutex::new(HashMap::<u64, ChannelState>::new());
        threads
            .lock()
            .await
            .entry(1)
            .or_insert_with(ChannelState::new_default)
            .stage_turn("prompt", "old", Vec::new());
        focus(&threads, 1, None).await;
        assert!(threads.lock().await[&1].messages.is_empty());
    }
}
