use super::*;
use crate::agent_host::McpAccess;

pub(super) fn build_args(
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
            ResumeStrategy::TokenOrArgs {
                with_token,
                otherwise,
            } => {
                let chosen = if resume_token.is_some() { with_token } else { otherwise };
                chosen
                    .iter()
                    .map(|a| a.replace("{token}", resume_token.unwrap_or_default()))
                    .collect()
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

/// The extra environment a session's agent starts with: its profile's
/// variables plus the session id. Launch and restart both build it here, so a
/// restarted agent gets exactly what the original had.
pub(super) fn launch_env(harness: &Harness, session_id: &str) -> Vec<(String, String)> {
    let profile_env = harness.profile.iter().flat_map(|p| p.env.iter());
    profile_env
        .map(|(k, v)| (k.clone(), v.clone()))
        .chain([(SESSION_ENV.to_string(), session_id.to_string())])
        .collect()
}

/// Starts the agents a restarted built-in host (local or remote) is holding for
/// their env, matching them to the sessions placed on that host.
/// Best effort: one that cannot be resumed stays held and is tried next sweep.
pub fn resume_awaiting(rows: &[HarnessSessionRow], host: &Host) {
    let Ok(waiting) = host.awaiting() else { return };
    for agent in waiting {
        let on_host = |r: &&HarnessSessionRow| r.host == host.name() && r.agent_name == agent.name;
        let Some(row) = rows.iter().find(on_host) else {
            continue;
        };
        let Ok(harness) = resolve(&row.harness) else {
            continue;
        };
        let env = launch_env(&harness, &row.id);
        if let Err(e) = host.resume_awaiting(&agent.name, env) {
            tracing::warn!(session = %row.id, error = %e, "could not resume a held agent");
        }
    }
}

/// How the built-in host restarts this agent after the host itself restarts:
/// only for harnesses that resume without a saved token.
fn restart_args(harness: &Harness, vault_path: &Path, session_id: &str) -> Option<Vec<String>> {
    let tokenless = match harness.spec.resume {
        ResumeStrategy::Args(args) => !args.iter().any(|a| a.contains("{token}")),
        ResumeStrategy::TokenOrArgs { .. } | ResumeStrategy::SessionDir => true,
        ResumeStrategy::None => false,
    };
    tokenless.then(|| build_args(harness, vault_path, session_id, None, true))
}

/// Everything `launch_session` needs beyond the harness spec.
pub(super) struct Launch<'a> {
    pub(super) host: Host,
    pub(super) session_id: &'a str,
    pub(super) cwd: &'a str,
    pub(super) label: &'a str,
    pub(super) prompt: Option<&'a str>,
    pub(super) resume_token: Option<&'a str>,
    pub(super) resuming: bool,
    pub(super) mission_id: Option<&'a str>,
    /// Web chat that launched the session and will watch it.
    pub(super) watch: Option<NewWatch<'a>>,
    pub(super) parent: Option<(&'a str, i64)>,
    pub(super) goal: GoalText<'a>,
}

pub(super) fn workspace_label(harness: &str, label: &str) -> String {
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
pub(super) static LAUNCHES_IN_FLIGHT: std::sync::Mutex<Vec<(usize, String, String)>> = std::sync::Mutex::new(Vec::new());

pub(super) struct InFlight(usize, String, String);

impl InFlight {
    pub(super) fn claim(db: &Arc<Database>, host: &str, agent: &str) -> Result<Self> {
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
pub(super) struct OwnedLaunch {
    host: Host,
    session_id: String,
    cwd: String,
    label: String,
    prompt: Option<String>,
    resume_token: Option<String>,
    resuming: bool,
    mission_id: Option<String>,
    watch: Option<(String, bool, bool)>,
    parent: Option<(String, i64)>,
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
            parent: l.parent.map(|(id, depth)| (id.to_string(), depth)),
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
            parent: self.parent.as_ref().map(|(id, depth)| (id.as_str(), *depth)),
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
/// bounded (`ScriptedHost::launch_bound`), so the caller is answered before its
/// transport times out.
pub(super) async fn launch_session(
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
/// A secret for this session and the endpoint to use it on, when HQ is set up
/// to let launched agents call back and the host can deliver it.
fn mcp_access(db: &Arc<Database>, host: &Host, harness: &Harness, session_id: &str) -> Option<McpAccess> {
    let Some(url) = agent_host_config().agent_mcp_url else {
        // No endpoint now: an older secret of a resumed session must not revive.
        let id = session_id.to_string();
        let _ = db.with_conn(move |c| hq_db::session_tokens::revoke(c, &id));
        return None;
    };
    if !host.accepts_mcp() || harness.spec.kind != "claude" {
        return None;
    }
    let id = session_id.to_string();
    match db.with_conn(move |c| hq_db::session_tokens::mint(c, &id)) {
        Ok(token) => Some(McpAccess { url, token }),
        Err(e) => {
            tracing::warn!(session = %session_id, error = %e, "could not mint a session token");
            None
        }
    }
}

pub(super) async fn run_launch(
    vault_path: &Path,
    db: &Arc<Database>,
    harness: &Harness,
    l: Launch<'_>,
) -> Result<Value> {
    preflight::require_binary(&l.host, harness)?;
    let mcp = mcp_access(db, &l.host, harness, l.session_id);
    let minted = mcp.is_some();
    let session_id = l.session_id.to_string();
    let launched = launch_with(vault_path, db, harness, l, mcp).await;
    if launched.is_err() && minted {
        let _ = db.with_conn(move |c| hq_db::session_tokens::revoke_if_unregistered(c, &session_id));
    }
    launched
}

async fn launch_with(
    vault_path: &Path,
    db: &Arc<Database>,
    harness: &Harness,
    l: Launch<'_>,
    mcp: Option<McpAccess>,
) -> Result<Value> {
    let profile = harness.profile.as_ref();
    let request = LaunchRequest {
        name: l.session_id.to_string(),
        kind: harness.spec.kind.to_string(),
        cwd: l.cwd.to_string(),
        label: workspace_label(&harness.name, l.label),
        env: launch_env(harness, l.session_id),
        args: build_args(
            harness,
            vault_path,
            l.session_id,
            l.resume_token,
            l.resuming,
        ),
        command: profile.and_then(|p| p.command.clone()),
        resume_args: restart_args(harness, vault_path, l.session_id),
        mcp,
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
pub(super) fn record_on_task(db: &Arc<Database>, session_id: &str, event: mission::Event) -> Option<Value> {
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

pub(super) fn record_placement(
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
    let parent = l.parent.map(|(id, depth)| (id.to_string(), depth));
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
        if let Some((parent, depth)) = &parent {
            registry::set_parent(c, name, parent, *depth)?;
        }
        if goal.0.is_some() || goal.1.is_some() {
            registry::set_goal(c, name, goal.0.as_deref(), goal.1.as_deref(), registry::ACTOR_HQ)?;
        }
        if let Some((thread, drive, opted_out)) = &watch {
            start_watch(c, name, NewWatch { thread, drive: *drive, opted_out: *opted_out })?;
        }
        Ok(())
    })
}

pub(super) struct Settled {
    ready: bool,
    status: Option<AgentStatus>,
    /// Screen text when the agent is stuck at a dialog.
    screen: Option<String>,
}

pub(super) fn settle_startup(
    host: &dyn HostBackend,
    name: &str,
    trust_pattern: Option<&str>,
    launched: &Launched,
) -> Result<Settled, AgentHostError> {
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
            "The host saw no activity after the prompt; a fast agent can finish first. Read the output before resending.".into(),
        ),
        PromptOutcome::TimedOut(msg) => Some(format!("prompt wait timed out: {msg}")),
        PromptOutcome::Submitted | PromptOutcome::Settled(_) => None,
    }
}

pub(super) const RESUBMITTED_NOTE: &str = "The host saw no activity after the prompt, so Enter was pressed once more. Read the output to confirm the agent started.";

pub(super) fn launch_report(
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
    /// host name; `None` uses the configured default.
    pub host: Option<&'a str>,
    pub harness: &'a str,
    pub prompt: Option<&'a str>,
    pub cwd: &'a Path,
    pub label: &'a str,
    /// HQ task (id or display id) the session works on.
    pub mission_id: Option<&'a str>,
    /// Web chat that will watch the session.
    pub watch: Option<NewWatch<'a>>,
    /// The session this one is started for, and its depth: recorded in the same
    /// write as the session itself, so no limit can be dodged by a half-made child.
    pub parent: Option<(&'a str, i64)>,
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
            parent: None,
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
    let host = agent_host::host(req.host)?;
    spawn_on(vault_path, db, host, req).await
}

/// `spawn_with` on an already resolved host, so a caller that must know the
/// host first (the handoff) and a test can supply it. The caller has already
/// vetted `req.cwd` against the config it loaded.
pub(crate) async fn spawn_on(
    vault_path: &Path,
    db: &Arc<Database>,
    host: Host,
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
            parent: req.parent,
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
pub(super) fn with_watch_state(db: &Arc<Database>, session_id: &str, mut report: Value) -> Value {
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
