//! Harness-agnostic session manager: spawn, monitor, steer, stop, and resume
//! long-lived external agent CLIs (claude-code, cursor, opencode, pi, kimi,
//! codex, qwen, antigravity, github-copilot) inside Herdr, on this machine or
//! on a remote host, tracked in the `harness_sessions` table.
//!
//! These tools only mutate sessions HQ launched. Agents a person started by
//! hand are visible through `herdr_agents`/`herdr_read` and steerable only
//! through `herdr_send`.

pub mod dismiss;
mod coalesce;
pub mod handoff;
pub mod mission;
mod preflight;
pub mod spec;
pub mod tools;

use crate::herdr::{
    self, AgentInfo, AgentStatus, HerdrError, HerdrHost, LaunchRequest, Launched, PromptOutcome,
};
use anyhow::{Result, bail};
use hq_db::Database;
use hq_db::harness_sessions_registry::{self as registry, HarnessSessionRow, Placement};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use spec::SPECS;
pub use spec::{Harness, HarnessSessionSpec, ResumeStrategy, resolve, resolve_in, spec_for};

const SESSION_DIR_ROOT: &str = "_data/session-dirs";

/// Set by governance on calls from a turn that has read untrusted content, so
/// what that turn newly watches starts with Drive off. A caller setting it
/// itself can only give up Drive, never gain it.
pub const UNTRUSTED_TURN_ARG: &str = "untrusted_turn";

/// The web chat a tool call came from, which watches what the call starts or links.
#[derive(Debug, Clone)]
pub struct WatchingChat {
    pub thread: String,
    /// Whether a session this chat newly watches starts with Drive on: the
    /// `herdr.drive_new_watches` default, never for a turn the driver started
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

fn herdr_config() -> hq_core::config::HerdrConfig {
    hq_core::config::HqConfig::load()
        .map(|c| c.herdr)
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
    let cfg = herdr_config();
    let (cap, setting) = match origin {
        registry::ORIGIN_ASK => (cfg.ask_spawned_session_cap(), "herdr.max_ask_spawned_sessions"),
        registry::ORIGIN_MCP => (cfg.mcp_started_session_cap(), "herdr.max_mcp_started_sessions"),
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
    let cfg = herdr_config();
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
    start_watch_capped(c, session_id, w, herdr_config().driven_session_cap())
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
                    "HQ already drives {cap} running sessions (herdr.max_driven_sessions), so this one starts observation-only."
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
fn task_goal(c: &rusqlite::Connection, task_id: &str) -> Option<String> {
    let task = hq_db::tasks::get_task(c, task_id).ok().flatten()?;
    let description = task.description.trim();
    Some(if description.is_empty() { task.title } else { format!("{}: {description}", task.title) })
}

/// How long to wait for an agent to reach its prompt after a trust dialog was
/// accepted for it.
const TRUST_SETTLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Screen lines shown to the caller when a launch stops at a dialog.
const BLOCKED_SCREEN_LINES: usize = 40;

/// Longest a single `harness_session_wait` may block.
pub const MAX_WAIT: Duration = Duration::from_secs(300);

/// What one host said about its agents during a sweep: the agents, or why it
/// could not be asked.
pub type HostPoll = HashMap<String, std::result::Result<Vec<AgentInfo>, String>>;

/// Where a stored session stands right now.
#[derive(Debug, Clone, PartialEq)]
pub enum Liveness {
    Alive(Box<AgentInfo>),
    /// The host answered and the agent is not there.
    Gone,
    /// The host could not be asked; nothing is known about the agent.
    HostUnreachable(String),
}

use herdr::blocking;

/// Ask each host named by `rows` for its agents, once per host.
pub fn poll_hosts(rows: &[HarnessSessionRow]) -> HostPoll {
    poll_hosts_with(rows, |name| herdr::host(Some(name)))
}

pub fn poll_hosts_with(
    rows: &[HarnessSessionRow],
    resolve: impl Fn(&str) -> anyhow::Result<HerdrHost>,
) -> HostPoll {
    let mut polled = HostPoll::new();
    for row in rows {
        polled.entry(row.host.clone()).or_insert_with(|| {
            resolve(&row.host)
                .map_err(|e| e.to_string())
                .and_then(|h| h.agents().map_err(|e| e.to_string()))
        });
    }
    polled
}

pub fn liveness(polled: &HostPoll, row: &HarnessSessionRow) -> Liveness {
    match polled.get(&row.host) {
        Some(Ok(agents)) => agents
            .iter()
            .find(|a| a.name.as_deref() == Some(row.agent_name.as_str()))
            .map_or(Liveness::Gone, |a| Liveness::Alive(Box::new(a.clone()))),
        Some(Err(detail)) => Liveness::HostUnreachable(detail.clone()),
        None => Liveness::HostUnreachable(format!("host '{}' was not polled", row.host)),
    }
}

/// Herdr agent names are `[a-z][a-z0-9_-]{0,31}`; the prefix and the ten-digit
/// suffix leave this much room for the harness name.
const MAX_HARNESS_IN_ID: usize = 18;

fn new_session_id(harness: &str) -> String {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let slug: String = harness
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9' | '-') => c,
            _ => '-',
        })
        .take(MAX_HARNESS_IN_ID)
        .collect();
    format!("hs-{slug}-{:x}", nanos as u64 & 0xff_ffff_ffff)
}

/// Set by the MCP gateway (never by a caller) on calls that arrive on the
/// handoff-scoped key, so `herdr.handoff_cwd_allow` can bind them.
pub const HANDOFF_SCOPE_ARG: &str = "_hq_handoff_scope";

/// Characters a shell would expand inside the path the agent is started in.
const CWD_FORBIDDEN: [char; 4] = ['~', '$', '`', '\0'];

/// A launch needs an absolute project directory. `/` and anything shaped like a user's home are
/// refused, whichever host the session runs on, because the agent stops at its folder-trust
/// dialog there and nobody is watching the pane. `..` components and shell-expanding characters
/// are refused so the string that was checked is the string herdr receives.
pub fn require_cwd(cwd: Option<&str>) -> Result<PathBuf> {
    let cwd = cwd.map(str::trim).unwrap_or_default();
    if cwd.is_empty() {
        bail!("cwd is required: name the project directory to run the session in");
    }
    if cwd.contains(CWD_FORBIDDEN) {
        bail!("cwd '{cwd}' contains '~', '$', a backtick or a NUL; name the directory literally");
    }
    let path = PathBuf::from(cwd);
    if !path.is_absolute() {
        bail!("cwd '{cwd}' is not an absolute path");
    }
    if path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        bail!("cwd '{cwd}' contains '..'; name the directory without parent references");
    }
    let is_own_home = dirs::home_dir().is_some_and(|home| home == path);
    if is_home_shaped(&path) || is_own_home {
        bail!("cwd '{cwd}' is a home or root directory; name the project directory instead");
    }
    Ok(path)
}

/// Home directories of other machines HQ can start sessions on: `/Users/<name>` (macOS),
/// `/home/<name>` and `/root`, `/var/root`, and the HQ service user's `/opt/hq`. Also the
/// directories that only hold homes, and `/`. Components, not strings, so a trailing or doubled
/// slash cannot get past it, and case-insensitive because macOS volumes are.
fn is_home_shaped(path: &Path) -> bool {
    let parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(p) => p.to_str().map(str::to_ascii_lowercase),
            _ => None,
        })
        .collect();
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    matches!(
        parts.as_slice(),
        [] | ["users" | "home" | "root"] | ["users" | "home", _] | ["var", "root"] | ["opt", "hq"]
    )
}

/// Lowercase, collapse repeated slashes and drop trailing ones so `/Clients//Acme/` and
/// `/clients/acme` compare equal.
fn normalize_fragment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.trim().to_lowercase().chars() {
        if !(c == '/' && out.ends_with('/')) {
            out.push(c);
        }
    }
    out.trim_end_matches('/').to_string()
}

/// Refuse a working directory that contains any `deny` entry, compared
/// case-insensitively after `.` and `..` are resolved (a case-insensitive
/// filesystem would otherwise let `/Clients/X` past `/clients/x`). Blank
/// entries are ignored and entries are normalized (trailing and doubled
/// slashes). The path is checked as written, so a symlink into a denied
/// directory is not caught.
pub fn check_cwd_allowed(cwd: &str, deny: &[String]) -> Result<()> {
    let normalized = normalize_fragment(
        &crate::util::lexically_normalize(Path::new(cwd.trim())).to_string_lossy(),
    );
    let hit = deny
        .iter()
        .map(|d| (d.trim(), normalize_fragment(d)))
        .filter(|(_, n)| !n.is_empty())
        .find(|(_, n)| normalized.contains(n.as_str()));
    if let Some((entry, _)) = hit {
        bail!(
            "cwd '{cwd}' is refused by herdr.spawn_cwd_deny (matches '{entry}'); no session was started"
        );
    }
    Ok(())
}

/// With `allow` non-empty, the handoff key may only start sessions in or under one of its entries.
pub fn check_handoff_cwd(cwd: &Path, allow: &[String]) -> Result<()> {
    let entries: Vec<&str> = allow.iter().map(|a| a.trim()).filter(|a| !a.is_empty()).collect();
    if entries.is_empty() || entries.iter().any(|a| cwd.starts_with(a)) {
        return Ok(());
    }
    bail!(
        "cwd '{}' is outside herdr.handoff_cwd_allow, which limits the handoff key; no session was started",
        cwd.display()
    )
}

/// `require_cwd` plus the deny list (and, for the handoff key, the allow list)
/// of an already loaded config.
pub fn require_cwd_in(
    cwd: Option<&str>,
    herdr: &hq_core::config::HerdrConfig,
    handoff_scope: bool,
) -> Result<PathBuf> {
    let path = require_cwd(cwd)?;
    check_cwd_allowed(&path.to_string_lossy(), &herdr.spawn_cwd_deny)?;
    if handoff_scope {
        check_handoff_cwd(&path, &herdr.handoff_cwd_allow)?;
    }
    Ok(path)
}

/// `require_cwd_in` with the config read from disk. A config that cannot be
/// read fails the check rather than skipping it.
pub fn require_allowed_cwd(cwd: Option<&str>) -> Result<PathBuf> {
    let cfg = hq_core::config::HqConfig::load()
        .map_err(|e| anyhow::anyhow!("cannot read the config to check herdr.spawn_cwd_deny: {e}"))?;
    require_cwd_in(cwd, &cfg.herdr, false)
}

/// Whether a tool call arrived on the handoff-scoped key.
pub fn is_handoff_scope(args: &Value) -> bool {
    args.get(HANDOFF_SCOPE_ARG).and_then(Value::as_bool) == Some(true)
}

fn build_args(
    harness: &Harness,
    vault_path: &Path,
    session_id: &str,
    resume_token: Option<&str>,
    resuming: bool,
) -> Vec<String> {
    let spec = harness.spec;
    let base: Vec<String> = if resuming {
        match spec.resume {
            ResumeStrategy::Args(args) => {
                let needs_token = args.iter().any(|a| a.contains("{token}"));
                if needs_token && resume_token.is_none() {
                    // No token harvested: fall back to a fresh spawn.
                    harness.fresh_args()
                } else {
                    args.iter()
                        .map(|a| a.replace("{token}", resume_token.unwrap_or_default()))
                        .collect()
                }
            }
            ResumeStrategy::SessionDir | ResumeStrategy::None => harness.fresh_args(),
        }
    } else {
        harness.fresh_args()
    };
    let mut args = base;
    if spec.resume == ResumeStrategy::SessionDir {
        let dir = vault_path.join(SESSION_DIR_ROOT).join(session_id);
        let _ = std::fs::create_dir_all(&dir);
        args.push("--session-dir".into());
        args.push(dir.to_string_lossy().to_string());
    }
    args
}

/// Everything `launch_session` needs beyond the harness spec.
struct Launch<'a> {
    host: HerdrHost,
    session_id: &'a str,
    cwd: &'a str,
    label: &'a str,
    prompt: Option<&'a str>,
    resume_token: Option<&'a str>,
    resuming: bool,
    mission_id: Option<&'a str>,
    /// Web chat that launched the session and will watch it.
    watch: Option<NewWatch<'a>>,
    goal: GoalText<'a>,
}

fn workspace_label(harness: &str, label: &str) -> String {
    if label.is_empty() {
        format!("hq {harness}")
    } else {
        format!("hq {label}")
    }
}

/// Launches running now, by (database, host, agent name). The database is part of the key only
/// so several in-memory databases in one test process do not see each other's launches. A retry or resume of the same
/// agent while an earlier launch is still waiting would otherwise start a second
/// one under the same name.
static LAUNCHES_IN_FLIGHT: std::sync::Mutex<Vec<(usize, String, String)>> = std::sync::Mutex::new(Vec::new());

struct InFlight(usize, String, String);

impl InFlight {
    fn claim(db: &Arc<Database>, host: &str, agent: &str) -> Result<Self> {
        let mut live = LAUNCHES_IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        let key = (Arc::as_ptr(db) as usize, host.to_string(), agent.to_string());
        if live.contains(&key) {
            bail!("a launch of agent '{agent}' on host '{host}' is already in progress; wait for it to finish");
        }
        let owner = key.0;
        live.push(key);
        Ok(Self(owner, host.to_string(), agent.to_string()))
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut live = LAUNCHES_IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|(d, h, a)| !(d == &self.0 && h == &self.1 && a == &self.2));
    }
}

/// `Launch` without borrows, so a launch can run on a task the caller's future
/// does not own.
struct OwnedLaunch {
    host: HerdrHost,
    session_id: String,
    cwd: String,
    label: String,
    prompt: Option<String>,
    resume_token: Option<String>,
    resuming: bool,
    mission_id: Option<String>,
    watch: Option<(String, bool, bool)>,
    goal: (Option<String>, Option<String>),
}

impl OwnedLaunch {
    fn new(l: &Launch<'_>) -> Self {
        let own = |s: Option<&str>| s.map(str::to_string);
        Self {
            host: l.host.clone(),
            session_id: l.session_id.to_string(),
            cwd: l.cwd.to_string(),
            label: l.label.to_string(),
            prompt: own(l.prompt),
            resume_token: own(l.resume_token),
            resuming: l.resuming,
            mission_id: own(l.mission_id),
            watch: l.watch.map(|w| (w.thread.to_string(), w.drive, w.opted_out)),
            goal: (own(l.goal.goal), own(l.goal.done_criteria)),
        }
    }

    fn borrow(&self) -> Launch<'_> {
        Launch {
            host: self.host.clone(),
            session_id: &self.session_id,
            cwd: &self.cwd,
            label: &self.label,
            prompt: self.prompt.as_deref(),
            resume_token: self.resume_token.as_deref(),
            resuming: self.resuming,
            mission_id: self.mission_id.as_deref(),
            watch: self.watch.as_ref().map(|(thread, drive, opted_out)| NewWatch {
                thread,
                drive: *drive,
                opted_out: *opted_out,
            }),
            goal: GoalText {
                goal: self.goal.0.as_deref(),
                done_criteria: self.goal.1.as_deref(),
            },
        }
    }
}

/// Starts the agent, records it, gets it to a prompt (accepting the one dialog
/// its spec vouches for), then types the initial prompt.
///
/// The work runs on its own task. An MCP client that gives up mid-launch drops
/// this future, and a launch cut off between "workspace created" and "row
/// recorded" would leave a pane nothing tracks. On its own task the launch
/// always finishes, either recorded or cleaned up, whether or not anyone is
/// still waiting for the answer. The wait for the agent to come up is also
/// bounded (`HerdrHost::launch_bound`), so the caller is answered before its
/// transport times out.
async fn launch_session(
    vault_path: &Path,
    db: &Arc<Database>,
    harness: &Harness,
    l: Launch<'_>,
) -> Result<Value> {
    let (vault_path, db, harness, owned) =
        (vault_path.to_path_buf(), db.clone(), harness.clone(), OwnedLaunch::new(&l));
    let in_flight = InFlight::claim(&db, owned.host.name(), &owned.session_id)?;
    tokio::spawn(async move {
        let _in_flight = in_flight;
        run_launch(&vault_path, &db, &harness, owned.borrow()).await
    })
        .await
        .map_err(|e| anyhow::anyhow!("the launch task did not finish: {e}"))?
}

/// The launch itself. From the moment the workspace exists until its row is
/// recorded, any failure closes the workspace (and says so if it could not), so
/// nothing is left untracked. Once the row is written the session is real and
/// later failures leave it tracked.
async fn run_launch(
    vault_path: &Path,
    db: &Arc<Database>,
    harness: &Harness,
    l: Launch<'_>,
) -> Result<Value> {
    preflight::require_binary(&l.host, harness)?;
    let profile = harness.profile.as_ref();
    let request = LaunchRequest {
        name: l.session_id.to_string(),
        kind: harness.spec.kind.to_string(),
        cwd: l.cwd.to_string(),
        label: workspace_label(&harness.name, l.label),
        env: profile
            .map(|p| {
                p.env
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
            .into_iter()
            .chain([(SESSION_ENV.to_string(), l.session_id.to_string())])
            .collect(),
        args: build_args(
            harness,
            vault_path,
            l.session_id,
            l.resume_token,
            l.resuming,
        ),
        command: profile.and_then(|p| p.command.clone()),
        start_timeout: l.host.launch_bound(),
    };
    let began = std::time::Instant::now();
    let host = l.host.clone();
    let launched = blocking(move || host.launch(&request)).await??;
    let (host, h, name) = (l.host.clone(), harness.clone(), l.session_id.to_string());
    let launched =
        blocking(move || preflight::ensure_started(&host, &h, &name, launched, began)).await??;
    if let Err(e) = record_placement(db, harness, &l, &launched) {
        let (host, ws) = (l.host.clone(), launched.workspace_id.clone());
        let note = blocking(move || preflight::close_note(&host, &ws)).await?;
        return Err(e.context(note));
    }
    let event = if l.resuming {
        mission::Event::Resumed
    } else {
        mission::Event::Launched
    };
    let task = l.mission_id.and_then(|_| record_on_task(db, l.session_id, event));

    let host = l.host.clone();
    let name = l.session_id.to_string();
    let trust = harness.spec.trust_pattern;
    let launched_for_settle = launched.clone();
    let settled =
        blocking(move || settle_startup(&host, &name, trust, &launched_for_settle)).await??;

    let mut note = None;
    if let (Some(prompt), true) = (l.prompt.filter(|p| !p.trim().is_empty()), settled.ready) {
        let host = l.host.clone();
        let (name, prompt) = (l.session_id.to_string(), prompt.to_string());
        note = prompt_note(blocking(move || host.submit(&name, &prompt)).await??);
    }
    let task = match (task, settled.screen.is_some()) {
        (Some(_), true) => record_on_task(db, l.session_id, mission::Event::BlockedAtLaunch),
        (task, _) => task,
    };
    let mut report = launch_report(harness, &l, &launched, &settled, note);
    if let Some(task) = task {
        report["task"] = task;
    }
    Ok(report)
}

/// Record `event` on the session's linked task, as JSON for the caller's
/// report. The session already exists, so a failure here is reported, never
/// raised: the work is running whether or not its task heard about it.
fn record_on_task(db: &Arc<Database>, session_id: &str, event: mission::Event) -> Option<Value> {
    let result = db.with_conn(|c| {
        let Some(row) = registry::get(c, session_id)? else {
            return Ok(None);
        };
        mission::record(c, &row, event)
    });
    match result {
        Ok(link) => link.map(|l| json!(l)),
        Err(e) => {
            tracing::warn!(session = %session_id, error = %e, "harness-session: task update failed");
            Some(json!({ "error": format!("session is running but its task was not updated: {e}") }))
        }
    }
}

fn record_placement(
    db: &Arc<Database>,
    harness: &Harness,
    l: &Launch<'_>,
    launched: &Launched,
) -> Result<()> {
    let host = l.host.name().to_string();
    let placement_owned = (
        host,
        l.session_id.to_string(),
        launched.workspace_id.clone(),
        launched.pane_id.clone(),
    );
    let (harness, label, cwd) = (harness.name.clone(), l.label.to_string(), l.cwd.to_string());
    let (resuming, mission) = (l.resuming, l.mission_id.map(str::to_string));
    let watch = l.watch.map(|w| (w.thread.to_string(), w.drive, w.opted_out));
    let goal = (l.goal.goal.map(str::to_string), l.goal.done_criteria.map(str::to_string));
    db.with_conn(move |c| {
        let (host, name, ws, pane) = &placement_owned;
        let placement = Placement {
            host,
            agent_name: name,
            workspace_id: ws,
            pane_id: pane,
        };
        if resuming {
            return registry::relaunch(c, name, &placement);
        }
        registry::insert(
            c,
            &registry::NewSession {
                id: name,
                harness: &harness,
                label: &label,
                cwd: &cwd,
                mission_id: mission.as_deref(),
                placement,
            },
        )?;
        if goal.0.is_some() || goal.1.is_some() {
            registry::set_goal(c, name, goal.0.as_deref(), goal.1.as_deref(), registry::ACTOR_HQ)?;
        }
        if let Some((thread, drive, opted_out)) = &watch {
            start_watch(c, name, NewWatch { thread, drive: *drive, opted_out: *opted_out })?;
        }
        Ok(())
    })
}

struct Settled {
    ready: bool,
    status: Option<AgentStatus>,
    /// Screen text when the agent is stuck at a dialog.
    screen: Option<String>,
}

fn settle_startup(
    host: &HerdrHost,
    name: &str,
    trust_pattern: Option<&str>,
    launched: &Launched,
) -> Result<Settled, HerdrError> {
    let status = launched.agent.as_ref().map(|a| a.status);
    if launched.ready {
        return Ok(Settled {
            ready: true,
            status,
            screen: None,
        });
    }
    let screen = host.read(name, BLOCKED_SCREEN_LINES)?;
    let accepts_trust = trust_pattern.is_some_and(|p| screen.contains(p));
    if !accepts_trust {
        return Ok(Settled {
            ready: false,
            status,
            screen: Some(screen),
        });
    }
    host.send_keys(name, &["enter".to_string()])?;
    let until = [AgentStatus::Idle, AgentStatus::Done, AgentStatus::Blocked];
    let agent = host.wait(name, &until, TRUST_SETTLE_TIMEOUT)?;
    let ready = matches!(agent.status, AgentStatus::Idle | AgentStatus::Done);
    let screen = if ready {
        None
    } else {
        Some(host.read(name, BLOCKED_SCREEN_LINES)?)
    };
    Ok(Settled {
        ready,
        status: Some(agent.status),
        screen,
    })
}

pub(crate) fn prompt_note(outcome: PromptOutcome) -> Option<String> {
    match outcome {
        PromptOutcome::Resubmitted => Some(RESUBMITTED_NOTE.into()),
        PromptOutcome::Stalled(_) => Some(
            "Herdr saw no activity after the prompt; a fast agent can finish first. Read the output before resending.".into(),
        ),
        PromptOutcome::TimedOut(msg) => Some(format!("prompt wait timed out: {msg}")),
        PromptOutcome::Submitted | PromptOutcome::Settled(_) => None,
    }
}

const RESUBMITTED_NOTE: &str = "Herdr saw no activity after the prompt, so Enter was pressed once more. Read the output to confirm the agent started.";

fn launch_report(
    harness: &Harness,
    l: &Launch<'_>,
    launched: &Launched,
    settled: &Settled,
    note: Option<String>,
) -> Value {
    let mut report = json!({
        "session_id": l.session_id,
        "harness": harness.name,
        "host": l.host.name(),
        "agent_name": l.session_id,
        "workspace_id": launched.workspace_id,
        "pane_id": launched.pane_id,
        "cwd": l.cwd,
        "resumed": l.resuming,
        "status": "running",
        "agent_status": settled.status.map(AgentStatus::as_str),
    });
    if let Some(screen) = &settled.screen {
        report["blocked"] = json!({
            "screen": screen,
            "next": "The agent is waiting at a dialog and no prompt was typed. Read the screen, then answer with harness_session_send using `keys`.",
        });
    }
    if let Some(note) = note {
        report["note"] = json!(note);
    }
    report
}

/// Where and how a new session should start.
pub struct SpawnRequest<'a> {
    /// Herdr host name; `None` uses the configured default.
    pub host: Option<&'a str>,
    pub harness: &'a str,
    pub prompt: Option<&'a str>,
    pub cwd: &'a Path,
    pub label: &'a str,
    /// HQ task (id or display id) the session works on.
    pub mission_id: Option<&'a str>,
    /// Web chat that will watch the session.
    pub watch: Option<NewWatch<'a>>,
    pub goal: GoalText<'a>,
}

/// Spawn a fresh interactive session for a harness on the default host.
pub async fn spawn(
    vault_path: &Path,
    db: &Arc<Database>,
    harness: &str,
    prompt: Option<&str>,
    cwd: &Path,
    label: &str,
    mission_id: Option<&str>,
) -> Result<Value> {
    spawn_with(
        vault_path,
        db,
        SpawnRequest {
            host: None,
            harness,
            prompt,
            cwd,
            label,
            mission_id,
            watch: None,
            goal: GoalText::default(),
        },
    )
    .await
}

pub async fn spawn_with(
    vault_path: &Path,
    db: &Arc<Database>,
    req: SpawnRequest<'_>,
) -> Result<Value> {
    require_allowed_cwd(Some(&req.cwd.to_string_lossy()))?;
    let host = herdr::host(req.host)?;
    spawn_on(vault_path, db, host, req).await
}

/// `spawn_with` on an already resolved host, so a caller that must know the
/// host first (the handoff) and a test can supply it. The caller has already
/// vetted `req.cwd` against the config it loaded.
pub(crate) async fn spawn_on(
    vault_path: &Path,
    db: &Arc<Database>,
    host: HerdrHost,
    req: SpawnRequest<'_>,
) -> Result<Value> {
    let harness = resolve(req.harness)?;
    let mission_id = match req.mission_id {
        Some(task) => Some(db.with_conn(|c| mission::resolve_task(c, task))?),
        None => None,
    };
    let task_goal = match (&mission_id, req.goal.goal) {
        (Some(id), None) => db.with_conn(|c| Ok(task_goal(c, id)))?,
        _ => None,
    };
    let goal = GoalText { goal: req.goal.goal.or(task_goal.as_deref()), ..req.goal };
    let session_id = new_session_id(&harness.name);
    let cwd = req.cwd.to_string_lossy();
    let report = launch_session(
        vault_path,
        db,
        &harness,
        Launch {
            host,
            session_id: &session_id,
            cwd: &cwd,
            label: req.label,
            prompt: req.prompt,
            resume_token: None,
            resuming: false,
            mission_id: mission_id.as_deref(),
            watch: req.watch,
            goal,
        },
    )
    .await?;
    if req.watch.is_none() {
        return Ok(report);
    }
    Ok(with_watch_state(db, &session_id, report))
}

/// Tell the caller whether the chat now drives the session, since the default
/// can be off (untrusted turn, opt-out, or config) and the model must not guess.
fn with_watch_state(db: &Arc<Database>, session_id: &str, mut report: Value) -> Value {
    if let (Ok(row), Some(obj)) = (get_row(db, session_id), report.as_object_mut()) {
        obj.insert("watched_by_thread".into(), json!(row.owner_thread));
        obj.insert("drive".into(), json!(row.drive));
        obj.insert("mode".into(), json!(mode_name(row.drive)));
        let gaps = registry::goal_gaps(&row);
        if !row.drive && !gaps.is_empty() {
            obj.insert("drive_blocked_by".into(), json!(gaps));
            obj.insert("next".into(), json!(GOAL_NEXT));
        }
        if let (false, Some(reason)) = (row.drive, &row.drive_off_reason) {
            obj.insert("drive_off_reason".into(), json!(reason));
        }
    }
    report
}

const MODE_DRIVE: &str = "drive";
const MODE_OBSERVE: &str = "observe";

const GOAL_NEXT: &str = "HQ only observes this session. Ask the user for a specific goal and an observable definition of done, record them with harness_session_goal, then switch to drive with harness_session_mode.";

fn mode_name(drive: bool) -> &'static str {
    if drive { MODE_DRIVE } else { MODE_OBSERVE }
}

fn get_row(db:&Arc<Database>, session_id: &str) -> Result<HarnessSessionRow> {
    let id = session_id.to_string();
    db.with_conn(move |c| registry::get(c, &id))?
        .ok_or_else(|| anyhow::anyhow!("no harness session '{session_id}'"))
}

/// The row's host and its agent, or an error naming why they are unavailable.
fn locate(row: &HarnessSessionRow) -> Result<(HerdrHost, Option<AgentInfo>)> {
    let host = herdr::host(Some(&row.host))?;
    let agent = host.agent(&row.agent_name)?;
    Ok((host, agent))
}

fn require_alive(row: &HarnessSessionRow) -> Result<HerdrHost> {
    let (host, agent) = locate(row)?;
    if agent.is_none() {
        bail!(
            "session {} is not running (no agent '{}' on host '{}')",
            row.id,
            row.agent_name,
            row.host
        );
    }
    Ok(host)
}

/// Resume a prior session: same session dir / resume token, new agent.
/// Falls back to a fresh spawn when the harness has no resume support or no
/// token was harvested; the result says which happened.
pub async fn resume(
    vault_path: &Path,
    db: &Arc<Database>,
    session_id: &str,
    prompt: Option<&str>,
) -> Result<Value> {
    let row = get_row(db, session_id)?;
    require_allowed_cwd(Some(&row.cwd))?;
    let harness = resolve(&row.harness)?;
    let spec = harness.spec;
    let host = herdr::host(Some(&row.host))?;
    let probe_host = host.clone();
    let name = row.agent_name.clone();
    if blocking(move || probe_host.agent(&name)).await??.is_some() {
        bail!(
            "session {session_id} is still running on '{}'; use harness_session_send to talk to it",
            row.host
        );
    }
    let resumable = match spec.resume {
        ResumeStrategy::Args(args) => {
            !args.iter().any(|a| a.contains("{token}")) || row.resume_token.is_some()
        }
        ResumeStrategy::SessionDir => true,
        ResumeStrategy::None => false,
    };
    let mut value = launch_session(
        vault_path,
        db,
        &harness,
        Launch {
            host,
            session_id,
            cwd: &row.cwd,
            label: &row.label,
            prompt,
            resume_token: row.resume_token.as_deref(),
            resuming: true,
            mission_id: row.mission_id.as_deref(),
            watch: None,
            goal: GoalText::default(),
        },
    )
    .await?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("resume_supported".into(), Value::Bool(resumable));
        if !resumable {
            obj.insert(
                "note".into(),
                Value::String(format!(
                    "{} has no resume support wired; started fresh in the same cwd",
                    row.harness
                )),
            );
        }
    }
    screen_changed(session_id);
    Ok(value)
}

/// Live status for one session: registry row plus what Herdr says now.
pub fn status(db: &Arc<Database>, session_id: &str) -> Result<Value> {
    let row = get_row(db, session_id)?;
    let polled = poll_hosts(std::slice::from_ref(&row));
    Ok(session_view(&row, &liveness(&polled, &row)))
}

/// The row, its live state, and what HQ is doing with it: the goal and
/// definition of done travel inside `session`, `mode` says whether HQ drives
/// or only observes, and `drive_blocked_by` says what the gate still wants.
fn session_view(row: &HarnessSessionRow, live: &Liveness) -> Value {
    let mut view = live_view(row, live);
    if row.owner_thread.is_some() {
        view["mode"] = json!(mode_name(row.drive));
        let gaps = registry::goal_gaps(row);
        if !gaps.is_empty() {
            view["drive_blocked_by"] = json!(gaps);
        }
    }
    view
}

fn live_view(row: &HarnessSessionRow, live: &Liveness) -> Value {
    let running = row.status == registry::STATUS_RUNNING;
    match live {
        Liveness::Alive(agent) => json!({
            "session": row, "alive": true, "reachable": true,
            "agent_status": agent.status, "title": agent.title,
        }),
        Liveness::Gone => json!({ "session": row, "alive": false, "reachable": true }),
        Liveness::HostUnreachable(detail) if running => json!({
            "session": row, "alive": null, "reachable": false, "detail": detail,
        }),
        Liveness::HostUnreachable(_) => json!({ "session": row, "alive": false }),
    }
}

pub fn list(db: &Arc<Database>, status_filter: Option<String>) -> Result<Value> {
    let rows = db.with_conn(move |c| registry::list(c, status_filter.as_deref(), 50))?;
    Ok(json!({ "sessions": session_views(&rows) }))
}

/// Attach an existing session (running or not) to an HQ task. From a web
/// chat, that chat also starts watching it.
pub fn link(db: &Arc<Database>, session_id: &str, task: &str, watch: Option<NewWatch<'_>>) -> Result<Value> {
    let link = db.with_conn(|c| {
        let link = mission::link(c, session_id, task)?;
        let row = registry::get(c, session_id)?;
        if let Some(row) = row.filter(|r| r.goal.is_none())
            && let Some(goal) = row.mission_id.as_deref().and_then(|m| task_goal(c, m))
        {
            registry::set_goal(c, session_id, Some(&goal), None, registry::ACTOR_HQ)?;
        }
        if let Some(w) = watch {
            start_watch(c, session_id, w)?;
        }
        Ok(link)
    })?;
    let report = json!({ "session_id": session_id, "task": link });
    Ok(if watch.is_some() { with_watch_state(db, session_id, report) } else { report })
}

/// Have a web chat watch a session. A tool can start a new watch driving (the
/// user's default) or stop driving, but never turn an existing watch's Drive on.
pub fn watch(db: &Arc<Database>, session_id: &str, w: NewWatch<'_>) -> Result<Value> {
    let row = db.with_conn(|c| {
        if !start_watch(c, session_id, w)? {
            bail!("no harness session '{session_id}'");
        }
        registry::get(c, session_id)
    })?;
    Ok(json!({ "session": row }))
}

/// Record what a session is for and how anyone could tell it is done. A change
/// that leaves a driven session failing the drive gate switches HQ to observing.
pub fn set_goal(db: &Arc<Database>, session_id: &str, text: GoalText<'_>, actor: &str) -> Result<Value> {
    if text.goal.is_none() && text.done_criteria.is_none() {
        bail!("pass `goal`, `done_criteria`, or both");
    }
    let update = db
        .with_conn(|c| registry::set_goal(c, session_id, text.goal, text.done_criteria, actor))?
        .ok_or_else(|| anyhow::anyhow!("no harness session '{session_id}'"))?;
    let row = get_row(db, session_id)?;
    let mut report = json!({
        "session_id": session_id,
        "goal": row.goal,
        "done_criteria": row.done_criteria,
        "mode": mode_name(row.drive),
    });
    if update.drive_stopped {
        report["drive_stopped"] = json!("the new text no longer passes the drive gate, so HQ is observing only");
    }
    if !update.gaps.is_empty() {
        report["drive_blocked_by"] = json!(update.gaps);
    }
    Ok(report)
}

/// Who asks for a drive mode, and from where.
#[derive(Debug, Clone, Copy)]
pub struct ModeRequest<'a> {
    /// The web chat asking; it must be the one watching the session.
    pub thread: &'a str,
    pub drive: bool,
    /// The turn has read untrusted content, so it may only give up Drive.
    pub untrusted: bool,
    pub actor: &'a str,
}

/// Switch HQ between driving a session and observing it. Only HQ's steering
/// changes: the agent keeps running exactly as it was, and stopping it stays a
/// separate `harness_session_stop`.
pub fn set_mode(db: &Arc<Database>, session_id: &str, req: ModeRequest<'_>) -> Result<Value> {
    let row = get_row(db, session_id)?;
    let polled = poll_hosts(std::slice::from_ref(&row));
    set_mode_with(db, &row, &liveness(&polled, &row), req)
}

fn set_mode_with(db: &Arc<Database>, row: &HarnessSessionRow, live: &Liveness, req: ModeRequest<'_>) -> Result<Value> {
    if row.owner_thread.as_deref() != Some(req.thread) {
        bail!("session {} is not watched by this chat; use harness_session_watch first", row.id);
    }
    if req.drive {
        require_drivable(row, live, req)?;
    }
    let cap = Some(herdr_config().driven_session_cap());
    let change = db.with_conn(|c| registry::request_drive_capped(c, &row.id, req.drive, req.actor, cap))?;
    let mut report = json!({
        "session_id": row.id,
        "agent": agent_state(live),
        "agent_untouched": "only HQ's steering changed; the agent was not paused or stopped",
    });
    match change {
        registry::DriveChange::Changed(on) => {
            report["mode"] = json!(mode_name(on));
        }
        registry::DriveChange::Refused(gaps) => {
            report["mode"] = json!(MODE_OBSERVE);
            report["refused"] = json!("drive was not enabled; HQ only observes");
            report["drive_blocked_by"] = json!(gaps);
            report["next"] = json!(GOAL_NEXT);
        }
        registry::DriveChange::NotWatched => bail!("session {} is not watched by any chat", row.id),
    }
    Ok(report)
}

/// Driving needs a session HQ can confirm is alive and a turn it may trust.
fn require_drivable(row: &HarnessSessionRow, live: &Liveness, req: ModeRequest<'_>) -> Result<()> {
    if req.untrusted {
        bail!("this turn read untrusted content, so it cannot turn Drive on; the user can, with the switch in the Watching panel");
    }
    match live {
        Liveness::Alive(_) => Ok(()),
        Liveness::Gone => bail!(
            "session {} has ended (no agent '{}' on host '{}'); HQ still only observes. Resume it with harness_session_resume first",
            row.id, row.agent_name, row.host
        ),
        Liveness::HostUnreachable(detail) => bail!(
            "host '{}' is unreachable ({detail}), so HQ cannot confirm the session is alive; Drive was not enabled",
            row.host
        ),
    }
}

fn agent_state(live: &Liveness) -> Value {
    match live {
        Liveness::Alive(agent) => json!({ "state": "running", "status": agent.status }),
        Liveness::Gone => json!({ "state": "ended" }),
        Liveness::HostUnreachable(detail) => json!({ "state": "unknown", "detail": detail }),
    }
}

/// Bring an agent Herdr already runs, one HQ did not launch in this chat (or
/// at all), under this chat's watch. It starts observation-only: Drive comes
/// later, through the goal and drive gate.
pub fn attach(db: &Arc<Database>, host: &HerdrHost, target: &str, thread: &str) -> Result<Value> {
    let agent = host
        .agent(target)
        .map_err(|e| anyhow::anyhow!("cannot reach host '{}' to attach: {e}", host.name()))?
        .ok_or_else(|| anyhow::anyhow!("no agent '{target}' on host '{}'; herdr_agents lists what is there", host.name()))?;
    let Some(name) = agent.name.clone() else {
        bail!("that agent has no name in Herdr, so HQ cannot track it; name it there first");
    };
    let Some(spec) = SPECS.iter().find(|s| s.kind == agent.kind) else {
        bail!("no supported harness runs Herdr agent kind '{}'", agent.kind);
    };
    let host_name = host.name().to_string();
    let known = db.with_conn(|c| registry::list(c, Some(registry::STATUS_RUNNING), 200))?;
    let existing = known.into_iter().find(|r| r.host == host_name && r.agent_name == name);
    let already = existing.is_some();
    let id = existing.map_or_else(|| new_session_id(spec.harness), |r| r.id);
    db.with_conn(|c| {
        if !already {
            let placement = Placement { host: &host_name, agent_name: &name, workspace_id: &agent.workspace_id, pane_id: &agent.pane_id };
            let title = agent.title.clone().unwrap_or_default();
            let new = registry::NewSession { id: &id, harness: spec.harness, label: &title, cwd: &agent.cwd, mission_id: None, placement };
            registry::insert(c, &new)?;
        }
        start_watch(c, &id, NewWatch { thread, drive: false, opted_out: false })?;
        registry::record_event(c, &id, registry::EVENT_ATTACHED, registry::ACTOR_HQ, Some(&format!("attached to {name} on {host_name}")))
    })?;
    let report = json!({ "session_id": id, "attached": true, "already_tracked": already, "agent_status": agent.status });
    Ok(with_watch_state(db, &id, report))
}

/// Which sessions a global listing wants. Empty fields match everything.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub status: Option<String>,
    /// A task id or display id; when set the rows are that task's sessions.
    pub task: Option<String>,
    pub host: Option<String>,
}

/// Most rows one global listing returns, newest first.
const LIST_LIMIT: usize = 200;

/// Registry rows matching `filter`, each with its live state when the row is
/// marked running (one poll per host). `None` means the row is not running, so
/// no host was asked.
pub fn list_live(db: &Arc<Database>, filter: &ListFilter) -> Result<Vec<(HarnessSessionRow, Option<Liveness>)>> {
    let mut rows = db.with_conn(|c| match filter.task.as_deref() {
        Some(task) => {
            let found = hq_db::tasks::get_task(c, task)?.ok_or_else(|| anyhow::anyhow!("no task '{task}'"))?;
            registry::list_for_mission(c, &found.id)
        }
        None => registry::list(c, filter.status.as_deref(), LIST_LIMIT),
    })?;
    rows.retain(|r| {
        filter.host.as_deref().is_none_or(|h| r.host == h)
            && filter.status.as_deref().is_none_or(|s| r.status == s)
    });
    let running: Vec<HarnessSessionRow> =
        rows.iter().filter(|r| r.status == registry::STATUS_RUNNING).cloned().collect();
    let polled = poll_hosts(&running);
    Ok(rows
        .into_iter()
        .map(|row| {
            let live = (row.status == registry::STATUS_RUNNING).then(|| liveness(&polled, &row));
            (row, live)
        })
        .collect())
}

/// The live half of a session's view as flat fields (`alive`, `reachable`,
/// `agent_status`, `title`, `detail`), for callers that build their own row JSON.
pub fn live_fields(row: &HarnessSessionRow, live: Option<&Liveness>) -> Value {
    let mut view = match live {
        Some(live) => live_view(row, live),
        None => json!({ "alive": false }),
    };
    if let Some(obj) = view.as_object_mut() {
        obj.remove("session");
    }
    view
}

/// Every session launched for one task (id or display id), with live status
/// for the running ones.
pub fn list_for_task(db: &Arc<Database>, task: &str) -> Result<Value> {
    let (task_id, rows) = db.with_conn(|c| {
        let found = hq_db::tasks::get_task(c, task)?
            .ok_or_else(|| anyhow::anyhow!("no task '{task}'"))?;
        let rows = registry::list_for_mission(c, &found.id)?;
        Ok((found.display_id, rows))
    })?;
    Ok(json!({ "task": task_id, "sessions": session_views(&rows) }))
}

fn session_views(rows: &[HarnessSessionRow]) -> Vec<Value> {
    let running: Vec<HarnessSessionRow> = rows
        .iter()
        .filter(|r| r.status == registry::STATUS_RUNNING)
        .cloned()
        .collect();
    let polled = poll_hosts(&running);
    rows.iter()
        .map(|row| {
            if row.status == registry::STATUS_RUNNING {
                session_view(row, &liveness(&polled, row))
            } else {
                json!({ "session": row, "alive": false })
            }
        })
        .collect()
}

/// Recent output: read live while the agent runs, else the last snapshot the
/// supervisor stored.
pub fn tail_log(db: &Arc<Database>, session_id: &str, lines: usize) -> Result<Value> {
    tail_log_with(db, session_id, lines, |row| {
        herdr::host(Some(&row.host))
            .ok()
            .and_then(|host| host.read_sourced(&row.agent_name, lines).ok())
    })
}

/// `read` is called only for a row the registry says is running, so a name
/// that no longer belongs to an agent is never read as live output. It is one
/// herdr call, not a lookup then a read: on a remote host each call is a full
/// ssh round trip, and a read of a gone agent fails the same way.
fn tail_log_with(
    db: &Arc<Database>,
    session_id: &str,
    lines: usize,
    read: impl FnOnce(&HarnessSessionRow) -> Option<(String, &'static str)>,
) -> Result<Value> {
    let row = get_row(db, session_id)?;
    let live = (row.status == registry::STATUS_RUNNING)
        .then(|| read(&row))
        .flatten();
    let (source, herdr_source, text) = match live {
        Some((text, herdr_source)) => ("live", Some(herdr_source), text),
        None => {
            let id = session_id.to_string();
            let snap = db.with_conn(move |c| registry::last_snapshot(c, &id))?;
            ("snapshot", None, snap.unwrap_or_default())
        }
    };
    let all: Vec<&str> = text.lines().collect();
    let tail = &all[all.len().saturating_sub(lines)..];
    Ok(json!({ "session_id": session_id, "source": source, "herdr_source": herdr_source, "lines": tail }))
}

static SCREEN_READS: std::sync::LazyLock<coalesce::Coalescer> =
    std::sync::LazyLock::new(coalesce::Coalescer::new);

/// `tail_log` for pollers: concurrent calls for one session share a single
/// read, and a live result is reused for about a second. A snapshot fallback
/// is never kept, so a host that comes back is noticed on the next poll.
pub fn tail_log_shared(db: &Arc<Database>, session_id: &str, lines: usize) -> Result<Value> {
    SCREEN_READS.get(
        session_id,
        lines,
        || tail_log(db, session_id, lines),
        |screen| screen["source"] == "live",
    )
}

/// The session's screen just changed (send, stop, resume): drop any cached read.
fn screen_changed(session_id: &str) {
    SCREEN_READS.forget(session_id);
}

/// Steer a running session: submit `text` as a prompt.
pub fn send(
    db: &Arc<Database>,
    session_id: &str,
    text: &str,
    chat: Option<&WatchingChat>,
) -> Result<Value> {
    let result = metered(db, session_id, chat, SendKind::Text, || send_text(db, session_id, text));
    screen_changed(session_id);
    result
}

fn send_text(db: &Arc<Database>, session_id: &str, text: &str) -> Result<Value> {
    let row = get_row(db, session_id)?;
    let host = require_alive(&row)?;
    let outcome = host.submit(&row.agent_name, text);
    match outcome {
        Err(HerdrError::Api { code, message }) if code == "agent_blocked" => bail!(
            "session {session_id} is waiting at a dialog and the text was not sent ({message}). Read its output, then answer with keys."
        ),
        Err(e) => Err(e.into()),
        Ok(outcome) => {
            let mut report = json!({ "session_id": session_id, "sent": text });
            if let Some(note) = prompt_note(outcome) {
                report["note"] = json!(note);
            }
            if let Some(task) = record_on_task(db, session_id, mission::Event::Steered) {
                report["task"] = task;
            }
            Ok(report)
        }
    }
}

/// Press logical keys (`enter`, `esc`, `down`, `ctrl+c`), for answering dialogs.
pub fn send_keys(
    db: &Arc<Database>,
    session_id: &str,
    keys: &[String],
    chat: Option<&WatchingChat>,
) -> Result<Value> {
    let sent = metered(db, session_id, chat, SendKind::Keys, || {
        let row = get_row(db, session_id)?;
        let host = require_alive(&row)?;
        host.send_keys(&row.agent_name, keys)?;
        Ok(())
    });
    screen_changed(session_id);
    sent?;
    Ok(json!({ "session_id": session_id, "keys": keys }))
}

/// Block until the session settles (`idle`, `done` or `blocked` by default).
pub fn wait(
    db: &Arc<Database>,
    session_id: &str,
    until: &[AgentStatus],
    timeout: Duration,
) -> Result<Value> {
    let row = get_row(db, session_id)?;
    let host = require_alive(&row)?;
    let until = if until.is_empty() {
        &[AgentStatus::Idle, AgentStatus::Done, AgentStatus::Blocked][..]
    } else {
        until
    };
    match host.wait(&row.agent_name, until, timeout.min(MAX_WAIT)) {
        Ok(agent) => {
            Ok(json!({ "session_id": session_id, "settled": true, "agent_status": agent.status }))
        }
        Err(HerdrError::Api { code, .. }) if code == "timeout" => Ok(
            json!({ "session_id": session_id, "settled": false, "note": "still working; call again or read the output" }),
        ),
        Err(e) => Err(e.into()),
    }
}

pub fn stop(db: &Arc<Database>, session_id: &str) -> Result<Value> {
    let row = get_row(db, session_id)?;
    let (host, agent) = locate(&row)?;
    let workspace = row
        .workspace_id
        .clone()
        .or_else(|| agent.map(|a| a.workspace_id));
    if let Some(ws) = workspace {
        match host.close_workspace(&ws) {
            Err(e) if e.code() != Some("not_found") => return Err(e.into()),
            _ => {}
        }
    }
    let id = session_id.to_string();
    db.with_conn(move |c| registry::set_status(c, &id, registry::STATUS_STOPPED))?;
    screen_changed(session_id);
    let mut report = json!({ "session_id": session_id, "status": "stopped" });
    if let Some(task) = record_on_task(db, session_id, mission::Event::Stopped) {
        report["task"] = task;
    }
    Ok(report)
}

/// Extract a harness's resume token from session output: the capture group of
/// the *last* match of `pattern`, since a screen can carry several stale
/// tokens from earlier runs and only the most recent is still valid.
fn extract_resume_token(pattern: &str, content: &str) -> Result<Option<String>> {
    let re = regex::Regex::new(pattern)?;
    Ok(re
        .captures_iter(content)
        .last()
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string()))
}

/// Look for the harness's resume token in `screen` (what the supervisor just
/// read) and store it when it changed.
pub fn harvest_resume_token(
    db: &Arc<Database>,
    session_id: &str,
    screen: &str,
) -> Result<Option<String>> {
    let row = get_row(db, session_id)?;
    let Some(pattern) = resolve(&row.harness)
        .ok()
        .and_then(|h| h.spec.token_pattern)
    else {
        return Ok(None);
    };
    let token = extract_resume_token(pattern, screen)?;
    if let Some(t) = &token
        && row.resume_token.as_deref() != Some(t.as_str()) {
            let (id, t) = (session_id.to_string(), t.clone());
            db.with_conn(move |c| registry::set_resume_token(c, &id, &t))?;
        }
    Ok(token)
}

#[cfg(test)]
mod tests;
