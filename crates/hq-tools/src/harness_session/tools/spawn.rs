//! Starting a session: the spawn tool and the folder checks it runs.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::Database;
use serde_json::{Value, json};
use std::sync::Arc;

use super::registry;
use crate::harness_session::WatchingChat;
use crate::registry::HqTool;
use crate::util::{arg_str, arg_str_list};

/// Where a tool-started session runs. An explicit folder that exists is used as given. A blank one,
/// or one the host does not have, falls back to a folder in that machine's HQ folder, which the
/// host creates; the deny and allow lists then judge the folder actually used.
pub(super) async fn choose_start_dir(
    host: &crate::agent_host::Host,
    requested: Option<&str>,
    cfg: &hq_core::config::HqConfig,
    host_name: &str,
    bound: crate::harness_session::CwdBound,
    names: (&str, &str),
    asker: &FolderAsker<'_>,
) -> Result<crate::harness_session::StartDir> {
    let (label, harness) = names;
    let granted;
    let agent_host = if bounded_by_folders(bound, &cfg.agent_host) {
        let mut folders = if bound == crate::harness_session::CwdBound::Connector {
            crate::harness_session::connector_folders(
                &cfg.agent_host,
                host_hq_folder(host).await.as_deref(),
            )
        } else {
            cfg.agent_host.clone()
        };
        let grants = crate::harness_session::folder_request::granted_folders(
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
    if bound == crate::harness_session::CwdBound::Connector && requested.is_none() {
        crate::harness_session::connector_may_spawn(&agent_host.handoff_cwd_allow)?;
    }
    let vault_guard = |p: &std::path::Path| {
        crate::harness_session::check_cwd_outside_vault(
            p,
            host_name,
            &agent_host.default_host,
            &cfg.vault_path,
        )
    };
    let checked = requested
        .map(|r| crate::harness_session::require_cwd_in(Some(r), agent_host, bound))
        .transpose();
    let wanted = match checked {
        Ok(wanted) => wanted,
        Err(e) => {
            let Some(outside) = e.downcast_ref::<crate::harness_session::OutsideAllowedFolders>()
            else {
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
        return Ok(crate::harness_session::StartDir {
            path: path.clone(),
            note: None,
        });
    }
    let (h, w, l, k) = (host.clone(), wanted, label.to_string(), harness.to_string());
    let fallback = tokio::task::spawn_blocking(move || {
        crate::harness_session::workspace_start_dir(&*h, w.as_deref(), &l, &k)
    })
    .await??;
    crate::harness_session::require_cwd_in(
        Some(&fallback.path.to_string_lossy()),
        agent_host,
        bound,
    )?;
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
    bound: crate::harness_session::CwdBound,
    asker: &FolderAsker<'_>,
) -> Result<std::path::PathBuf> {
    use crate::harness_session::folder_request::{self as requests, Answer, AwaitingApproval};
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
            if bound == crate::harness_session::CwdBound::Connector {
                folders = crate::harness_session::connector_folders(
                    agent_host,
                    host_hq_folder(host).await.as_deref(),
                );
            }
            folders.handoff_cwd_allow.extend(requests::granted_folders(
                asker.db,
                host_name,
                &asker.requester,
                Some(&shown),
            )?);
            crate::harness_session::require_cwd_in(Some(&shown), &folders, bound)
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
pub(super) struct FolderAsker<'a> {
    pub(super) db: &'a hq_db::Database,
    requester: String,
}

impl<'a> FolderAsker<'a> {
    pub(super) fn of(db: &'a hq_db::Database, args: &Value) -> Self {
        let requester = crate::harness_session::connector_label(args)
            .map(str::to_string)
            .unwrap_or_else(|| {
                crate::harness_session::folder_request::HANDOFF_REQUESTER.to_string()
            });
        Self { db, requester }
    }
}

/// Whether a caller is held to a folder list: a connector always, the handoff key only while
/// `handoff_cwd_allow` lists something (empty leaves that key unrestricted).
fn bounded_by_folders(
    bound: crate::harness_session::CwdBound,
    agent_host: &hq_core::config::AgentHostConfig,
) -> bool {
    match bound {
        crate::harness_session::CwdBound::Any => false,
        crate::harness_session::CwdBound::Connector => true,
        crate::harness_session::CwdBound::Handoff => agent_host
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
    pub(super) db: Arc<Database>,
    pub(super) chat: Option<WatchingChat>,
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
                "harness": { "type": "string", "description": format!("One of: {} (built-in harnesses plus any agent_host.harness_profiles). Coding agents: {}. General-purpose agents: {}.", crate::harness_session::spec::known_harnesses().join(", "), crate::harness_session::spec::names_in(crate::harness_session::HarnessCategory::Coding).join(", "), crate::harness_session::spec::names_in(crate::harness_session::HarnessCategory::GeneralPurpose).join(", ")) },
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
        if let Some(from) = crate::harness_session::delegating_session(&args) {
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
        let purpose = crate::harness_session::Purpose::parse_opt(
            args.get("purpose").and_then(Value::as_str),
        )?;
        let mut observe = arg_str_list(&args, "observe");
        // Joining a remote client's team: checked and held so two starts cannot both pass
        // the team's limits.
        let _joining = match crate::harness_session::team::parse(&args) {
            Some(_) => Some(crate::session_policy::DELEGATION_LOCK.lock().await),
            None => None,
        };
        let team = match crate::harness_session::team::parse(&args) {
            Some(root) => Some(crate::harness_session::team::join(&self.db, &args, &root)?),
            None => None,
        };
        // A reviewer in a team watches the team's writers unless it names its own.
        if let Some((root, _)) = &team
            && purpose == Some(crate::harness_session::Purpose::Review)
            && observe.is_empty()
            && !task_id.trim().is_empty()
        {
            observe = crate::harness_session::team::writers(&self.db, root);
        }
        crate::harness_session::subscribe::check_spawn_observe(&self.db, &args, &observe)?;
        let worktree = crate::harness_session::checkout::parse_worktree(&args)?;
        let owns = crate::harness_session::checkout::parse_owns(&args)?;
        let policy = crate::harness_session::policy::parse(&args);
        let model = arg_str(&args, "model");
        let model = Some(model.trim()).filter(|m| !m.is_empty());
        // Before a start folder is chosen or made, so a refused model leaves nothing behind.
        crate::harness_session::spec::resolve(&harness)?.with_model(model)?;
        let cfg = hq_core::config::HqConfig::load()?;
        if self.chat.as_ref().is_some_and(|c| c.driver_turn) {
            anyhow::bail!("a driver turn cannot start sessions; tell the user what you need");
        }
        let origin = crate::harness_session::call_origin(self.chat.as_ref(), &args);
        crate::harness_session::check_origin_cap(&self.db, origin)?;
        let host_handle = crate::agent_host::host(Some(host.as_str()).filter(|h| !h.is_empty()))?;
        let requested = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|c| !c.is_empty());
        let bound = crate::harness_session::CwdBound::of(&args);
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
            ($cwd:expr, $lane:expr) => {
                crate::harness_session::SpawnRequest {
                    host: Some(host.as_str()).filter(|h| !h.is_empty()),
                    harness: $lane,
                    prompt: Some(prompt.as_str()).filter(|p| !p.is_empty()),
                    cwd: $cwd,
                    label: &label,
                    mission_id: Some(task_id.as_str()).filter(|t| !t.is_empty()),
                    watch: self.chat.as_ref().map(|c| c.new_watch(&args)),
                    parent: team.as_ref().map(|(root, depth)| (root.as_str(), *depth)),
                    goal: crate::harness_session::GoalText {
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
        // A start that joins no team claims a slot here, after any folder approval, and keeps
        // it only until the session row is written, so a waiting approval never blocks others.
        let slot = match &team {
            None => Some(crate::harness_session::spawn_limits::reserve(
                &self.db,
                &cfg.agent_host.spawn_limits,
                &cfg.budgets,
                &harness,
                chrono::Utc::now().timestamp(),
            )?),
            Some(_) => None,
        };
        let mut note = start.note;
        let plan = crate::harness_session::quota_gate::Plan::new(
            &self.db,
            &cfg,
            host_handle.name(),
            &harness,
            chrono::Utc::now(),
        );
        let first_lane = plan.chain[plan.begin].clone();
        let mut started_lane = first_lane.clone();
        let mut report = crate::harness_session::spawn_on(
            &self.db,
            host_handle.clone(),
            launch!(&start.path, &first_lane),
        )
        .await;
        // A remote host cannot be asked whether a folder exists; its launch error says so, and
        // nothing was started, so the one retry is in the machine's own HQ folder.
        if let (Err(e), Some(asked), None) = (&report, requested, &note)
            && crate::harness_session::is_missing_dir_error(e)
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
            report = crate::harness_session::spawn_on(
                &self.db,
                host_handle.clone(),
                launch!(&fallback.path, &first_lane),
            )
            .await;
        }
        // A lane skipped for its quota is a fallback too, whether or not the start then worked.
        let mut fell_back = plan
            .skipped_reason()
            .map(|why| json!({ "from": harness, "to": first_lane, "reason": why }));
        if let Err(first) = &report
            && crate::harness_session::fallback::lane_unavailable(&first.to_string())
        {
            let why = match plan.skipped_reason() {
                Some(quota) => format!("{quota}; {first_lane}: {first}"),
                None => first.to_string(),
            };
            let open = plan.any_open();
            for lane in plan.chain.iter().skip(plan.begin + 1) {
                if open && plan.is_blocked(lane) {
                    continue;
                }
                let tried = crate::harness_session::spawn_on(
                    &self.db,
                    host_handle.clone(),
                    launch!(&start.path, lane),
                )
                .await;
                match tried {
                    Ok(started) => {
                        fell_back = Some(json!({ "from": harness, "to": lane, "reason": why }));
                        started_lane = lane.clone();
                        report = Ok(started);
                        break;
                    }
                    Err(e)
                        if crate::harness_session::fallback::lane_unavailable(&e.to_string()) =>
                    {
                        continue;
                    }
                    Err(e) => {
                        report = Err(e);
                        break;
                    }
                }
            }
        }
        drop(slot);
        let mut report = report?;
        if let Some(fell_back) = fell_back {
            report["fell_back"] = fell_back;
        }
        if let Some(warning) = plan.warning(&started_lane) {
            report["quota_warning"] = warning;
        }
        if let Some(note) = note {
            report["cwd_note"] = json!(note);
        }
        crate::harness_session::tag_origin(&self.db, &report, origin);
        if !prompt.is_empty()
            && let Some(id) = report["session_id"].as_str()
        {
            let who = crate::harness_session::answer::Answerer::of(&args, self.chat.as_ref());
            crate::harness_session::turn_origin::note_prompt(id, who);
        }
        let brief = crate::harness_session::with_observed(
            crate::harness_session::connector_brief(&args, &["prompt", "goal", "done_criteria"]),
            &observe,
        );
        let cwd = report["cwd"].as_str().unwrap_or("unknown").to_string();
        crate::harness_session::notify_connector_start(&self.db, &args, &report, &brief, &cwd);
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
        if crate::harness_session::CwdBound::of(args) == crate::harness_session::CwdBound::Handoff {
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
        let origin = crate::harness_session::call_origin(None, args);
        crate::harness_session::check_origin_cap(&self.db, origin)?;
        let cfg = hq_core::config::HqConfig::load()?;
        let host_handle = crate::agent_host::host(Some(delegator.host.as_str()))?;
        let start = choose_start_dir(
            &host_handle,
            Some(delegator.cwd.as_str()),
            &cfg,
            &delegator.host,
            crate::harness_session::CwdBound::of(args),
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
            super::super::delegator::delegate_from_spawn(self.db.clone(), from, args).await?;
        crate::harness_session::tag_origin(&self.db, &report, origin);
        let brief =
            crate::harness_session::connector_brief(args, &["prompt", "goal", "done_criteria"]);
        crate::harness_session::notify_connector_start(
            &self.db,
            args,
            &report,
            &brief,
            &delegator.cwd,
        );
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family_guest::FamilyGuestContext;

    #[tokio::test]
    async fn family_guest_rejects_non_agy_harness() {
        let db = Arc::new(Database::open_memory().unwrap());
        let tool = HarnessSessionSpawnTool {
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
        assert!(!bounded_by_folders(
            crate::harness_session::CwdBound::Handoff,
            &cfg
        ));
        assert!(!bounded_by_folders(
            crate::harness_session::CwdBound::Any,
            &cfg
        ));
        assert!(bounded_by_folders(
            crate::harness_session::CwdBound::Connector,
            &cfg
        ));
        cfg.handoff_cwd_allow = vec!["  ".into()];
        assert!(!bounded_by_folders(
            crate::harness_session::CwdBound::Handoff,
            &cfg
        ));
        cfg.handoff_cwd_allow = vec!["/srv/work".into()];
        assert!(bounded_by_folders(
            crate::harness_session::CwdBound::Handoff,
            &cfg
        ));
    }
}
