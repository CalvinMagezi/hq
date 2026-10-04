//! Ties one plan's execution to the durable run registry: registers every
//! child before dispatch, tracks liveness, and settles each child exactly
//! once with an execution status and a separate acceptance verdict.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hq_db::Database;
use hq_db::subagent_runs::{self as runs, NewRun, Settlement};
use tracing::warn;

use super::types::{
    ChildCompletionEvent, ChildExecContext, ChildOutcome, ChildPlan, ChildStatus, CompletionSink,
    RunInfo,
};

const BLOCKER_LINE_MAX: usize = 240;
const CANCEL_POLL: Duration = Duration::from_secs(5);
const TOUCH_INTERVAL_SECS: i64 = 5;
const MISSING_PREFIXES: [&str; 3] = ["missing:", "deliverable_missing:", "not written"];
const BLOCKER_PREFIXES: [&str; 5] = [
    "blocker:",
    "blocked:",
    "not done",
    "unable to complete",
    "cannot complete",
];
const BLOCKER_PHRASES: [&str; 8] = [
    "do not have access to",
    "don't have access to",
    "lack the tool",
    "lacked the tool",
    "no tool available",
    "tool is unavailable",
    "was not able to",
    "could not be completed",
];

/// Appended to every child's context so unfinished work is reported in a
/// form the registry can read, not buried in prose.
pub(super) const REPORTING_PROTOCOL: &str = "Reporting protocol: end your reply with one line `BLOCKER: <what stopped you>` for anything you could not do, and one line `MISSING: <deliverable>` for each requested deliverable you did not produce. Never describe unfinished work as complete.";

pub(super) fn exec_status_name(status: ChildStatus) -> &'static str {
    match status {
        ChildStatus::Completed => runs::EXEC_COMPLETED,
        ChildStatus::Failed => runs::EXEC_FAILED,
        ChildStatus::TimedOut => runs::EXEC_TIMED_OUT,
        ChildStatus::Blocked => runs::EXEC_BLOCKED,
        ChildStatus::Rejected => runs::EXEC_REJECTED,
        ChildStatus::Cancelled => runs::EXEC_CANCELLED,
    }
}

pub(super) struct Assessment {
    pub accept: &'static str,
    pub missing: Vec<String>,
    pub blocker: Option<String>,
    pub next_action: String,
}

fn clip(line: &str) -> String {
    runs::preview(line.trim(), BLOCKER_LINE_MAX)
}

/// Pull blocker and missing-deliverable markers out of a child's output.
fn scan_markers(output: &str) -> (Option<String>, Vec<String>) {
    let mut blocker: Option<String> = None;
    let mut missing: Vec<String> = Vec::new();
    for raw in output.lines() {
        let line = raw
            .trim()
            .trim_start_matches(['-', '*', '>', '#', '`', ' ']);
        let lower = line.to_lowercase();
        if MISSING_PREFIXES.iter().any(|p| lower.starts_with(p)) {
            let item = line.split_once(':').map_or(line, |(_, rest)| rest).trim();
            let item = clip(if item.is_empty() { line } else { item });
            if !missing.contains(&item) {
                missing.push(item);
            }
            blocker.get_or_insert_with(|| clip(line));
        } else if BLOCKER_PREFIXES.iter().any(|p| lower.starts_with(p))
            || BLOCKER_PHRASES.iter().any(|p| lower.contains(p))
        {
            blocker.get_or_insert_with(|| clip(line));
        }
    }
    (blocker, missing)
}

/// Execution success is not acceptance: a clean exit stays `unverified`, and
/// one that says it left work undone is `partial`. Only the parent reviewing
/// the result may raise it to `accepted`.
pub(super) fn assess(outcome: &ChildOutcome) -> Assessment {
    let (marker_blocker, missing) = scan_markers(&outcome.output);
    let has_output = !outcome.output.trim().is_empty();
    let accept = match outcome.status {
        ChildStatus::Completed if marker_blocker.is_some() || !missing.is_empty() => {
            runs::ACCEPT_PARTIAL
        }
        ChildStatus::Completed => runs::ACCEPT_UNVERIFIED,
        ChildStatus::Failed | ChildStatus::TimedOut if has_output => runs::ACCEPT_PARTIAL,
        _ => runs::ACCEPT_BLOCKED,
    };
    let blocker = marker_blocker.or_else(|| outcome.error.as_deref().map(clip));
    let next_action = if accept == runs::ACCEPT_UNVERIFIED {
        "verify the deliverable against the success criteria, then record a verdict with subagent_run_review"
    } else {
        "read the full result with subagent_run_result, check what was actually changed, then continue within the original goal or escalate the blocker"
    };
    Assessment {
        accept,
        missing,
        blocker,
        next_action: next_action.to_string(),
    }
}

/// Stops the cancel poller when the child finishes.
pub(super) struct CancelWatch(tokio::task::JoinHandle<()>);

impl Drop for CancelWatch {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Entry {
    run_id: String,
    role: String,
}

pub(super) struct Ledger {
    db: Option<Arc<Database>>,
    detached: bool,
    sink: Option<CompletionSink>,
    parent_turn_id: Option<String>,
    entries: HashMap<String, Entry>,
    emitted: Mutex<HashSet<String>>,
    last_touch: Mutex<HashMap<String, i64>>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Ledger {
    /// Register every child of the plan, before anything is dispatched or
    /// acknowledged. A registry failure is logged and the plan still runs:
    /// losing durability must not lose the work itself.
    pub(super) fn open(
        db: Option<Arc<Database>>,
        plan: &ChildPlan,
        ctx: &ChildExecContext,
        detached: bool,
    ) -> Arc<Self> {
        let mut entries = HashMap::new();
        for child in &plan.children {
            entries.insert(
                child.id.clone(),
                Entry {
                    run_id: uuid::Uuid::new_v4().to_string(),
                    role: child.role().to_string(),
                },
            );
        }
        if let Some(db) = &db {
            let result = db.with_conn(|c| {
                let depth = runs::followup_depth_for(c, ctx.parent_turn_id.as_deref())?;
                for child in &plan.children {
                    let run = NewRun {
                        run_id: entries[&child.id].run_id.clone(),
                        parent_run_id: ctx.parent_run_id.clone(),
                        parent_turn_id: ctx.parent_turn_id.clone(),
                        origin: ctx.origin.clone(),
                        task_id: child.task_id.clone(),
                        child_id: child.id.clone(),
                        role: child.role().to_string(),
                        goal: child.goal.clone(),
                        success_criteria: child.success_criteria.clone(),
                        required_tools: child.required_tools.clone(),
                        detached,
                        owner_pid: Some(i64::from(std::process::id())),
                        followup_depth: depth,
                    };
                    runs::insert_run(c, &run, now())?;
                }
                Ok(())
            });
            if let Err(e) = result {
                warn!(error = %e, "subagent runs: could not register plan");
            }
        }
        Arc::new(Self {
            db,
            detached,
            sink: ctx.completion_sink.clone(),
            parent_turn_id: ctx.parent_turn_id.clone(),
            entries,
            emitted: Mutex::new(HashSet::new()),
            last_touch: Mutex::new(HashMap::new()),
        })
    }

    /// Whether the run was cancelled in the registry before it started.
    pub(super) fn is_cancelled(&self, child_id: &str) -> bool {
        let (Some(db), Some(entry)) = (&self.db, self.entries.get(child_id)) else {
            return false;
        };
        db.with_conn(|c| runs::get(c, &entry.run_id))
            .ok()
            .flatten()
            .is_some_and(|r| r.exec_status == runs::EXEC_CANCELLED)
    }

    /// Poll the registry while the child runs and trip its cancel flag when
    /// the run is cancelled there. Stops when the returned guard is dropped.
    pub(super) fn watch_cancel(&self, child_id: &str, cancel: Arc<AtomicBool>) -> Option<CancelWatch> {
        let db = self.db.clone()?;
        let run_id = self.entries.get(child_id)?.run_id.clone();
        Some(CancelWatch(tokio::spawn(async move {
            loop {
                tokio::time::sleep(CANCEL_POLL).await;
                let row = db.with_conn(|c| runs::get(c, &run_id)).ok().flatten();
                match row {
                    Some(r) if r.exec_status == runs::EXEC_CANCELLED => {
                        cancel.store(true, Ordering::SeqCst);
                        break;
                    }
                    Some(r) if r.settled_at.is_some() => break,
                    _ => {}
                }
            }
        })))
    }

    pub(super) fn run_id(&self, child_id: &str) -> Option<String> {
        self.entries.get(child_id).map(|e| e.run_id.clone())
    }

    pub(super) fn started(&self, child_id: &str, timeout_secs: u64) {
        let (Some(db), Some(entry)) = (&self.db, self.entries.get(child_id)) else {
            return;
        };
        let timeout = i64::try_from(timeout_secs).unwrap_or(i64::MAX / 4);
        if let Err(e) = db.with_conn(|c| runs::mark_running(c, &entry.run_id, now(), timeout)) {
            warn!(error = %e, "subagent runs: could not mark running");
        }
    }

    /// Record evidence of life, at most once per interval per child.
    pub(super) fn touch(&self, child_id: &str) {
        let (Some(db), Some(entry)) = (&self.db, self.entries.get(child_id)) else {
            return;
        };
        let t = now();
        {
            let mut last = self.last_touch.lock().unwrap_or_else(|p| p.into_inner());
            if last
                .get(child_id)
                .is_some_and(|&p| t - p < TOUCH_INTERVAL_SECS)
            {
                return;
            }
            last.insert(child_id.to_string(), t);
        }
        if let Err(e) = db.with_conn(|c| runs::touch(c, &entry.run_id, t)) {
            warn!(error = %e, "subagent runs: could not record activity");
        }
    }

    /// Settle a child once: persist its result and verdict, queue the wake
    /// event (detached plans), and push the completion notice. Safe to call
    /// again for the same child; later calls change nothing.
    pub(super) fn settle(&self, outcome: &mut ChildOutcome) {
        let Some(entry) = self.entries.get(&outcome.id) else {
            return;
        };
        if outcome.run.is_some() {
            return;
        }
        let a = assess(outcome);
        outcome.run = Some(RunInfo {
            run_id: entry.run_id.clone(),
            accept_status: a.accept.to_string(),
            missing_deliverables: a.missing.clone(),
            blocker: a.blocker.clone(),
            next_action: Some(a.next_action.clone()),
        });
        let first = match &self.db {
            Some(db) => {
                let s = Settlement {
                    exec_status: exec_status_name(outcome.status).to_string(),
                    accept_status: a.accept.to_string(),
                    missing_deliverables: a.missing,
                    blocker_reason: a.blocker,
                    next_action: Some(a.next_action),
                    output_full: outcome.output.clone(),
                    output_preview: runs::preview(&outcome.output, runs::RESULT_PREVIEW_BYTES),
                    error: outcome.error.clone(),
                    resolved_backend: outcome.resolved_backend.clone(),
                };
                match db.with_conn(|c| runs::settle(c, &entry.run_id, &s, now(), self.detached)) {
                    Ok(true) => true,
                    Ok(false) => {
                        self.reconcile_with_registry(db, &entry.run_id, outcome)
                    }
                    Err(e) => {
                        warn!(error = %e, "subagent runs: could not settle run");
                        true
                    }
                }
            }
            None => true,
        };
        if first && self.detached {
            self.notify(outcome, &entry.role);
        }
    }

    /// The registry refused the settle. A row that is gone means registration
    /// failed, so this still counts as the first (and only) report. A row
    /// already settled elsewhere wins: a cancelled run is reported as such, so
    /// the parent never sees a result the registry says was discarded.
    fn reconcile_with_registry(&self, db: &Database, run_id: &str, outcome: &mut ChildOutcome) -> bool {
        match db.with_conn(|c| runs::get(c, run_id)) {
            Ok(Some(row)) => {
                if row.exec_status == runs::EXEC_CANCELLED {
                    outcome.status = ChildStatus::Cancelled;
                    if let Some(info) = outcome.run.as_mut() {
                        info.accept_status = runs::ACCEPT_BLOCKED.to_string();
                        info.blocker = Some("cancelled".to_string());
                    }
                }
                false
            }
            Ok(None) | Err(_) => true,
        }
    }

    fn notify(&self, outcome: &ChildOutcome, role: &str) {
        let Some(sink) = &self.sink else {
            return;
        };
        let fresh = self
            .emitted
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(outcome.id.clone());
        if !fresh {
            return;
        }
        let info = outcome.run.as_ref();
        sink(ChildCompletionEvent {
            parent_turn_id: self.parent_turn_id.clone(),
            task_id: outcome.id.clone(),
            role: role.to_string(),
            success: outcome.status.is_success(),
            summary: completion_summary(outcome),
            run_id: info.map(|i| i.run_id.clone()),
            status: Some(exec_status_name(outcome.status).to_string()),
            accept_status: info.map(|i| i.accept_status.clone()),
            error: outcome.error.clone(),
        });
    }
}

/// The child's output when present, else its error, cut to a notice-sized
/// preview. The full text lives in the registry.
pub(super) fn completion_summary(outcome: &ChildOutcome) -> String {
    let text = if !outcome.output.is_empty() {
        outcome.output.as_str()
    } else {
        outcome.error.as_deref().unwrap_or_default()
    };
    runs::preview(text, runs::RESULT_PREVIEW_BYTES)
}
