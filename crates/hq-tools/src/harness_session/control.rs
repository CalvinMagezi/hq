use super::*;

pub(super) const MODE_DRIVE: &str = "drive";
pub(super) const MODE_OBSERVE: &str = "observe";

pub(super) const GOAL_NEXT: &str = "HQ only observes this session. Ask the user for a specific goal and an observable definition of done, record them with harness_session_goal, then switch to drive with harness_session_mode.";

pub(super) fn mode_name(drive: bool) -> &'static str {
    if drive { MODE_DRIVE } else { MODE_OBSERVE }
}

pub(super) fn get_row(db:&Arc<Database>, session_id: &str) -> Result<HarnessSessionRow> {
    let id = session_id.to_string();
    db.with_conn(move |c| registry::get(c, &id))?
        .ok_or_else(|| anyhow::anyhow!("no harness session '{session_id}'"))
}

/// The row's host and its agent, or an error naming why they are unavailable.
pub(super) fn locate(row: &HarnessSessionRow) -> Result<(Host, Option<AgentInfo>)> {
    let host = herdr::host(Some(&row.host))?;
    let agent = host.agent(&row.agent_name)?;
    Ok((host, agent))
}

pub(super) fn require_alive(row: &HarnessSessionRow) -> Result<Host> {
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
        ResumeStrategy::TokenOrArgs { .. } | ResumeStrategy::SessionDir => true,
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
pub(super) fn session_view(row: &HarnessSessionRow, live: &Liveness) -> Value {
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

pub(super) fn live_view(row: &HarnessSessionRow, live: &Liveness) -> Value {
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

pub(super) fn set_mode_with(db: &Arc<Database>, row: &HarnessSessionRow, live: &Liveness, req: ModeRequest<'_>) -> Result<Value> {
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
pub(super) fn require_drivable(row: &HarnessSessionRow, live: &Liveness, req: ModeRequest<'_>) -> Result<()> {
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

pub(super) fn agent_state(live: &Liveness) -> Value {
    match live {
        Liveness::Alive(agent) => json!({ "state": "running", "status": agent.status }),
        Liveness::Gone => json!({ "state": "ended" }),
        Liveness::HostUnreachable(detail) => json!({ "state": "unknown", "detail": detail }),
    }
}

/// Bring an agent Herdr already runs, one HQ did not launch in this chat (or
/// at all), under this chat's watch. It starts observation-only: Drive comes
/// later, through the goal and drive gate.
pub fn attach(db: &Arc<Database>, host: &dyn HostBackend, target: &str, thread: &str) -> Result<Value> {
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
pub(super) const LIST_LIMIT: usize = 200;

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

pub(super) fn session_views(rows: &[HarnessSessionRow]) -> Vec<Value> {
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
pub(super) fn tail_log_with(
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

pub(super) static SCREEN_READS: std::sync::LazyLock<coalesce::Coalescer> =
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
pub(super) fn screen_changed(session_id: &str) {
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

pub(super) fn send_text(db: &Arc<Database>, session_id: &str, text: &str) -> Result<Value> {
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
pub(super) fn extract_resume_token(pattern: &str, content: &str) -> Result<Option<String>> {
    let re = regex::Regex::new(pattern)?;
    Ok(re
        .captures_iter(content)
        .last()
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string()))
}

/// Stores the conversation id the agent's hooks reported as the session's
/// resume token when it changed. Returns whether it did.
pub fn record_agent_session_id(
    db: &Arc<Database>,
    row: &HarnessSessionRow,
    agent: &AgentInfo,
) -> Result<bool> {
    let Some(id) = agent.agent_session_id.as_deref().filter(|id| !id.is_empty()) else {
        return Ok(false);
    };
    if row.resume_token.as_deref() == Some(id) {
        return Ok(false);
    }
    let (session, token) = (row.id.clone(), id.to_string());
    db.with_conn(move |c| registry::set_resume_token(c, &session, &token))?;
    Ok(true)
}

/// Once the agent has reported its conversation id, has the host restart it
/// into exactly that conversation instead of the most recent one.
pub fn refresh_restart_command(
    vault_path: &Path,
    host: &Host,
    row: &HarnessSessionRow,
    agent: &AgentInfo,
) -> Result<()> {
    let Some(token) = agent.agent_session_id.as_deref() else {
        return Ok(());
    };
    let harness = resolve(&row.harness)?;
    let args = build_args(&harness, vault_path, &row.id, Some(token), true);
    host.update_resume(&row.agent_name, &agent.kind, args)?;
    Ok(())
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
