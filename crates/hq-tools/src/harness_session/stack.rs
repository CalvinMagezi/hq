//! The Agentic Stack (`stack:` in the config): named general-purpose agents
//! that pull the tasks assigned to them and run each one as a single bounded
//! job in the host. Assignment is the trigger, the harness's one-shot command
//! is the run, and the process ending is the completion signal (the
//! supervisor records `RunEnded`, which puts the task into review). A person
//! signs off; HQ never completes a task. See `docs/plans/agentic-stack.md`.

use super::category::{HarnessCategory, Purpose};
use super::launch::{Launch, launch_session};
use super::{GoalText, Harness};
use crate::agent_host::Host;
use anyhow::{Result, bail};
use hq_core::config::{HqConfig, Specialist};
use hq_db::Database;
use hq_db::harness_sessions_registry as registry;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a task is left alone after a run for it could not start.
pub const LAUNCH_BACKOFF: Duration = Duration::from_secs(30 * 60);
/// The rolling window the run budget is counted over.
const BUDGET_WINDOW_MINUTES: i64 = 24 * 60;
/// Longest task description carried into a brief.
const MAX_BRIEF_DESCRIPTION: usize = 4000;
/// Author of the comments the dispatcher leaves.
const COMMENT_AUTHOR: &str = "stack";

/// Resolves a host by name; `None` is the default host.
pub type HostResolver<'a> = &'a (dyn Fn(Option<&str>) -> Result<Host> + Sync);

/// Tasks whose last launch failed, and when: tried again after `LAUNCH_BACKOFF`.
static BACKOFF: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// How far down a specialist's queue one sweep looks for a task that is not
/// backed off, so one task that cannot start never holds up the rest.
const QUEUE_LOOKAHEAD: usize = 25;

fn backed_off(task_id: &str) -> bool {
    let guard = BACKOFF.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .as_ref()
        .and_then(|m| m.get(task_id))
        .is_some_and(|at| at.elapsed() < LAUNCH_BACKOFF)
}

pub(super) fn back_off(task_id: &str) {
    let mut guard = BACKOFF.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(task_id.to_string(), Instant::now());
}

/// What one sweep did.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct DispatchReport {
    /// Launch reports of the runs started.
    pub started: Vec<Value>,
    /// One line per specialist that started nothing, saying why.
    pub skipped: Vec<String>,
}

/// A specialist's lane: its harness resolved and its purposes checked.
#[derive(Debug)]
pub struct Lane {
    pub harness: Harness,
    pub purposes: Vec<Purpose>,
}

/// Resolve a specialist's harness and purposes, refusing a coding harness and
/// the `code` purpose: the stack is for work that never touches code.
pub fn lane_for(cfg: &HqConfig, s: &Specialist) -> Result<Lane> {
    let harness = super::resolve_in(&cfg.agent_host, &s.harness)?;
    if harness.category() != HarnessCategory::GeneralPurpose {
        bail!(
            "specialist '{}' names '{}', a coding agent; the stack takes only general-purpose agents ({})",
            s.name,
            s.harness,
            super::names_in(HarnessCategory::GeneralPurpose).join(", ")
        );
    }
    if harness.one_shot_args("").is_none() {
        bail!(
            "specialist '{}': '{}' has no one-shot command; give its profile `one_shot_args`",
            s.name,
            s.harness
        );
    }
    let mut purposes = Vec::new();
    for text in &s.purposes {
        let p = Purpose::parse(text)?;
        harness.category().check(&harness.name, p)?;
        if !purposes.contains(&p) {
            purposes.push(p);
        }
    }
    if purposes.is_empty() {
        purposes.push(Purpose::General);
    }
    Ok(Lane { harness, purposes })
}

/// The purpose a run for `task` gets: a tag naming one the lane takes, else the
/// lane's first.
pub fn purpose_for_task(task: &t::Task, lane: &[Purpose]) -> Purpose {
    task.tags
        .iter()
        .filter_map(|tag| Purpose::parse(tag).ok())
        .find(|p| lane.contains(p))
        .or_else(|| lane.first().copied())
        .unwrap_or(Purpose::General)
}

/// The text a run starts with: the task, where it sits, where the last run left
/// off, and what to print at the end. Plain text, since it travels as one argument.
pub fn brief_for(
    specialist: &str,
    purpose: Purpose,
    task: &t::Task,
    initiative: Option<&str>,
    resume: Option<&t::TaskCheckpoint>,
) -> String {
    let mut out = format!(
        "You are '{specialist}', a specialist in this HQ's agentic stack. This is one run for one task; do the work, print your report, and exit. Purpose: {}.\n\n",
        purpose.as_str()
    );
    out.push_str(&format!("Task {}: {}\n", task.display_id, task.title));
    if let Some(name) = initiative {
        out.push_str(&format!("Initiative: {name}\n"));
    }
    if let Some(due) = &task.due_date {
        out.push_str(&format!("Due: {due}\n"));
    }
    let description = task.description.trim();
    if !description.is_empty() {
        out.push_str("\nBrief and definition of done:\n");
        out.push_str(hq_core::text::truncate_chars(
            description,
            MAX_BRIEF_DESCRIPTION,
        ));
        out.push('\n');
    }
    if let Some(cp) = resume {
        out.push_str("\nWhere the last run left off:\n");
        if !cp.summary.is_empty() {
            out.push_str(&format!("- Summary: {}\n", cp.summary));
        }
        if !cp.next_step.is_empty() {
            out.push_str(&format!("- Next step: {}\n", cp.next_step));
        }
        if !cp.open_questions.is_empty() {
            out.push_str(&format!("- Open questions: {}\n", cp.open_questions));
        }
    }
    out.push_str(
        "\nRules: this is not code work; change no repository. Send no messages and take no action outside this task. \
         Finish by printing a section headed REPORT with what you did, what you found, what remains, and your open questions. \
         A person reviews it on the task before anything is marked done.\n",
    );
    out
}

/// One sweep: for every enabled specialist, start a run for its next task when
/// nothing of its is running, its budget has room, and a task is ready. A spent
/// budget and a running run are nothing to do, not a problem to report; a
/// specialist that cannot be resolved or a run that could not start is one line
/// in `skipped`.
pub async fn dispatch(
    vault_path: &Path,
    db: &Arc<Database>,
    cfg: &HqConfig,
    resolve: HostResolver<'_>,
) -> Result<DispatchReport> {
    let mut report = DispatchReport::default();
    for problem in cfg.stack.problems() {
        report.skipped.push(problem);
    }
    for s in cfg
        .stack
        .specialists
        .iter()
        .filter(|s| s.enabled && Specialist::valid_name(&s.name))
    {
        match dispatch_one(vault_path, db, cfg, s, resolve).await {
            Ok(Some(launched)) => report.started.push(launched),
            Ok(None) => {}
            Err(e) => report.skipped.push(format!("{}: {e:#}", s.name)),
        }
    }
    Ok(report)
}

/// The sweep for one specialist. `Ok(None)` means nothing to do this time.
async fn dispatch_one(
    vault_path: &Path,
    db: &Arc<Database>,
    cfg: &HqConfig,
    s: &Specialist,
    resolve: HostResolver<'_>,
) -> Result<Option<Value>> {
    // Before the lane: a run past its limit is stopped even when the
    // specialist's config has stopped resolving since it started.
    if stop_overrunning(db, s)? {
        return Ok(None);
    }
    let lane = lane_for(cfg, s)?;
    let label = s.label();
    let running = db.with_conn({
        let label = label.clone();
        move |c| registry::running_with_label(c, registry::ORIGIN_STACK, &label)
    })?;
    if !running.is_empty() {
        return Ok(None);
    }
    let started_today = db.with_conn({
        let label = label.clone();
        move |c| {
            registry::count_started_since(c, registry::ORIGIN_STACK, &label, BUDGET_WINDOW_MINUTES)
        }
    })?;
    if started_today >= s.runs_per_day() as i64 {
        tracing::debug!(specialist = %s.name, started_today, budget = s.runs_per_day(), "stack: run budget spent");
        return Ok(None);
    }
    let name = s.name.clone();
    let no_code = lane.harness.category() == HarnessCategory::GeneralPurpose;
    let next = db.with_conn(move |c| {
        t::next_tasks(
            c,
            &t::NextTaskQuery {
                assignee: &name,
                include_unassigned: false,
                initiative_id: None,
                tag: None,
                no_code_work: no_code,
            },
            QUEUE_LOOKAHEAD,
        )
    })?;
    let Some(task) = next.into_iter().find(|task| !backed_off(&task.id)) else {
        return Ok(None);
    };
    start_run(vault_path, db, cfg, s, &lane, &task, resolve)
        .await
        .map(Some)
}

/// `hq stack run <name> <task>`: start a run for one task now, whoever it is
/// assigned to. The lane, the budget and the one-run-at-a-time rule still hold.
pub async fn run_for(
    vault_path: &Path,
    db: &Arc<Database>,
    cfg: &HqConfig,
    name: &str,
    task_ref: &str,
    resolve: HostResolver<'_>,
) -> Result<Value> {
    let Some(s) = cfg.stack.get(name) else {
        let known: Vec<&str> = cfg
            .stack
            .specialists
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        bail!(
            "no specialist '{name}' in stack.specialists (known: {})",
            known.join(", ")
        );
    };
    let lane = lane_for(cfg, s)?;
    let label = s.label();
    let running = db.with_conn({
        let label = label.clone();
        move |c| registry::running_with_label(c, registry::ORIGIN_STACK, &label)
    })?;
    if let Some(run) = running.first() {
        bail!(
            "'{name}' is already running session {} ; one run at a time",
            run.id
        );
    }
    let started = db.with_conn({
        let label = label.clone();
        move |c| {
            registry::count_started_since(c, registry::ORIGIN_STACK, &label, BUDGET_WINDOW_MINUTES)
        }
    })?;
    if started >= s.runs_per_day() as i64 {
        bail!(
            "'{name}' has started {started} runs in the last 24h, its budget (runs_per_day {})",
            s.runs_per_day()
        );
    }
    let wanted = task_ref.to_string();
    let task = db
        .with_conn(move |c| t::get_task(c, &wanted))?
        .ok_or_else(|| anyhow::anyhow!("no task '{task_ref}'"))?;
    let task_id = task.id.clone();
    if lane.harness.category() == HarnessCategory::GeneralPurpose
        && db.with_conn(move |c| t::is_code_work(c, &task_id))?
    {
        bail!(
            "task {} is code work (tagged '{}' or linked to a commit or pull request); a general-purpose specialist never takes it",
            task.display_id,
            t::CODE_TAG
        );
    }
    start_run(vault_path, db, cfg, s, &lane, &task, resolve).await
}

/// Launch the run and leave the task a comment when it could not start.
async fn start_run(
    vault_path: &Path,
    db: &Arc<Database>,
    cfg: &HqConfig,
    s: &Specialist,
    lane: &Lane,
    task: &t::Task,
    resolve: HostResolver<'_>,
) -> Result<Value> {
    let purpose = purpose_for_task(task, &lane.purposes);
    let (initiative, resume) = db.with_conn({
        let (init_id, task_id) = (task.initiative_id.clone(), task.id.clone());
        move |c| {
            let name = t::get_initiative(c, &init_id)?.map(|i| i.name);
            Ok((name, t::latest_checkpoint(c, &task_id)?))
        }
    })?;
    let brief = brief_for(
        &s.name,
        purpose,
        task,
        initiative.as_deref(),
        resume.as_ref(),
    );
    let launched = async {
        let host = resolve(s.host.as_deref())?;
        let cwd = run_dir(&*host, cfg, s)?;
        super::check_cwd_outside_vault(
            Path::new(&cwd),
            host.name(),
            &cfg.agent_host.default_host,
            &cfg.vault_path,
        )?;
        let session_id = super::new_session_id(&lane.harness.name);
        let label = s.label();
        let report = launch_session(
            vault_path,
            db,
            &lane.harness,
            Launch {
                host,
                session_id: &session_id,
                cwd: &cwd,
                label: &label,
                prompt: None,
                resume_token: None,
                resuming: false,
                mission_id: Some(&task.id),
                watch: None,
                parent: None,
                goal: GoalText {
                    goal: Some(&task.title),
                    done_criteria: None,
                },
                purpose,
                observe: &[],
                one_shot: Some(&brief),
                // In the row's own write: a run under the default origin would
                // escape the one-at-a-time rule and the budget.
                origin: Some(registry::ORIGIN_STACK),
            },
        )
        .await?;
        anyhow::Ok(report)
    }
    .await;
    match launched {
        Ok(mut report) => {
            report["specialist"] = json!(s.name);
            report["run"] = json!({ "purpose": purpose.as_str(), "run_minutes": s.run_minutes() });
            tracing::info!(specialist = %s.name, task = %task.display_id, "stack: run started");
            Ok(report)
        }
        Err(e) => {
            back_off(&task.id);
            let body = format!(
                "A run by stack specialist '{}' could not start: {e:#}. HQ tries this task again in {} minutes; fix the cause or assign the task elsewhere.",
                s.name,
                LAUNCH_BACKOFF.as_secs() / 60
            );
            let id = task.id.clone();
            let _ = db.with_conn(move |c| t::add_comment(c, &id, COMMENT_AUTHOR, &body, None));
            Err(e)
        }
    }
}

/// The folder a specialist's runs start in: the configured one, else
/// `stack-<name>` in the host's HQ folder, made there when missing.
fn run_dir(
    host: &dyn crate::agent_host::HostBackend,
    cfg: &HqConfig,
    s: &Specialist,
) -> Result<String> {
    if let Some(cwd) = s.cwd.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        let path = super::require_cwd_in(Some(cwd), &cfg.agent_host, false)?;
        return Ok(path.to_string_lossy().into_owned());
    }
    let made = host.make_dir(None, &format!("stack-{}", s.name)).map_err(|e| {
        anyhow::anyhow!("no cwd is configured and host '{}' could not make a folder for the run ({e}); set stack.specialists[].cwd", host.name())
    })?;
    let path = made["path"].as_str().ok_or_else(|| {
        anyhow::anyhow!("host '{}' did not say which folder it made", host.name())
    })?;
    super::require_cwd_in(Some(path), &cfg.agent_host, false)?;
    Ok(path.to_string())
}

/// Stops a run older than `run_minutes` and blocks its task with the reason.
/// True when something was stopped, so the sweep starts nothing else this time.
fn stop_overrunning(db: &Arc<Database>, s: &Specialist) -> Result<bool> {
    let label = s.label();
    let running =
        db.with_conn(move |c| registry::running_with_label(c, registry::ORIGIN_STACK, &label))?;
    let limit = chrono::Duration::minutes(s.run_minutes() as i64);
    let mut stopped = false;
    for row in running {
        let Ok(started) =
            chrono::NaiveDateTime::parse_from_str(&row.created_at, "%Y-%m-%d %H:%M:%S")
        else {
            continue;
        };
        if chrono::Utc::now().naive_utc() - started < limit {
            continue;
        }
        tracing::warn!(specialist = %s.name, session = %row.id, "stack: run past its time limit, stopping it");
        if let Err(e) = super::stop(db, &row.id) {
            tracing::warn!(session = %row.id, error = %e, "stack: could not stop an overrunning run");
            continue;
        }
        stopped = true;
        if let Some(task_id) = row.mission_id.clone() {
            let reason = format!(
                "stack run by '{}' exceeded {} minutes and was stopped",
                s.name,
                s.run_minutes()
            );
            let _ = db.with_conn(move |c| {
                let patch = t::TaskPatch {
                    status: Some(t::STATUS_BLOCKED.into()),
                    blocked_reason: Some(Some(reason.clone())),
                    ..Default::default()
                };
                let _ = t::update_task(c, &task_id, &patch, Some(t::STATUS_IN_PROGRESS));
                let body = format!(
                    "{reason}. Its last screen is in `harness_session_logs` for session `{}`.",
                    row.id
                );
                t::add_comment(c, &task_id, COMMENT_AUTHOR, &body, None).map(|_| ())
            });
        }
    }
    Ok(stopped)
}

/// What `hq stack list` shows: each specialist with its lane, what is running
/// and the day's count.
pub fn status(db: &Arc<Database>, cfg: &HqConfig) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for s in &cfg.stack.specialists {
        let label = s.label();
        let (running, started, queued) = db.with_conn({
            let (label, name) = (label.clone(), s.name.clone());
            move |c| {
                let running: Vec<String> =
                    registry::running_with_label(c, registry::ORIGIN_STACK, &label)?
                        .into_iter()
                        .map(|r| r.id)
                        .collect();
                let started = registry::count_started_since(
                    c,
                    registry::ORIGIN_STACK,
                    &label,
                    BUDGET_WINDOW_MINUTES,
                )?;
                let filter = t::TaskFilter {
                    assignee: Some(name),
                    status: Some(t::STATUS_TO_DO.into()),
                    ..Default::default()
                };
                Ok((running, started, t::count_tasks(c, &filter)?))
            }
        })?;
        let (lane, problem) = match lane_for(cfg, s) {
            Ok(lane) => (
                lane.purposes.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
                None,
            ),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        out.push(json!({
            "name": s.name,
            "harness": s.harness,
            "purposes": lane,
            "host": s.host,
            "cwd": s.cwd,
            "enabled": s.enabled,
            "run_minutes": s.run_minutes(),
            "runs_per_day": s.runs_per_day(),
            "runs_last_24h": started,
            "running": running,
            "queued": queued,
            "problem": problem,
        }));
    }
    Ok(out)
}
