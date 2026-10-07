use super::*;

/// Set by governance on calls from a turn that has read untrusted content, so
/// what that turn newly watches starts with Drive off. A caller setting it
/// itself can only give up Drive, never gain it.
pub const UNTRUSTED_TURN_ARG: &str = "untrusted_turn";

/// The web chat a tool call came from, which watches what the call starts or links.
#[derive(Debug, Clone)]
pub struct WatchingChat {
    pub thread: String,
    /// Whether a session this chat newly watches starts with Drive on: the
    /// `agent_host.drive_new_watches` default, never for a turn the driver started
    /// or one answering an `hq_ask`.
    pub drive_new: bool,
    /// The session driver started this turn, so its sends are budgeted and it
    /// may steer only the session its chat drives.
    pub driver_turn: bool,
    /// The chat thread belongs to an `hq_ask`, so what it spawns is capped.
    pub from_ask: bool,
}

/// Set by the gateway when the caller says it runs inside a session HQ spawned
/// (header `x-hq-session-id`, filled from the `HQ_SESSION_ID` the pane was
/// launched with). A caller that sets it itself only restricts itself.
pub const SPAWNED_SESSION_ARG: &str = "_hq_spawned_session";

/// The environment variable HQ sets in every pane it launches, naming the session.
pub const SESSION_ENV: &str = "HQ_SESSION_ID";

/// Argument the gateway sets, and only the gateway, to the session a call came
/// from once that session proved itself with its token. Tools that act on behalf
/// of an agent (messaging, claiming) read the caller from here, never from a
/// field the caller supplies.
pub const CALLER_SESSION_ARG: &str = "_hq_caller_session";

/// The session the gateway attested for this call, if any.
pub fn caller_session(args: &Value) -> Option<&str> {
    args.get(CALLER_SESSION_ARG)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

pub const SPAWNED_REFUSAL: &str =
    "HQ-spawned sessions cannot start further sessions or full-mode asks; ask the owner";

/// The id of the HQ-spawned session this call came from, when the gateway marked it.
pub fn spawned_session(args: &Value) -> Option<&str> {
    args.get(SPAWNED_SESSION_ARG)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

pub fn refuse_if_spawned(args: &Value) -> Result<()> {
    match spawned_session(args) {
        Some(_) => bail!(SPAWNED_REFUSAL),
        None => Ok(()),
    }
}

pub(super) fn agent_host_config() -> hq_core::config::AgentHostConfig {
    hq_core::config::HqConfig::load()
        .map(|c| c.agent_host)
        .unwrap_or_default()
}

/// Taking over an existing session is for the owner's typed turns: not a driver turn, not an ask reply.
pub fn refuse_attach(chat: Option<&WatchingChat>) -> Result<()> {
    match chat {
        Some(c) if c.driver_turn || c.from_ask => {
            bail!("this turn cannot attach or link sessions; tell the user what you need")
        }
        _ => Ok(()),
    }
}

/// Where a session started from: an MCP client with no chat, an `hq_ask` chat, or the owner's chat.
pub fn start_origin(chat: Option<&WatchingChat>) -> &'static str {
    match chat {
        None => registry::ORIGIN_MCP,
        Some(c) if c.from_ask => registry::ORIGIN_ASK,
        Some(_) => registry::ORIGIN_USER,
    }
}

/// Refuse a start that would pass the cap on sessions of its origin. The owner's own chats are not capped.
pub fn check_origin_cap(db: &Arc<Database>, origin: &str) -> Result<()> {
    let cfg = agent_host_config();
    let (cap, setting) = match origin {
        registry::ORIGIN_ASK => (cfg.ask_spawned_session_cap(), "agent_host.max_ask_spawned_sessions"),
        registry::ORIGIN_MCP => (cfg.mcp_started_session_cap(), "agent_host.max_mcp_started_sessions"),
        _ => return Ok(()),
    };
    if db.with_conn(|c| registry::count_running_with_origin(c, origin))? >= cap {
        bail!("{cap} sessions started this way are already running ({setting}); stop one or ask the owner");
    }
    Ok(())
}

/// Remember how a session started, from the `session_id` of a launch report.
pub fn tag_origin(db: &Arc<Database>, report: &Value, origin: &str) {
    if let Some(id) = report.get("session_id").and_then(Value::as_str)
        && let Err(e) = db.with_conn(|c| registry::set_origin(c, id, origin))
    {
        tracing::warn!(session = id, error = %e, "could not record how a session started");
    }
}

/// Whether a marked (HQ-spawned) caller may type into `target`: only its own session.
pub fn spawned_may_target(args: &Value, target: &str) -> Result<()> {
    match spawned_session(args) {
        Some(own) if own != target => bail!(SPAWNED_REFUSAL),
        _ => Ok(()),
    }
}

/// What a driver turn is about to type: text prompts and key presses are budgeted separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendKind {
    Text,
    Keys,
}

/// Claim one of the driver's instructions before sending, in a single conditional write so
/// parallel tool calls cannot overshoot. `Ok(false)` means nothing was claimed because this is
/// not a driver turn. A driver turn may steer only the session its chat drives, within its allowance.
pub fn reserve_send(db: &Arc<Database>, session_id: &str, chat: Option<&WatchingChat>, kind: SendKind) -> Result<bool> {
    let Some(chat) = chat.filter(|c| c.driver_turn) else { return Ok(false) };
    let cfg = agent_host_config();
    let (limit, noun) = match kind {
        SendKind::Text => (cfg.nudge_budget(), "instructions"),
        SendKind::Keys => (cfg.key_allowance(), "key presses"),
    };
    let keys = kind == SendKind::Keys;
    if db.with_conn(|c| registry::reserve_nudge(c, session_id, &chat.thread, keys, limit))? {
        return Ok(true);
    }
    let row = get_row(db, session_id)?;
    if row.owner_thread.as_deref() != Some(chat.thread.as_str()) || !row.drive {
        bail!("Drive is off for session {session_id}, so HQ sends it nothing. Tell the user what you found instead.");
    }
    bail!("the driver has used its {limit} {noun} for session {session_id}. Do not send more; tell the user what the session last did.");
}

/// Close out a send: a failed one gives its reservation back, a successful one is recorded.
pub(crate) fn settle_send(db: &Arc<Database>, session_id: &str, chat: Option<&WatchingChat>, kind: SendKind, reserved: bool, sent: bool) {
    let keys = kind == SendKind::Keys;
    let done = db.with_conn(|c| match (sent, reserved) {
        (false, true) => registry::refund_nudge(c, session_id, keys),
        (false, false) => Ok(()),
        (true, _) => registry::note_send(c, session_id, chat.is_some_and(|c| c.driver_turn), keys),
    });
    if let Err(e) = done {
        tracing::warn!(session = session_id, error = %e, "could not settle a send for the driver budget");
    }
}

/// Run a send under the driver's allowance.
pub(crate) fn metered<T>(db: &Arc<Database>, session_id: &str, chat: Option<&WatchingChat>, kind: SendKind, send: impl FnOnce() -> Result<T>) -> Result<T> {
    let reserved = reserve_send(db, session_id, chat, kind)?;
    let result = send();
    settle_send(db, session_id, chat, kind, reserved, result.is_ok());
    result
}

impl WatchingChat {
    /// The watch one call makes. `drive=false` opts out of Drive (and stops it
    /// on a session this chat already drives); an untrusted turn only loses the default.
    pub fn new_watch(&self, args: &Value) -> NewWatch<'_> {
        let opted_out = args.get("drive").and_then(Value::as_bool) == Some(false);
        let untrusted = args.get(UNTRUSTED_TURN_ARG).and_then(Value::as_bool) == Some(true);
        NewWatch {
            thread: &self.thread,
            drive: self.drive_new && !opted_out && !untrusted,
            opted_out,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NewWatch<'a> {
    pub thread: &'a str,
    pub drive: bool,
    pub opted_out: bool,
}

/// Have the chat watch a session. See `registry::watch_from_chat` for when
/// `drive` applies. Returns whether the session exists.
pub fn start_watch(c: &rusqlite::Connection, session_id: &str, w: NewWatch<'_>) -> Result<bool> {
    start_watch_capped(c, session_id, w, agent_host_config().driven_session_cap())
}

/// `start_watch` with the limit on running driven sessions passed in. A default-on
/// watch that would pass it starts observation-only and says why.
pub fn start_watch_capped(
    c: &rusqlite::Connection,
    session_id: &str,
    w: NewWatch<'_>,
    cap: i64,
) -> Result<bool> {
    let was_driven = registry::get(c, session_id)?.is_some_and(|r| r.drive);
    let found = registry::watch_from_chat(c, session_id, w.thread, w.drive)?;
    if found && w.opted_out {
        registry::set_drive(c, session_id, false)?;
    }
    if found {
        // A default-on watch of a session with no usable goal stays observation-only.
        registry::enforce_gate(c, session_id)?;
        // A session the user already turned on keeps running; the cap limits new default-on starts.
        if !was_driven && registry::count_driven_running(c)? > cap {
            registry::stop_drive(
                c,
                session_id,
                registry::ACTOR_GUARD,
                &format!(
                    "HQ already drives {cap} running sessions (agent_host.max_driven_sessions), so this one starts observation-only."
                ),
            )?;
        }
    }
    Ok(found)
}

/// The goal and definition of done a session is launched or linked with.
#[derive(Debug, Clone, Copy, Default)]
pub struct GoalText<'a> {
    pub goal: Option<&'a str>,
    pub done_criteria: Option<&'a str>,
}

/// A task's title and description as a session goal, so a tracked session has
/// one without being told; the definition of done still has to be stated.
pub(super) fn task_goal(c: &rusqlite::Connection, task_id: &str) -> Option<String> {
    let task = hq_db::tasks::get_task(c, task_id).ok().flatten()?;
    let description = task.description.trim();
    Some(if description.is_empty() { task.title } else { format!("{}: {description}", task.title) })
}
