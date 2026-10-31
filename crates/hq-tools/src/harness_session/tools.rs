//! MCP tools over the harness session manager.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

use super::WatchingChat;
use crate::registry::HqTool;
use crate::util::{arg_str, arg_str_list};
use hq_db::harness_sessions_registry as registry;

/// Where a tool-started session runs. An explicit folder that exists is used as given. A blank one,
/// or one the host does not have, falls back to a folder in that machine's HQ folder, which the
/// host creates; the deny and allow lists then judge the folder actually used.
async fn choose_start_dir(
    host: &crate::agent_host::Host,
    requested: Option<&str>,
    cfg: &hq_core::config::HqConfig,
    host_name: &str,
    bound: super::CwdBound,
    names: (&str, &str),
    asker: &FolderAsker<'_>,
) -> Result<super::StartDir> {
    let (label, harness) = names;
    let granted;
    let agent_host = if bounded_by_folders(bound, &cfg.agent_host) {
        let mut folders = if bound == super::CwdBound::Connector {
            super::connector_folders(&cfg.agent_host, host_hq_folder(host).await.as_deref())
        } else {
            cfg.agent_host.clone()
        };
        let grants = super::folder_request::granted_folders(
            asker.db,
            host.name(),
            &asker.requester,
            requested,
        )?;
        folders.handoff_cwd_allow.extend(grants);
        granted = folders;
        &granted
    } else {
        &cfg.agent_host
    };
    // With no folder asked for, refused before the fallback folder is made for it.
    if bound == super::CwdBound::Connector && requested.is_none() {
        super::connector_may_spawn(&agent_host.handoff_cwd_allow)?;
    }
    let vault_guard = |p: &std::path::Path| {
        super::check_cwd_outside_vault(p, host_name, &agent_host.default_host, &cfg.vault_path)
    };
    let checked = requested
        .map(|r| super::require_cwd_in(Some(r), agent_host, bound))
        .transpose();
    let wanted = match checked {
        Ok(wanted) => wanted,
        Err(e) => {
            let Some(outside) = e.downcast_ref::<super::OutsideAllowedFolders>() else {
                return Err(e);
            };
            // A folder that could never be granted is refused, not asked for.
            vault_guard(&outside.cwd)?;
            let folder = outside.cwd.clone();
            Some(await_folder_approval(host, &folder, &cfg.agent_host, bound, asker).await?)
        }
    };
    // A folder that is not allowed is refused whether or not it exists, never quietly swapped.
    if let Some(path) = &wanted {
        vault_guard(path)?;
    }
    if let Some(path) = wanted
        .as_ref()
        .filter(|p| host.dir_exists(p) != Some(false))
    {
        return Ok(super::StartDir {
            path: path.clone(),
            note: None,
        });
    }
    let (h, w, l, k) = (host.clone(), wanted, label.to_string(), harness.to_string());
    let fallback =
        tokio::task::spawn_blocking(move || super::workspace_start_dir(&*h, w.as_deref(), &l, &k))
            .await??;
    super::require_cwd_in(Some(&fallback.path.to_string_lossy()), agent_host, bound)?;
    vault_guard(&fallback.path)?;
    Ok(fallback)
}

/// Asks the owner for a folder the caller may not use yet, and waits a while for the answer, so
/// a quick approval starts the session in this same call. Until they approve, the call fails with
/// a block that says nothing started; a decline fails at once.
async fn await_folder_approval(
    host: &crate::agent_host::Host,
    folder: &std::path::Path,
    agent_host: &hq_core::config::AgentHostConfig,
    bound: super::CwdBound,
    asker: &FolderAsker<'_>,
) -> Result<std::path::PathBuf> {
    use super::folder_request::{self as requests, Answer, AwaitingApproval};
    let host_name = host.name();
    let asked = requests::ask_owner(asker.db, host_name, &asker.requester, folder);
    if asked.downcast_ref::<AwaitingApproval>().is_none() {
        // Declined, too many waiting, or a folder that cannot be shown to the owner.
        return Err(asked);
    }
    let budget = requests::answer_wait(agent_host);
    let started = std::time::Instant::now();
    match requests::wait_for_answer(asker.db, host_name, &asker.requester, folder, budget).await {
        Answer::Approved => {
            let shown = folder.to_string_lossy().into_owned();
            let mut folders = agent_host.clone();
            if bound == super::CwdBound::Connector {
                folders =
                    super::connector_folders(agent_host, host_hq_folder(host).await.as_deref());
            }
            folders.handoff_cwd_allow.extend(requests::granted_folders(
                asker.db,
                host_name,
                &asker.requester,
                Some(&shown),
            )?);
            super::require_cwd_in(Some(&shown), &folders, bound)
        }
        Answer::Declined => Err(requests::ask_owner(
            asker.db,
            host_name,
            &asker.requester,
            folder,
        )),
        Answer::Waiting => Err(requests::still_waiting(
            folder,
            host_name,
            started.elapsed(),
        )),
    }
}

/// What a refused folder needs to ask the owner for it.
struct FolderAsker<'a> {
    db: &'a hq_db::Database,
    requester: String,
}

impl<'a> FolderAsker<'a> {
    fn of(db: &'a hq_db::Database, args: &Value) -> Self {
        let requester = super::connector_label(args)
            .map(str::to_string)
            .unwrap_or_else(|| super::folder_request::HANDOFF_REQUESTER.to_string());
        Self { db, requester }
    }
}

/// Whether a caller is held to a folder list: a connector always, the handoff key only while
/// `handoff_cwd_allow` lists something (empty leaves that key unrestricted).
fn bounded_by_folders(
    bound: super::CwdBound,
    agent_host: &hq_core::config::AgentHostConfig,
) -> bool {
    match bound {
        super::CwdBound::Any => false,
        super::CwdBound::Connector => true,
        super::CwdBound::Handoff => agent_host
            .handoff_cwd_allow
            .iter()
            .any(|a| !a.trim().is_empty()),
    }
}

/// The host's own HQ folder as that machine resolves it, or `None` for a host without one.
async fn host_hq_folder(host: &crate::agent_host::Host) -> Option<String> {
    let h = host.clone();
    let workspace = tokio::task::spawn_blocking(move || h.workspace())
        .await
        .ok()?
        .ok()?;
    workspace["root"].as_str().map(str::to_string)
}

pub struct HarnessSessionSpawnTool {
    vault_path: PathBuf,
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    pub(crate) family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

#[async_trait]
impl HqTool for HarnessSessionSpawnTool {
    fn name(&self) -> &str {
        "harness_session_spawn"
    }
    fn description(&self) -> &str {
        "Spawn a long-lived interactive session for an agent in the host, on this machine or a remote host such as the user's laptop (see host_list). Two categories of agent run there. Coding agents (claude-code, cursor, opencode, pi, kimi, codex, qwen, antigravity, github-copilot) take code and repository changes. General-purpose agents (hermes, openclaw) are full assistants with their own tools and memory: they take `purpose` monitor (watch other sessions and report), research (parallel reading and searching), review (peer review of another agent's work) or general, and they refuse `purpose` code, so never send them a coding task. A monitor or reviewer linked to a task can be given sessions to `observe`: their finished turns, blocks and exits are relayed to it. Returns a session_id for status/logs/wait/send/stop/resume. If the agent stops at a dialog the result carries the screen and no prompt is typed. A configured profile name (agent_host.harness_profiles, such as a named Claude account) is accepted as the harness and reported back as `harness`. Pass task_id to make an HQ task the durable record of the work: the supervisor then comments on it and moves its status as the session starts, finishes a turn, blocks or exits (never to complete: the owner does, or HQ's task verification when it is on). A monitor or reviewer comments on its task but never moves it. Aliases: agy = antigravity."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "harness": { "type": "string", "description": format!("One of: {} (built-in harnesses plus any agent_host.harness_profiles). Coding agents: {}. General-purpose agents: {}.", super::spec::known_harnesses().join(", "), super::spec::names_in(super::HarnessCategory::Coding).join(", "), super::spec::names_in(super::HarnessCategory::GeneralPurpose).join(", ")) },
                "purpose": { "type": "string", "enum": ["code", "monitor", "research", "review", "general"], "description": "What the session is for. Default: code for a coding agent, general for a general-purpose agent. A general-purpose agent refuses code. monitor and review sessions never move their task's status." },
                "model": { "type": "string", "description": "Model to start the agent on, passed with its CLI's --model flag (for example sonnet or gpt-5.5 for claude-code, cursor or codex). Omit for the agent's own default. Refused, before anything starts, for an agent with no model flag HQ knows. The result's `model` reports what was passed." },
                "worktree": { "type": "object", "properties": { "branch": { "type": "string" }, "from": { "type": "string" }, "detached": { "type": "string" } }, "description": "Start in a git worktree of its own, under the host's HQ folder, so it never shares a working tree with another agent: {\"branch\": \"<name>\"} for a writer (made from the folder's current commit if it does not exist; refused while another worktree has it checked out), {\"detached\": \"<commit or ref>\"} for a reviewer. {\"branch\": \"<name>\", \"from\": \"<ref>\"} starts a new branch at that ref instead (ignored when the branch exists). A new worktree gets the gitignored files the repository's .worktreeinclude names (such as .env, node_modules), copied as copy-on-write clones, listed in the reply's worktree.copied. Removed at stop or exit when nothing is uncommitted; the branch keeps every commit." },
                "owns": { "type": "array", "items": { "type": "string" }, "description": "Paths, relative to cwd, this session writes to. A coding session in a folder another coding session already writes in is refused unless both own paths that do not overlap (no owns means the whole folder). Use a worktree instead when they would overlap." },
                "policy": { "type": "string", "description": "The command policy it runs under: builder or reviewer, one the owner configured under agent_host.command_policies, or none (every dialog goes to the owner). Default: builder for a coding session, reviewer for a review. A policy answers the dialogs it decides; see docs/AGENT_SESSIONS.md." },
                "team": { "type": "string", "description": "A root session id: the new session joins that session's team as a worker under it, with the team's roster, messages between members, stop-with-root and limits. For the owner, a web chat and a claude.ai connection (on a team it started); an agent inside a session uses agent_delegate. A reviewer joining a team observes its running coding sessions unless observe names others." },
                "observe": { "type": "array", "items": { "type": "string" }, "description": "Session ids a monitor or reviewer watches (at most 10; needs task_id). When one finishes a turn, blocks or exits, this session gets a message on its task thread, typed in when it is next idle." },
                "prompt": { "type": "string", "description": "Initial prompt typed into the session after it starts" },
                "host": { "type": "string", "description": "host to run on, from host_list (default: the configured default host)" },
                "cwd": { "type": "string", "description": "Project directory on that host. Optional: when omitted, or when the folder does not exist there, the session starts in a folder inside that machine's own HQ folder (Documents/HQ), created if needed. A home or root directory is refused. A claude.ai connector (and the handoff key while agent_host.handoff_cwd_allow lists anything) may use only the host's HQ folder, that list and folders the owner approved. Asking for any other folder sends the owner an approval request and this call waits up to 30 seconds for their answer: it starts the session if they approve, and otherwise fails with BLOCKED and starts nothing. Tell the user you are waiting, then call again with the same arguments. Never start elsewhere and cd into it." },
                "label": { "type": "string", "description": "Short human label, e.g. 'auth-refactor'", "default": "" },
                "task_id": { "type": "string", "description": "HQ task (id or display id such as FR-053) this session works on. Must exist and not be complete. Its title and description become the goal when `goal` is omitted." },
                "goal": { "type": "string", "description": "What the session should accomplish. HQ drives only a session with a specific goal and definition of done; without them it observes." },
                "done_criteria": { "type": "string", "description": "Observable conditions that show the goal is met, e.g. a named test passing." },
                "drive": { "type": "boolean", "description": "Web chat only. Omit to follow the default (a session no chat watched starts with Drive on, when its goal and definition of done pass the drive gate); false starts the watch with Drive off, or stops driving a session this chat already drives." },
                "from_session": { "type": "string", "description": "Set this to your own HQ session id (the HQ_SESSION_ID environment variable) when you run inside a session HQ launched. The new session then joins your team as your worker: it gets a sub-task under your task, runs in your folder and host, reports back to you when it finishes, stops when you stop, and counts against the team's limits. cwd, host and task_id are then ignored." }
            },
            "required": ["harness"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        if let Some(from) = super::delegating_session(&args) {
            // Only an agent inside a session delegates; a chat or a guest starting
            // sessions keeps its own checks and caps below.
            if self.chat.is_some() || self.family_guest.is_some() {
                anyhow::bail!(
                    "from_session is for an agent running inside an HQ session; a chat starts sessions without it"
                );
            }
            return self.delegate_claimed(from, &args).await;
        }
        let harness = arg_str(&args, "harness");
        if let Some(guest) = &self.family_guest {
            let canonical = |h: &str| {
                if h == "antigravity" {
                    "agy".to_string()
                } else {
                    h.to_string()
                }
            };
            let allowed = &guest.allowed_harnesses;
            if !allowed.is_empty() && !allowed.iter().any(|a| canonical(a) == canonical(&harness)) {
                anyhow::bail!(
                    "{} only has access to these coding-agent harnesses through me: {}. '{harness}' is not one of them, so it would need a separate request. Tell {} plainly which harnesses are available.",
                    guest.name,
                    allowed.join(", "),
                    guest.name
                );
            }
        }
        let prompt = arg_str(&args, "prompt");
        let label = arg_str(&args, "label");
        let host = arg_str(&args, "host");
        let task_id = arg_str(&args, "task_id");
        let (goal, done) = (arg_str(&args, "goal"), arg_str(&args, "done_criteria"));
        let purpose = super::Purpose::parse_opt(args.get("purpose").and_then(Value::as_str))?;
        let mut observe = arg_str_list(&args, "observe");
        // Joining a remote client's team: checked and held so two starts cannot both pass
        // the team's limits.
        let _joining = match super::team::parse(&args) {
            Some(_) => Some(crate::a2a::DELEGATION_LOCK.lock().await),
            None => None,
        };
        let team = match super::team::parse(&args) {
            Some(root) => Some(super::team::join(&self.db, &args, &root)?),
            None => None,
        };
        // A reviewer in a team watches the team's writers unless it names its own.
        if let Some((root, _)) = &team
            && purpose == Some(super::Purpose::Review)
            && observe.is_empty()
            && !task_id.trim().is_empty()
        {
            observe = super::team::writers(&self.db, root);
        }
        let worktree = super::checkout::parse_worktree(&args)?;
        let owns = super::checkout::parse_owns(&args)?;
        let policy = super::policy::parse(&args);
        let model = arg_str(&args, "model");
        let model = Some(model.trim()).filter(|m| !m.is_empty());
        // Before a start folder is chosen or made, so a refused model leaves nothing behind.
        super::spec::resolve(&harness)?.with_model(model)?;
        let cfg = hq_core::config::HqConfig::load()?;
        if self.chat.as_ref().is_some_and(|c| c.driver_turn) {
            anyhow::bail!("a driver turn cannot start sessions; tell the user what you need");
        }
        let origin = super::call_origin(self.chat.as_ref(), &args);
        super::check_origin_cap(&self.db, origin)?;
        let host_handle = crate::agent_host::host(Some(host.as_str()).filter(|h| !h.is_empty()))?;
        let requested = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|c| !c.is_empty());
        let bound = super::CwdBound::of(&args);
        let asker = FolderAsker::of(&self.db, &args);
        let start = choose_start_dir(
            &host_handle,
            requested,
            &cfg,
            &host,
            bound,
            (&label, &harness),
            &asker,
        )
        .await?;
        macro_rules! launch {
            ($cwd:expr) => {
                super::SpawnRequest {
                    host: Some(host.as_str()).filter(|h| !h.is_empty()),
                    harness: &harness,
                    prompt: Some(prompt.as_str()).filter(|p| !p.is_empty()),
                    cwd: $cwd,
                    label: &label,
                    mission_id: Some(task_id.as_str()).filter(|t| !t.is_empty()),
                    watch: self.chat.as_ref().map(|c| c.new_watch(&args)),
                    parent: team.as_ref().map(|(root, depth)| (root.as_str(), *depth)),
                    goal: super::GoalText {
                        goal: Some(goal.as_str()).filter(|g| !g.is_empty()),
                        done_criteria: Some(done.as_str()).filter(|d| !d.is_empty()),
                    },
                    purpose,
                    observe: &observe,
                    model,
                    worktree: worktree.clone(),
                    owns: &owns,
                    policy: policy.as_deref(),
                }
            };
        }
        let mut note = start.note;
        let mut report = super::spawn_on(
            &self.vault_path,
            &self.db,
            host_handle.clone(),
            launch!(&start.path),
        )
        .await;
        // A remote host cannot be asked whether a folder exists; its launch error says so, and
        // nothing was started, so the one retry is in the machine's own HQ folder.
        if let (Err(e), Some(asked), None) = (&report, requested, &note)
            && super::is_missing_dir_error(e)
        {
            let fallback = choose_start_dir(
                &host_handle,
                None,
                &cfg,
                &host,
                bound,
                (&label, &harness),
                &asker,
            )
            .await?;
            note = Some(format!(
                "{asked} does not exist on host '{}', so the session started in {} instead.",
                host_handle.name(),
                fallback.path.display()
            ));
            report = super::spawn_on(
                &self.vault_path,
                &self.db,
                host_handle.clone(),
                launch!(&fallback.path),
            )
            .await;
        }
        let mut report = report?;
        if let Some(note) = note {
            report["cwd_note"] = json!(note);
        }
        super::tag_origin(&self.db, &report, origin);
        let brief = super::connector_brief(&args, &["prompt", "goal", "done_criteria"]);
        let cwd = report["cwd"].as_str().unwrap_or("unknown").to_string();
        super::notify_connector_start(&self.db, &args, &report, &brief, &cwd);
        Ok(report)
    }
}

impl HarnessSessionSpawnTool {
    /// A spawn from a caller that says it runs inside session `from`. The claim is
    /// the caller's own word (the marker header or `from_session`), so the caller's
    /// own checks still apply: its origin cap, and its folder fence on the
    /// delegator's folder, where the worker will run. Only then does it become a
    /// delegation from that session.
    async fn delegate_claimed(&self, from: String, args: &Value) -> Result<Value> {
        // A worker joins its delegator's team and sees every member, more than the
        // handoff key may read itself, so that key cannot put one there.
        if super::CwdBound::of(args) == super::CwdBound::Handoff {
            anyhow::bail!(
                "the handoff key cannot start a worker in another session's team; start a session without from_session"
            );
        }
        let delegator = {
            let id = from.clone();
            self.db.with_conn(move |c| registry::get(c, &id))?
        }
        .filter(|row| row.status == registry::STATUS_RUNNING)
        .ok_or_else(|| anyhow::anyhow!("the delegating session is not running"))?;
        let origin = super::call_origin(None, args);
        super::check_origin_cap(&self.db, origin)?;
        let cfg = hq_core::config::HqConfig::load()?;
        let host_handle = crate::agent_host::host(Some(delegator.host.as_str()))?;
        let start = choose_start_dir(
            &host_handle,
            Some(delegator.cwd.as_str()),
            &cfg,
            &delegator.host,
            super::CwdBound::of(args),
            (&arg_str(args, "label"), &delegator.harness),
            &FolderAsker::of(&self.db, args),
        )
        .await?;
        if start.path != std::path::Path::new(&delegator.cwd) {
            anyhow::bail!(
                "session {from}'s folder {} is not one this caller may start sessions in",
                delegator.cwd
            );
        }
        let report =
            crate::a2a::delegate_from_spawn(self.vault_path.clone(), self.db.clone(), from, args)
                .await?;
        super::tag_origin(&self.db, &report, origin);
        let brief = super::connector_brief(args, &["prompt", "goal", "done_criteria"]);
        super::notify_connector_start(&self.db, args, &report, &brief, &delegator.cwd);
        Ok(report)
    }
}

pub struct HarnessSessionListTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionListTool {
    fn name(&self) -> &str {
        "harness_session_list"
    }
    fn description(&self) -> &str {
        "List harness sessions (running and recent) with live the host status (idle, working, blocked, done) and the host each runs on. A session on an unreachable host reports reachable=false instead of a guess. Pass task_id to list every session launched for one HQ task."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "description": "Optional filter: running, exited, stopped, orphaned" },
                "task_id": { "type": "string", "description": "Only the sessions of this HQ task (id or display id); ignores status" },
                "worktree_status": { "type": "boolean", "description": "Default false. When true, each running session with a worktree also reports worktree_status (changed files, ahead and behind its start), one host call each." }
            }
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let status = args
            .get("status")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let task = arg_str(&args, "task_id");
        let with_worktrees = args
            .get("worktree_status")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            if task.is_empty() {
                super::list_with(&db, status, with_worktrees)
            } else {
                super::list_for_task(&db, &task)
            }
        })
        .await?
    }
}

pub struct HarnessSessionStatusTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionStatusTool {
    fn name(&self) -> &str {
        "harness_session_status"
    }
    fn description(&self) -> &str {
        "Status of one harness session: registry row plus the live host agent status and whether its host is reachable. A session with a live worktree also reports worktree_status (changed files, ahead and behind its start)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        let caller = super::caller_session(&args).map(str::to_string);
        tokio::task::spawn_blocking(move || {
            // A launched agent may look at itself, its parent and its children.
            db.with_conn(|c| crate::a2a::check_session_access(c, caller.as_deref(), &id))?;
            super::status(&db, &id)
        })
        .await?
    }
}

pub struct HarnessSessionLogsTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionLogsTool {
    fn name(&self) -> &str {
        "harness_session_logs"
    }
    fn description(&self) -> &str {
        "Recent terminal output of a harness session: read live while it runs, from the last stored snapshot after it ends. To follow a session without reading the same lines again, pass `since` (\"\" on the first call, then the `next` each reply returns): the reply holds only the lines that scrolled off the screen since then, plus the current screen. An agent that redraws a full-screen view in place (Copilot) scrolls nothing off, so only its screen changes."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "lines": { "type": "integer", "description": "How many trailing lines (default 40); with `since`, the most new lines to return", "default": 40 },
                "since": { "type": "string", "description": "A cursor from an earlier reply's `next`, or \"\" to start one" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let lines = args.get("lines").and_then(|v| v.as_u64()).unwrap_or(40) as usize;
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        super::connector_may_drive(&db, &args, &id)?;
        let caller = super::caller_session(&args).map(str::to_string);
        let since = args
            .get("since")
            .and_then(Value::as_str)
            .map(str::to_string);
        tokio::task::spawn_blocking(move || {
            // A launched agent may read the sessions of its own team.
            db.with_conn(|c| crate::a2a::check_session_access(c, caller.as_deref(), &id))?;
            match since {
                Some(cursor) => super::logs_since(&db, &id, &cursor, lines.clamp(1, 500)),
                None => super::tail_log(&db, &id, lines.clamp(1, 500)),
            }
        })
        .await?
    }
}

pub struct HarnessSessionSendTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionSendTool {
    fn name(&self) -> &str {
        "harness_session_send"
    }
    fn description(&self) -> &str {
        "Steer a running harness session. `text` is submitted as a prompt. If the session is blocked at a dialog the text is refused: read the dialog with harness_session_pending and answer it with harness_session_answer, or pass `keys` (logical names such as enter, esc, down, ctrl+c) when HQ cannot read it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "text": { "type": "string", "description": "Prompt to submit" },
                "keys": { "type": "array", "items": { "type": "string" }, "description": "Logical keys to press instead of text, e.g. [\"down\", \"enter\"]" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        super::spawned_may_target(&args, &id)?;
        super::connector_may_drive(&db, &args, &id)?;
        let text = arg_str(&args, "text");
        let keys = arg_str_list(&args, "keys");
        if text.is_empty() && keys.is_empty() {
            anyhow::bail!("pass `text` to submit a prompt or `keys` to press keys");
        }
        let chat = self.chat.clone();
        tokio::task::spawn_blocking(move || {
            if keys.is_empty() {
                super::send(&db, &id, &text, chat.as_ref())
            } else {
                super::send_keys(&db, &id, &keys, chat.as_ref())
            }
        })
        .await?
    }
}

pub struct HarnessSessionPendingTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionPendingTool {
    fn name(&self) -> &str {
        "harness_session_pending"
    }
    fn description(&self) -> &str {
        "The dialog a blocked harness session waits at, as data: its number (`seq`), kind (command, edit, path, question, trust, other), question, the command or path it is about, the session's folder, every option, and the dialog's text. An option with `remember: true` would let the agent do more from now on without asking; `typed: true` opens a text field. Answer with harness_session_answer. `pending` is null when the session waits at no dialog HQ can read."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        super::connector_may_drive(&db, &args, &id)?;
        let caller = super::caller_session(&args).map(str::to_string);
        tokio::task::spawn_blocking(move || {
            db.with_conn(|c| crate::a2a::check_session_access(c, caller.as_deref(), &id))?;
            super::answer::pending(&db, &id)
        })
        .await?
    }
}

pub struct HarnessSessionAnswerTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionAnswerTool {
    fn name(&self) -> &str {
        "harness_session_answer"
    }
    fn description(&self) -> &str {
        "Answer the dialog a harness session waits at by option, not by keys: pass the `seq` from harness_session_pending and an `option` id, or `decline: true` (with an optional `message` the agent gets as its next prompt once the dialog is gone). Refused if the session is at a different dialog by now: read it again. Options with remember: true are the owner's call; Drive, agent sessions and claude.ai connections are refused them. The reply says registered: false (not_registered) when the dialog is still open afterwards."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "seq": { "type": "integer", "description": "The dialog's number from harness_session_pending" },
                "option": { "type": "integer", "description": "The id of the option to pick" },
                "decline": { "type": "boolean", "description": "Dismiss the dialog without picking an option" },
                "message": { "type": "string", "description": "With decline: what to tell the agent instead, sent once it is back at its prompt" }
            },
            "required": ["session_id", "seq"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        use super::answer::{Answerer, Choice};
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        super::spawned_may_target(&args, &id)?;
        super::connector_may_drive(&db, &args, &id)?;
        let seq = args.get("seq").and_then(Value::as_u64).ok_or_else(|| {
            anyhow::anyhow!("pass `seq`, the dialog's number from harness_session_pending")
        })?;
        let option = args.get("option").and_then(Value::as_u64);
        let decline = args.get("decline").and_then(Value::as_bool) == Some(true);
        let message = Some(arg_str(&args, "message")).filter(|m| !m.trim().is_empty());
        let choice = match (option, decline) {
            (Some(o), false) => {
                Choice::Option(u32::try_from(o).map_err(|_| anyhow::anyhow!("no option {o}"))?)
            }
            (None, true) => Choice::Decline(message),
            _ => anyhow::bail!("pass either `option` or `decline: true`"),
        };
        let who = Answerer::of(&args, self.chat.as_ref());
        let chat = self.chat.clone();
        tokio::task::spawn_blocking(move || {
            super::answer::answer(&db, &id, seq, choice, who, chat.as_ref())
        })
        .await?
    }
}

pub struct HarnessSessionStopTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionStopTool {
    fn name(&self) -> &str {
        "harness_session_stop"
    }
    fn description(&self) -> &str {
        "Stop a harness session (closes its the host workspace on its host; resume token is kept). Fails if the host is unreachable."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        super::connector_may_drive(&db, &args, &id)?;
        tokio::task::spawn_blocking(move || super::stop(&db, &id)).await?
    }
}

pub struct HarnessSessionResumeTool {
    vault_path: PathBuf,
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionResumeTool {
    fn name(&self) -> &str {
        "harness_session_resume"
    }
    fn description(&self) -> &str {
        "Resume a stopped/exited harness session using its saved resume token or session dir. Falls back to a fresh session in the same cwd when the harness has no resume support."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "prompt": { "type": "string", "description": "Optional prompt typed after resuming" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_if_spawned(&args)?;
        let prompt = arg_str(&args, "prompt");
        let id = arg_str(&args, "session_id");
        super::connector_may_drive(&self.db, &args, &id)?;
        let reserved =
            super::reserve_send(&self.db, &id, self.chat.as_ref(), super::SendKind::Text)?;
        let result = super::resume(
            &self.vault_path,
            &self.db,
            &id,
            Some(prompt.as_str()).filter(|p| !p.is_empty()),
        )
        .await;
        super::settle_send(
            &self.db,
            &id,
            self.chat.as_ref(),
            super::SendKind::Text,
            reserved,
            result.is_ok(),
        );
        result
    }
}

pub struct HarnessSessionWaitTool {
    db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionWaitTool {
    fn name(&self) -> &str {
        "harness_session_wait"
    }
    fn description(&self) -> &str {
        "Block until a harness session settles: idle or done (ready for input, usually finished a turn) or blocked (waiting on a dialog). Returns settled=false if the timeout passes first. Use after harness_session_send instead of polling logs."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "description": "Seconds to wait, 3 to 45 (default 40). To wait longer, call again.", "default": 40 },
                "until": { "type": "array", "items": { "type": "string", "enum": ["idle", "working", "blocked", "done"] }, "description": "States that end the wait (default: idle, done, blocked)" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(super::DEFAULT_WAIT.as_secs());
        let until: Vec<crate::agent_host::AgentStatus> = args
            .get("until")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(crate::agent_host::AgentStatus::parse)
                    .collect()
            })
            .unwrap_or_default();
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        let timeout = std::time::Duration::from_secs(secs.clamp(3, super::MAX_WAIT.as_secs()));
        let caller = super::caller_session(&args).map(str::to_string);
        tokio::task::spawn_blocking(move || {
            db.with_conn(|c| crate::a2a::check_session_access(c, caller.as_deref(), &id))?;
            super::wait(&db, &id, &until, timeout)
        })
        .await?
    }
}

pub struct HarnessSessionLinkTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionLinkTool {
    fn name(&self) -> &str {
        "harness_session_link"
    }
    fn description(&self) -> &str {
        "Link an existing harness session to an HQ task, for a session launched without task_id, or move a session on to the next task of a plan. From then on the supervisor records its lifecycle on that task, as harness_session_spawn does with task_id. Linking a running session moves a to_do, blocked or ready_for_review task to in_progress. When the session was on another task, its goal moves to the new one (Drive reads that task, not the launch brief) and the reply has next_message, a fixed line naming the task; continue: true sends it to the session."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "task_id": { "type": "string", "description": "HQ task id or display id such as FR-053. Must exist and not be complete." },
                "drive": { "type": "boolean", "description": "Web chat only. Omit to follow the default (a session no chat watched starts with Drive on); false starts the watch with Drive off, or stops driving a session this chat already drives." },
                "continue": { "type": "boolean", "description": "When the session moves from another task, send it \"Continue with <task id>: <title>. Its acceptance is on the task.\" (never the task's description)" }
            },
            "required": ["session_id", "task_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_attach(self.chat.as_ref())?;
        let db = self.db.clone();
        let (id, task) = (arg_str(&args, "session_id"), arg_str(&args, "task_id"));
        if id.is_empty() || task.is_empty() {
            anyhow::bail!("session_id and task_id are required");
        }
        super::connector_may_drive(&db, &args, &id)?;
        let chat = self.chat.clone();
        let tell = args.get("continue").and_then(Value::as_bool) == Some(true);
        tokio::task::spawn_blocking(move || {
            super::link_with(
                &db,
                &id,
                &task,
                chat.as_ref().map(|c| c.new_watch(&args)),
                tell,
                chat.as_ref(),
            )
        })
        .await?
    }
}

pub struct HarnessSessionWatchTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionWatchTool {
    fn name(&self) -> &str {
        "harness_session_watch"
    }
    fn description(&self) -> &str {
        "Have this web chat watch a harness session: its finished turns, blocks and exit post into this chat instead of Telegram, and it shows in the chat's Watching panel. A session no chat watched starts with Drive on (HQ answering it and approving its prompts toward its task) unless you pass drive=false; one this chat already watches keeps its Drive switch, and one taken from another chat starts with Drive off. Only the user turns Drive back on, with the switch in that panel; drive=false stops driving. Only available in a web chat."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
        "drive": { "type": "boolean", "description": "Web chat only. Omit to follow the default (a session no chat watched starts with Drive on); false starts the watch with Drive off, or stops driving a session this chat already drives." }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(chat) = self.chat.clone() else {
            anyhow::bail!(
                "harness_session_watch only works from a web chat; this session has no chat thread"
            );
        };
        // An explicit true could re-enable Drive the user switched off, so only
        // the default or the user's switch turns it on.
        if args.get("drive").and_then(Value::as_bool) == Some(true) {
            anyhow::bail!(
                "drive=true is not accepted: omit drive to watch with the default (Drive on for a session no chat watched), and the user turns Drive on for any other with the switch in this chat's Watching panel. The session is not watched yet"
            );
        }
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        tokio::task::spawn_blocking(move || super::watch(&db, &id, chat.new_watch(&args))).await?
    }
}

pub struct HarnessSessionGoalTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionGoalTool {
    fn name(&self) -> &str {
        "harness_session_goal"
    }
    fn description(&self) -> &str {
        "Record what a harness session is for (`goal`) and what would be observable when it is met (`done_criteria`), for a session launched without them or whose aim changed. HQ drives a session only while both are specific enough to judge completion (not empty, not a placeholder, not just the goal repeated); editing them so they no longer are switches HQ to observing. A session exiting or going idle is never evidence the goal was met, and a person reviews the work. Every change is audited."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "goal": { "type": "string", "description": "What the session should accomplish, specific to the work" },
                "done_criteria": { "type": "string", "description": "Observable conditions that show the goal is met, e.g. a test passing or an endpoint's behaviour" }
            },
            "required": ["session_id"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        if args.get(super::UNTRUSTED_TURN_ARG).and_then(Value::as_bool) == Some(true) {
            anyhow::bail!(
                "this turn read untrusted content, so it cannot change a session's goal; ask the user"
            );
        }
        super::connector_may_drive(&self.db, &args, &arg_str(&args, "session_id"))?;
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        let (goal, done) = (arg_str(&args, "goal"), arg_str(&args, "done_criteria"));
        let actor = if self.chat.is_some() {
            registry::ACTOR_HQ
        } else {
            registry::ACTOR_MCP
        };
        tokio::task::spawn_blocking(move || {
            let text = super::GoalText {
                goal: Some(goal.as_str()).filter(|g| !g.is_empty()),
                done_criteria: Some(done.as_str()).filter(|d| !d.is_empty()),
            };
            super::set_goal(&db, &id, text, actor)
        })
        .await?
    }
}

pub struct HarnessSessionModeTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
}

#[async_trait]
impl HqTool for HarnessSessionModeTool {
    fn name(&self) -> &str {
        "harness_session_mode"
    }
    fn description(&self) -> &str {
        "Switch HQ between `drive` (answering the session and approving its prompts toward its goal) and `observe` (watching and reporting only) for a session this chat watches. `observe` always works and takes effect at once. `drive` needs a live session and a goal and definition of done that pass the drive gate (see harness_session_goal); otherwise HQ stays observing and the result says what is missing. Only HQ's steering changes: the agent is never paused or stopped by this, use harness_session_stop for that. Only available in a web chat."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "mode": { "type": "string", "enum": ["drive", "observe"] }
            },
            "required": ["session_id", "mode"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(chat) = self.chat.clone() else {
            anyhow::bail!(
                "harness_session_mode only works from a web chat; this session has no chat thread"
            );
        };
        let drive = match arg_str(&args, "mode").as_str() {
            "drive" => true,
            "observe" => false,
            other => anyhow::bail!("mode must be `drive` or `observe`, not '{other}'"),
        };
        if drive && (chat.driver_turn || chat.from_ask) {
            anyhow::bail!(
                "this turn cannot turn Drive on; only the user can, with the switch in the Watching panel"
            );
        }
        let untrusted = args.get(super::UNTRUSTED_TURN_ARG).and_then(Value::as_bool) == Some(true);
        let db = self.db.clone();
        let id = arg_str(&args, "session_id");
        tokio::task::spawn_blocking(move || {
            let req = super::ModeRequest {
                thread: &chat.thread,
                drive,
                untrusted,
                actor: registry::ACTOR_HQ,
            };
            super::set_mode(&db, &id, req)
        })
        .await?
    }
}

pub struct HarnessSessionAttachTool {
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

#[async_trait]
impl HqTool for HarnessSessionAttachTool {
    fn name(&self) -> &str {
        "harness_session_attach"
    }
    fn description(&self) -> &str {
        "Attach this web chat to a coding agent the host already runs that HQ did not launch here, such as one the user started by hand (find it with host_agents). It becomes a tracked session this chat watches, observation-only: give it a goal and definition of done with harness_session_goal, then harness_session_mode to drive. A host that is unreachable or an agent that is gone is reported as such, never pretended attached. Only available in a web chat."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "Agent name from host_agents" },
                "host": { "type": "string", "description": "host from host_list (default: the configured default host)" }
            },
            "required": ["agent"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_attach(self.chat.as_ref())?;
        if let Some(guest) = &self.family_guest {
            anyhow::bail!(
                "{} cannot attach to existing sessions; only the owner can",
                guest.name
            );
        }
        let Some(chat) = self.chat.clone() else {
            anyhow::bail!(
                "harness_session_attach only works from a web chat; this session has no chat thread"
            );
        };
        let target = arg_str(&args, "agent");
        if target.is_empty() {
            anyhow::bail!("`agent` is required; host_agents lists the names");
        }
        let host_name = arg_str(&args, "host");
        let host = crate::agent_host::host(Some(host_name.as_str()).filter(|h| !h.is_empty()))?;
        let db = self.db.clone();
        crate::agent_host::blocking(move || super::attach(&db, &host, &target, &chat.thread))
            .await?
    }
}

pub struct HarnessSessionHandoffTool {
    vault_path: PathBuf,
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
}

#[async_trait]
impl HqTool for HarnessSessionHandoffTool {
    fn name(&self) -> &str {
        "harness_session_handoff"
    }
    fn description(&self) -> &str {
        "Hand a piece of work to an agent in one call: files an HQ task (or reuses one), starts a session linked to it with the goal and acceptance criteria, and opens a web chat thread that owns the session so its finished turns, blocks and Drive show up there instead of the relay. Returns the task id and display id, session id, thread id and web links (/chat?thread=<id>, /tasks?task=<id>). Idempotent: the same external_id returns the same task, and a task that already has a live session gets no second one. If the host is unreachable or the agent stops at a dialog the result says so (blocked_at_dialog, prompt not typed) and nothing is claimed as running."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Task title. Required unless task_id names an existing task." },
                "description": { "type": "string", "description": "What the work is, for the task and the agent" },
                "acceptance": { "type": "string", "description": "Observable conditions that show the work is done (also accepted as done_criteria). HQ drives the session only when these are specific." },
                "done_criteria": { "type": "string", "description": "Alias of acceptance" },
                "harness": { "type": "string", "description": format!("One of: {}. A general-purpose agent ({}) refuses code work; give it purpose research, review or general.", super::spec::known_harnesses().join(", "), super::spec::names_in(super::HarnessCategory::GeneralPurpose).join(", ")) },
                "purpose": { "type": "string", "enum": ["code", "monitor", "research", "review", "general"], "description": "What the session is for. Default: code for a coding agent, general for a general-purpose agent." },
                "worktree": { "type": "object", "properties": { "branch": { "type": "string" }, "from": { "type": "string" }, "detached": { "type": "string" } }, "description": "Start in a git worktree of its own, under the host's HQ folder, so it never shares a working tree with another agent: {\"branch\": \"<name>\"} for a writer (made from the folder's current commit if it does not exist; refused while another worktree has it checked out), {\"detached\": \"<commit or ref>\"} for a reviewer. {\"branch\": \"<name>\", \"from\": \"<ref>\"} starts a new branch at that ref instead (ignored when the branch exists). A new worktree gets the gitignored files the repository's .worktreeinclude names (such as .env, node_modules), copied as copy-on-write clones, listed in the reply's worktree.copied. Removed at stop or exit when nothing is uncommitted; the branch keeps every commit." },
                "owns": { "type": "array", "items": { "type": "string" }, "description": "Paths, relative to cwd, this session writes to. A coding session in a folder another coding session already writes in is refused unless both own paths that do not overlap (no owns means the whole folder). Use a worktree instead when they would overlap." },
                "policy": { "type": "string", "description": "The command policy it runs under: builder or reviewer, one the owner configured under agent_host.command_policies, or none (every dialog goes to the owner). Default: builder for a coding session, reviewer for a review. A policy answers the dialogs it decides; see docs/AGENT_SESSIONS.md." },
                "team": { "type": "string", "description": "A root session id: the new session joins that session's team as a worker under it, with the team's roster, messages between members, stop-with-root and limits. For the owner, a web chat and a claude.ai connection (on a team it started); an agent inside a session uses agent_delegate. A reviewer joining a team observes its running coding sessions unless observe names others." },
                "model": { "type": "string", "description": "Model to start the agent on, passed with its CLI's --model flag (for example sonnet or gpt-5.5 for claude-code, cursor or codex). Omit for the agent's own default. Refused, before anything starts, for an agent with no model flag HQ knows. The result's `model` reports what was passed." },
                "cwd": { "type": "string", "description": "Project directory on that host. Optional: when omitted, or when the folder does not exist there, the session starts in a folder inside that machine's own HQ folder (Documents/HQ), created if needed. A home or root directory is refused, as is anything matching agent_host.spawn_cwd_deny. A claude.ai connector (and the handoff key while agent_host.handoff_cwd_allow lists anything) may use only the host's HQ folder, that list and folders the owner approved. Asking for any other folder sends the owner an approval request and this call waits up to 30 seconds for their answer: it starts the session if they approve, and otherwise fails with BLOCKED and starts nothing. Tell the user you are waiting, then call again with the same arguments. Never start elsewhere and cd into it." },
                "host": { "type": "string", "description": "host from host_list (default: the configured default host, normally local)" },
                "external_id": { "type": "string", "description": "Idempotency key for the task, unique per space. Not combinable with task_id." },
                "task_id": { "type": "string", "description": "Work on this existing HQ task (id or display id) instead of filing one. Must not be complete." },
                "prompt": { "type": "string", "description": "First prompt typed into the session. Default: the task title, description and acceptance criteria." },
                "space_id": { "type": "string", "description": "Space slug to file a new task in (default personal)" },
                "initiative": { "type": "string", "description": "Initiative name to file a new task under (default Inbox)" },
                "drive": { "type": "boolean", "description": "false starts the thread watching without HQ driving the session. Omit to follow agent_host.drive_new_watches." }
            },
            "required": ["harness"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        super::refuse_if_spawned(&args)?;
        if self.chat.as_ref().is_some_and(|c| c.driver_turn) {
            anyhow::bail!("a driver turn cannot start sessions; tell the user what you need");
        }
        if let Some(guest) = &self.family_guest {
            anyhow::bail!(
                "{} cannot hand work to coding agents; only the owner can",
                guest.name
            );
        }
        let origin = super::call_origin(self.chat.as_ref(), &args);
        super::check_origin_cap(&self.db, origin)?;
        let cfg = hq_core::config::HqConfig::load()?;
        let host_name = arg_str(&args, "host");
        let host = crate::agent_host::host(Some(host_name.as_str()).filter(|h| !h.is_empty()))?;
        let requested = args
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty());
        let (label, harness) = (arg_str(&args, "title"), arg_str(&args, "harness"));
        let model = arg_str(&args, "model");
        // Before a start folder is chosen or made, so a refused model leaves nothing behind.
        super::spec::resolve(&harness)?.with_model(Some(model.trim()).filter(|m| !m.is_empty()))?;
        let bound = super::CwdBound::of(&args);
        let asker = FolderAsker::of(&self.db, &args);
        let start = choose_start_dir(
            &host,
            requested,
            &cfg,
            &host_name,
            bound,
            (&label, &harness),
            &asker,
        )
        .await?;
        let _joining = match super::team::parse(&args) {
            Some(_) => Some(crate::a2a::DELEGATION_LOCK.lock().await),
            None => None,
        };
        let team = match super::team::parse(&args) {
            Some(root) => Some(super::team::join(&self.db, &args, &root)?),
            None => None,
        };
        let owned = |key: &str| arg_str(&args, key);
        let acceptance = [owned("acceptance"), owned("done_criteria")]
            .into_iter()
            .find(|a| !a.trim().is_empty())
            .unwrap_or_default();
        let report = super::handoff::handoff(
            &self.vault_path,
            &self.db,
            host,
            super::handoff::HandoffRequest {
                title: owned("title"),
                description: owned("description"),
                acceptance,
                harness: owned("harness"),
                cwd: start.path.to_string_lossy().into_owned(),
                external_id: owned("external_id"),
                task_id: owned("task_id"),
                prompt: owned("prompt"),
                space_id: owned("space_id"),
                initiative: owned("initiative"),
                // A handoff from an hq_ask reply or over the handoff key never starts out driving,
                // so a handoff-key holder cannot fill the driven-session cap and starve the owner.
                drive_new: handoff_drives_new(
                    cfg.agent_host.drive_new_watches,
                    &args,
                    self.chat.as_ref(),
                ),
                drive_opted_out: args.get("drive").and_then(Value::as_bool) == Some(false),
                model: owned("model"),
                purpose: super::Purpose::parse_opt(args.get("purpose").and_then(Value::as_str))?,
                worktree: super::checkout::parse_worktree(&args)?,
                owns: super::checkout::parse_owns(&args)?,
                policy: owned("policy"),
                team: team.clone(),
            },
        )
        .await?;
        let mut report = report;
        if let Some(note) = start.note {
            report["cwd_note"] = json!(note);
        }
        // A handoff that found a session already working started nothing: its origin stays as it was.
        if report["handoff"] != "existing_session" {
            super::tag_origin(&self.db, &report, origin);
        }
        let mut brief = super::connector_brief(
            &args,
            &[
                "task_id",
                "title",
                "description",
                "acceptance",
                "done_criteria",
                "prompt",
            ],
        );
        // The prompt is built from the stored task, which may hold text written earlier.
        if let Some(task_id) = report["task"]["id"].as_str() {
            let stored = self.db.with_conn(|c| hq_db::tasks::get_task(c, task_id));
            if let Ok(Some(task)) = stored {
                brief.push_str(&format!(
                    "\nstored task title: {}\nstored task description: {}",
                    task.title, task.description
                ));
            }
        }
        let cwd = start.path.to_string_lossy().into_owned();
        super::notify_connector_start(&self.db, &args, &report, &brief, &cwd);
        Ok(report)
    }
}

/// Whether a handoff may start its session driven: not from the handoff key (so its holder cannot
/// fill the driven-session cap), not from a claude.ai connector for the same reason, and not from
/// an `hq_ask` reply.
pub(super) fn handoff_drives_new(
    default_on: bool,
    args: &Value,
    chat: Option<&WatchingChat>,
) -> bool {
    default_on
        && !super::is_handoff_scope(args)
        && super::connector_label(args).is_none()
        && !chat.is_some_and(|c| c.from_ask)
}

/// `chat` is the web chat of the calling session, if any.
pub fn create_harness_session_tools(
    vault_path: PathBuf,
    db: Arc<Database>,
    chat: Option<WatchingChat>,
    family_guest: Option<crate::family_guest::FamilyGuestContext>,
) -> Vec<Box<dyn HqTool>> {
    let family_guest_for_attach = family_guest.clone();
    let family_guest_for_handoff = family_guest.clone();
    vec![
        Box::new(HarnessSessionHandoffTool {
            vault_path: vault_path.clone(),
            db: db.clone(),
            chat: chat.clone(),
            family_guest: family_guest_for_handoff,
        }),
        Box::new(HarnessSessionSpawnTool {
            vault_path: vault_path.clone(),
            db: db.clone(),
            chat: chat.clone(),
            family_guest,
        }),
        Box::new(HarnessSessionListTool { db: db.clone() }),
        Box::new(HarnessSessionStatusTool { db: db.clone() }),
        Box::new(HarnessSessionLogsTool { db: db.clone() }),
        Box::new(HarnessSessionSendTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionWaitTool { db: db.clone() }),
        Box::new(HarnessSessionPendingTool { db: db.clone() }),
        Box::new(super::events::HarnessSessionEventsTool { db: db.clone() }),
        Box::new(super::decisions::TaskDecisionsTool { db: db.clone() }),
        Box::new(super::stats::HarnessSessionStatsTool { db: db.clone() }),
        Box::new(HarnessSessionAnswerTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionStopTool { db: db.clone() }),
        Box::new(HarnessSessionLinkTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionWatchTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionGoalTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionModeTool {
            db: db.clone(),
            chat: chat.clone(),
        }),
        Box::new(HarnessSessionAttachTool {
            db: db.clone(),
            chat: chat.clone(),
            family_guest: family_guest_for_attach,
        }),
        Box::new(HarnessSessionResumeTool {
            vault_path,
            db,
            chat,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family_guest::FamilyGuestContext;

    #[tokio::test]
    async fn family_guest_rejects_non_agy_harness() {
        let db = Arc::new(Database::open_memory().unwrap());
        let tool = HarnessSessionSpawnTool {
            vault_path: PathBuf::from("/tmp"),
            db,
            chat: None,
            family_guest: Some(FamilyGuestContext {
                name: "Bob".into(),
                origin_channel_id: 12345,
                owner_name: "Owner".into(),
                allowed_harnesses: vec!["agy".into()],
            }),
        };

        let err = tool
            .execute(json!({ "harness": "claude-code" }))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("Bob only has access to these coding-agent harnesses through me: agy")
        );
        assert!(err.to_string().contains("'claude-code' is not one of them"));

        let err_cursor = tool
            .execute(json!({ "harness": "cursor" }))
            .await
            .unwrap_err();
        assert!(
            err_cursor
                .to_string()
                .contains("'cursor' is not one of them")
        );
    }

    #[tokio::test]
    async fn non_family_guest_allows_any_harness() {
        let db = Arc::new(Database::open_memory().unwrap());
        let tool = HarnessSessionSpawnTool {
            vault_path: PathBuf::from("/tmp"),
            db,
            chat: None,
            family_guest: None,
        };

        // A root cwd is refused after the guest check and before any host is asked, so the
        // test never launches a real agent on the machine running it.
        let err = tool
            .execute(json!({ "harness": "claude-code", "cwd": "/" }))
            .await
            .unwrap_err()
            .to_string();
        assert!(!err.contains("is not one of them"), "{err}");
        assert!(err.contains("home or root directory"), "{err}");
    }
}

#[cfg(test)]
mod folder_bound_tests {
    use super::*;

    #[test]
    fn the_handoff_key_with_an_empty_list_stays_unrestricted_and_a_connector_never_is() {
        let mut cfg = hq_core::config::AgentHostConfig::default();
        assert!(!bounded_by_folders(super::super::CwdBound::Handoff, &cfg));
        assert!(!bounded_by_folders(super::super::CwdBound::Any, &cfg));
        assert!(bounded_by_folders(super::super::CwdBound::Connector, &cfg));
        cfg.handoff_cwd_allow = vec!["  ".into()];
        assert!(!bounded_by_folders(super::super::CwdBound::Handoff, &cfg));
        cfg.handoff_cwd_allow = vec!["/srv/work".into()];
        assert!(bounded_by_folders(super::super::CwdBound::Handoff, &cfg));
    }
}
