//! One call that turns a request from outside HQ (an MCP client) into tracked
//! work: an HQ task, a coding-agent session linked to it, and a web chat
//! thread that owns the session so its finished turns, blocks and Drive land in
//! the UI instead of the relay. Calling it again for the same work returns what
//! exists rather than starting a second session.

use anyhow::{Result, anyhow, bail};
use hq_db::Database;
use hq_db::harness_sessions_registry::{self as registry, HarnessSessionRow};
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

use super::{GoalText, Liveness, NewWatch, SpawnRequest};
use crate::agent_host::{Host, HostBackend};
use crate::tasks::{Placement, create_task_in};
use crate::util::generate_id;

const CREATED_BY: &str = "mcp-handoff";
const DEFAULT_SPACE: &str = "personal";
const DEFAULT_INITIATIVE: &str = "Inbox";
const ACCEPTANCE_HEADING: &str = "Acceptance criteria:";

/// Two handoffs for the same work must not both find "no session yet" and each
/// start one, so the check and the launch run under one lock per process.
static HANDOFF_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// What the caller asked for. Blank strings count as absent. Owned, because the
/// handoff runs on its own task (see `handoff`).
#[derive(Debug, Clone, Default)]
pub struct HandoffRequest {
    pub title: String,
    pub description: String,
    /// Observable conditions that show the work is done.
    pub acceptance: String,
    pub harness: String,
    pub cwd: String,
    /// Idempotency key for the task, unique per space.
    pub external_id: String,
    /// An existing task to work on instead of filing one.
    pub task_id: String,
    pub prompt: String,
    pub space_id: String,
    pub initiative: String,
    /// Whether a session this handoff starts is driven by HQ when its goal
    /// allows it (`agent_host.drive_new_watches`, unless the caller opts out).
    pub drive_new: bool,
    pub drive_opted_out: bool,
}

fn non_blank(s: &str) -> Option<&str> {
    Some(s.trim()).filter(|s| !s.is_empty())
}

fn task_description(description: &str, acceptance: &str) -> String {
    match (non_blank(description), non_blank(acceptance)) {
        (Some(d), Some(a)) => format!("{d}\n\n{ACCEPTANCE_HEADING}\n{a}"),
        (Some(d), None) => d.to_string(),
        (None, Some(a)) => format!("{ACCEPTANCE_HEADING}\n{a}"),
        (None, None) => String::new(),
    }
}

/// What the agent is told when the caller wrote no prompt of their own.
fn default_prompt(task: &t::Task, acceptance: &str) -> String {
    let mut prompt = format!("Work on HQ task {}: {}", task.display_id, task.title);
    if let Some(d) = non_blank(&task.description) {
        prompt.push_str(&format!("\n\n{d}"));
    }
    if let Some(a) = non_blank(acceptance).filter(|a| !task.description.contains(a)) {
        prompt.push_str(&format!("\n\n{ACCEPTANCE_HEADING}\n{a}"));
    }
    prompt.push_str(
        "\n\nWhen you finish, stop and summarize what you changed and how you checked it.",
    );
    prompt
}

fn links(thread: Option<&str>, task_id: &str) -> Value {
    json!({
        "chat": thread.map(|t| format!("/chat?thread={t}")),
        "task": format!("/tasks?task={task_id}"),
    })
}

fn task_view(task: &t::Task, created: bool) -> Value {
    json!({
        "id": task.id,
        "display_id": task.display_id,
        "title": task.title,
        "status": task.status,
        "created": created,
        "deduplicated": !created,
    })
}

/// The task this handoff is about: the one named, the one an earlier call with
/// the same `external_id` filed, or a new one. The flag says it was just created.
fn resolve_or_file_task(db: &Arc<Database>, req: &HandoffRequest) -> Result<(t::Task, bool)> {
    let external = non_blank(&req.external_id);
    if let Some(existing) = non_blank(&req.task_id) {
        if external.is_some() {
            bail!(
                "pass task_id to work on an existing task or external_id to file one idempotently, not both"
            );
        }
        return db.with_conn(|c| {
            let id = super::mission::resolve_task(c, existing)?;
            let task = t::get_task(c, &id)?.ok_or_else(|| anyhow!("task {id} vanished"))?;
            Ok((task, false))
        });
    }
    let Some(title) = non_blank(&req.title) else {
        bail!("title is required (or pass task_id to work on an existing task)");
    };
    let description = task_description(&req.description, &req.acceptance);
    let (space, initiative) = (
        non_blank(&req.space_id).unwrap_or(DEFAULT_SPACE),
        non_blank(&req.initiative).unwrap_or(DEFAULT_INITIATIVE),
    );
    db.with_conn(|c| {
        let (task, created) = create_task_in(
            c,
            &generate_id("tk"),
            None,
            &Placement {
                space_id: space,
                folder_name: None,
                initiative_name: initiative,
            },
            &t::NewTask {
                title,
                description: &description,
                created_by: CREATED_BY,
                external_id: external,
                ..Default::default()
            },
        )?;
        if !created {
            super::mission::resolve_task(c, &task.id)?;
        }
        Ok((task, created))
    })
}

/// A session already working on the task: alive, or on a host that cannot be
/// asked right now. A session marked running that its host no longer has is
/// stale and does not count.
fn live_session_for(
    db: &Arc<Database>,
    host: &Host,
    task_id: &str,
) -> Result<Option<(HarnessSessionRow, Liveness)>> {
    let rows: Vec<HarnessSessionRow> = db
        .with_conn(|c| registry::list_for_mission(c, task_id))?
        .into_iter()
        .filter(|r| r.status == registry::STATUS_RUNNING)
        .collect();
    let polled = super::poll_hosts_with(&rows, |name| {
        if name == host.name() {
            Ok(host.clone())
        } else {
            crate::agent_host::host(Some(name))
        }
    });
    Ok(rows
        .into_iter()
        .find_map(|row| match super::liveness(&polled, &row) {
            Liveness::Gone => None,
            live => Some((row, live)),
        }))
}

/// Archive a thread nothing came to own, unless a session row already took it.
fn drop_unused_thread(db: &Arc<Database>, thread: &str) {
    let owned = db
        .with_conn(|c| registry::list_for_thread(c, thread))
        .map(|rows| !rows.is_empty())
        .unwrap_or(true);
    if !owned {
        let _ = db.with_conn(|c| hq_db::chat::archive_thread(c, thread));
    }
}

/// Session ids a thread already owns, so a failed call can tell "nothing was
/// started" from "a session exists but the call failed after creating it".
fn sessions_of_thread(db: &Arc<Database>, thread: &str) -> Vec<String> {
    db.with_conn(|c| registry::list_for_thread(c, thread))
        .map(|rows| rows.into_iter().map(|r| r.id).collect())
        .unwrap_or_default()
}

fn existing_report(
    task: &t::Task,
    created: bool,
    (row, live): (HarnessSessionRow, Liveness),
) -> Value {
    let unreachable = matches!(live, Liveness::HostUnreachable(_));
    let mut report = json!({
        "handoff": "existing_session",
        "note": "A session is already working on this task, so no second one was started.",
        "task": task_view(task, created),
        "session_id": row.id,
        "thread_id": row.owner_thread,
        "host": row.host,
        "links": links(row.owner_thread.as_deref(), &task.id),
        "session": super::session_view(&row, &live),
    });
    if unreachable {
        report["note"] = json!(
            "A session is recorded as running on this task but its host cannot be reached right now, so no second one was started. Its state is unknown, not confirmed running."
        );
    }
    if row.owner_thread.is_none() {
        report["thread_note"] =
            json!("No web chat thread owns this session; it was started outside a handoff.");
    }
    report
}

/// File (or find) the task, then start one session for it under a new chat
/// thread. Everything that can fail before a session exists is checked first.
/// The caller vets `req.cwd` against the config (`require_cwd_in`) before calling.
/// A failure after the task exists leaves it in place and says so, so a retry
/// with the same `external_id` picks it up.
///
/// The whole handoff runs on its own task: an MCP client that disconnects
/// mid-launch drops this future, and the thread, task and process-wide lock
/// must still be settled. The launch itself is bounded below the client's
/// transport limit (`ScriptedHost::launch_bound`), so a caller normally gets the
/// real error rather than a timeout.
pub async fn handoff(
    vault_path: &Path,
    db: &Arc<Database>,
    host: Host,
    req: HandoffRequest,
) -> Result<Value> {
    let (vault_path, db) = (vault_path.to_path_buf(), db.clone());
    tokio::spawn(async move { run_handoff(&vault_path, &db, host, req).await })
        .await
        .map_err(|e| anyhow!("the handoff task did not finish: {e}"))?
}

async fn run_handoff(
    vault_path: &Path,
    db: &Arc<Database>,
    host: Host,
    req: HandoffRequest,
) -> Result<Value> {
    super::require_cwd(Some(&req.cwd))?;
    super::resolve(&req.harness)?;
    let _guard = HANDOFF_LOCK.lock().await;

    let (task, created) = resolve_or_file_task(db, &req)?;
    let task_ref = format!(
        "{} ({})",
        task.display_id,
        if created { "filed" } else { "reused" }
    );

    let tid = task.id.clone();
    let (db2, host2) = (db.clone(), host.clone());
    let found = crate::agent_host::blocking(move || live_session_for(&db2, &host2, &tid)).await??;
    if let Some(live) = found {
        return Ok(existing_report(&task, created, live));
    }

    let title = format!("{} {}", task.display_id, task.title);
    let thread = db
        .with_conn(|c| hq_db::chat::create_thread(c, &title, "user", "user"))?
        .thread_id;
    let prompt = non_blank(&req.prompt)
        .map_or_else(|| default_prompt(&task, &req.acceptance), str::to_string);
    let spawned = super::spawn_on(
        vault_path,
        db,
        host.clone(),
        SpawnRequest {
            host: Some(host.name()),
            harness: &req.harness,
            prompt: Some(&prompt),
            cwd: Path::new(req.cwd.trim()),
            label: &task.display_id,
            mission_id: Some(&task.id),
            watch: Some(NewWatch {
                thread: &thread,
                drive: req.drive_new && !req.drive_opted_out,
                opted_out: req.drive_opted_out,
            }),
            parent: None,
            goal: GoalText {
                goal: None,
                done_criteria: non_blank(&req.acceptance),
            },
        },
    )
    .await;
    let mut session = match spawned {
        Ok(session) => session,
        Err(e) => return Err(launch_failure(db, &thread, &task_ref, &e)),
    };

    let blocked = session.get("blocked").is_some();
    let current = db.with_conn(|c| t::get_task(c, &task.id))?.unwrap_or(task);
    let mut report = json!({
        "handoff": if blocked { "blocked_at_dialog" } else { "started" },
        "task": task_view(&current, created),
        "session_id": session["session_id"],
        "thread_id": thread,
        "host": session["host"],
        "links": links(Some(&thread), &current.id),
    });
    if blocked {
        report["warning"] = json!(
            "The agent stopped at a dialog before it reached its prompt, so the prompt was NOT typed and no work is running. The task is marked blocked. Read session.blocked.screen and answer with harness_session_send keys."
        );
    }
    if let Some(obj) = session.as_object_mut() {
        obj.remove("task");
    }
    report["session"] = session;
    Ok(report)
}

/// Settle what a failed launch left behind and say what is true. Usually
/// nothing was started: the thread is archived; the task was never moved, because only a recorded launch starts it. If
/// a session row exists anyway (the failure came after it was recorded) the
/// thread stays with it and the error says so.
fn launch_failure(
    db: &Arc<Database>,
    thread: &str,
    task_ref: &str,
    cause: &anyhow::Error,
) -> anyhow::Error {
    let sessions = sessions_of_thread(db, thread);
    if !sessions.is_empty() {
        return anyhow!(
            "session {} was started for task {task_ref} but the call failed afterwards: {cause:#}. The session is tracked; check it with harness_session_status before retrying.",
            sessions.join(", ")
        );
    }
    drop_unused_thread(db, thread);
    anyhow!(
        "no session was started for task {task_ref}: {cause:#}. Retry the same call to reuse the task; nothing has run."
    )
}
